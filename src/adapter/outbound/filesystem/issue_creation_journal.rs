//! 새 이슈 생성 의도의 영속 장부이다 (#2436).
//! 저장소 runtime 의 OS lease를 조회부터 원격 생성까지 유지한다. 잠금 파일은 삭제하지 않는다.
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::application::port::outbound::issue_creation_journal_port::{
    IssueCreationJournalPort, IssueCreationJournalTransaction,
};

type RuntimeDirectoryResolver = dyn Fn(&str) -> Result<PathBuf> + Send + Sync;
type WorkspaceResolver = dyn Fn(&str) -> Result<String> + Send + Sync;

pub struct IssueCreationJournalAdapter {
    resolve_runtime_dir: Box<RuntimeDirectoryResolver>,
    resolve_workspace: Option<Box<WorkspaceResolver>>,
}

impl IssueCreationJournalAdapter {
    pub fn new(
        resolve_runtime_dir: impl Fn(&str) -> Result<PathBuf> + Send + Sync + 'static,
    ) -> Self {
        Self {
            resolve_runtime_dir: Box::new(resolve_runtime_dir),
            resolve_workspace: None,
        }
    }
    /// Production supplies the repository kernel's primary checkout. The default
    /// remains filesystem canonicalization for standalone adapters and test fixtures.
    pub fn with_workspace_resolver(
        mut self,
        resolve_workspace: impl Fn(&str) -> Result<String> + Send + Sync + 'static,
    ) -> Self {
        self.resolve_workspace = Some(Box::new(resolve_workspace));
        self
    }
}

fn canonical_absolute_directory(workspace_dir: &str) -> Result<String> {
    let path = Path::new(workspace_dir);
    if !path.is_absolute() {
        bail!("the issue creation workspace must be an absolute directory")
    }
    let canonical = std::fs::canonicalize(path)
        .context("the issue creation workspace could not be resolved")?;
    if !canonical.is_dir() {
        bail!("the issue creation workspace is not a directory")
    }
    canonical
        .to_str()
        .map(str::to_string)
        .context("the issue creation workspace is not valid UTF-8")
}

impl IssueCreationJournalPort for IssueCreationJournalAdapter {
    fn canonical_workspace(&self, workspace_dir: &str) -> Result<String> {
        let canonical = canonical_absolute_directory(workspace_dir)?;
        match &self.resolve_workspace {
            Some(resolve) => canonical_absolute_directory(&resolve(&canonical)?),
            None => Ok(canonical),
        }
    }

    fn begin(&self, workspace_dir: &str) -> Result<Box<dyn IssueCreationJournalTransaction + '_>> {
        let workspace = self.canonical_workspace(workspace_dir)?;
        let runtime = (self.resolve_runtime_dir)(&workspace)?;
        #[cfg(unix)]
        {
            Ok(Box::new(unix::Transaction::open(
                runtime,
                workspace,
                std::time::Duration::from_secs(30),
            )?))
        }
        #[cfg(not(unix))]
        {
            let _ = (runtime, workspace);
            // The existing secure_fs journal and Vault have no safe non-Unix implementation.
            bail!("the issue creation journal is unavailable on this platform")
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::collections::HashSet;
    use std::time::Duration;

    use anyhow::anyhow;

    use super::*;
    use crate::adapter::outbound::filesystem::private_runtime_lease::PrivateRuntimeLease;
    use crate::adapter::outbound::filesystem::secure_fs;
    use crate::domain::issue_creation::{
        IssueCreationIntentRecord, IssueCreationIntentState, IssueRepositoryReference,
    };
    use crate::domain::issue_work::{ISSUE_FIELD_HASH_VERSION, IssueFieldHash, IssueProvider};

    pub(super) const JOURNAL_NAME: &str = "issue-create-intents.jsonl";
    pub(super) const LOCK_NAME: &str = "issue-create-intents.lock";
    const MAX_JOURNAL_BYTES: usize = 8 * 1024 * 1024;
    const MAX_RECORDS: usize = 4096;

    pub(super) struct Transaction {
        runtime: PathBuf,
        workspace: String,
        lease: PrivateRuntimeLease,
    }

    impl Transaction {
        pub(super) fn open(runtime: PathBuf, workspace: String, timeout: Duration) -> Result<Self> {
            let lease = PrivateRuntimeLease::acquire(runtime.clone(), LOCK_NAME, timeout)?;
            let transaction = Self {
                runtime,
                workspace,
                lease,
            };
            transaction.verify_identity()?;
            // A corrupt or oversized journal must fail before any application/network action.
            transaction.read_records()?;
            Ok(transaction)
        }

        fn verify_identity(&self) -> Result<()> {
            self.lease.verify_identity()
        }

        fn read_records(&self) -> Result<Vec<IssueCreationIntentRecord>> {
            self.verify_identity()?;
            let text = secure_fs::read_optional_file_with_limit(
                &self.runtime,
                Path::new(JOURNAL_NAME),
                MAX_JOURNAL_BYTES,
            )?
            .unwrap_or_default();
            self.verify_identity()?;
            let mut records = Vec::new();
            let mut ids = HashSet::new();
            for line in text.lines().filter(|line| !line.trim().is_empty()) {
                if records.len() >= MAX_RECORDS {
                    bail!("the issue creation journal contains too many records")
                }
                // Do not include source text in the parse error.
                let record: IssueCreationIntentRecord =
                    serde_json::from_str(line).map_err(|_| {
                        anyhow!("the issue creation journal contains an invalid record")
                    })?;
                validate_record(&record)?;
                if !ids.insert(record.intent_id.clone()) {
                    bail!("the issue creation journal contains duplicate intent IDs")
                }
                records.push(record);
            }
            Ok(records)
        }
    }

    impl IssueCreationJournalTransaction for Transaction {
        fn records(&mut self) -> Result<Vec<IssueCreationIntentRecord>> {
            self.read_records()
        }

        fn save(&mut self, record: &IssueCreationIntentRecord) -> Result<()> {
            validate_record(record)?;
            if record.repository.project_scope.workspace_dir != self.workspace {
                bail!("the creation intent belongs to a different workspace")
            }
            let mut records = self.read_records()?;
            match records
                .iter_mut()
                .find(|existing| existing.intent_id == record.intent_id)
            {
                Some(existing) => {
                    if existing.repository != record.repository
                        || existing.predecessor_intent_id != record.predecessor_intent_id
                        || (existing.approved.is_some() && existing.approved != record.approved)
                    {
                        bail!("an existing creation intent or approved copy cannot be replaced")
                    }
                    *existing = record.clone();
                }
                None => {
                    if records.len() >= MAX_RECORDS {
                        bail!("the issue creation journal contains too many records")
                    }
                    records.push(record.clone());
                }
            }
            let mut body = Vec::new();
            for record in records {
                serde_json::to_writer(&mut body, &record)
                    .context("the issue creation journal could not be encoded")?;
                body.push(b'\n');
                if body.len() > MAX_JOURNAL_BYTES {
                    bail!("the issue creation journal is full")
                }
            }
            self.verify_identity()?;
            // This transaction owns the cross-process lease. Preserve secure_fs's anchored
            // anti-alias and durability guarantees without taking a second mutation lock.
            secure_fs::write_file_atomic_unlocked_with_limit_and_unix_mode(
                &self.runtime,
                Path::new(JOURNAL_NAME),
                &body,
                MAX_JOURNAL_BYTES,
                0o600,
            )?;
            self.verify_identity()
        }
    }

    fn valid_id(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 200
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-=".contains(&byte))
    }

    fn valid_hash(hash: &IssueFieldHash) -> bool {
        hash.version == ISSUE_FIELD_HASH_VERSION
            && hash.sha256.len() == 64
            && hash
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }

    fn validate_repository(repository: &IssueRepositoryReference) -> Result<()> {
        if !valid_id(&repository.repository_id)
            || repository.server_id != "github.com"
            || repository.provider != IssueProvider::GitHub
            || !Path::new(&repository.project_scope.workspace_dir).is_absolute()
        {
            bail!("the issue creation journal contains an invalid repository")
        }
        Ok(())
    }

    fn validate_record(record: &IssueCreationIntentRecord) -> Result<()> {
        validate_repository(&record.repository)?;
        if !valid_id(&record.intent_id)
            || record
                .predecessor_intent_id
                .as_ref()
                .is_some_and(|previous| !valid_id(previous) || previous == &record.intent_id)
        {
            bail!("the issue creation journal contains an invalid intent ID")
        }
        if record
            .observed_hash
            .as_ref()
            .is_some_and(|hash| !valid_hash(hash))
        {
            bail!("the issue creation journal contains an invalid observed hash")
        }
        match &record.approved {
            Some(approved) => {
                if approved.approval.intent_id != record.intent_id
                    || approved.approval.repository != record.repository
                    || !valid_hash(&approved.approval.shared_copy_hash)
                    || approved.approval.shared_copy_hash
                        != IssueFieldHash::of_fields(
                            &approved.shared_copy.title,
                            &approved.shared_copy.body,
                        )
                {
                    bail!("the issue creation journal contains an inconsistent approved copy")
                }
            }
            None => {
                if !matches!(
                    record.state,
                    IssueCreationIntentState::Prepared | IssueCreationIntentState::Abandoned
                ) || !record.pending_vault_fields.is_empty()
                    || record.created.is_some()
                    || record.observed_hash.is_some()
                    || record.restore_link_finalized
                    || record.write_response_mismatch
                {
                    bail!("the issue creation journal contains an unapproved result")
                }
            }
        }
        if record.pending_vault_fields.len() > 2
            || (record.pending_vault_fields.len() == 2
                && record.pending_vault_fields[0] == record.pending_vault_fields[1])
        {
            bail!("the issue creation journal contains duplicate Vault fields")
        }
        if let Some(created) = &record.created {
            let locator = &created.locator;
            let valid_name = |name: &str, extra: &[u8]| {
                !name.is_empty()
                    && name.len() <= 100
                    && name != "."
                    && name != ".."
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || extra.contains(&byte))
            };
            if !record.repository.contains(&created.issue)
                || !valid_id(&created.issue.issue_id)
                || !valid_name(&locator.owner, b"-_")
                || !valid_name(&locator.repository, b"-_.")
                || locator.number == 0
                || locator.number > i32::MAX as u64
                || created.url
                    != format!(
                        "https://github.com/{}/{}/issues/{}",
                        locator.owner, locator.repository, locator.number
                    )
            {
                bail!("the issue creation journal contains a different creation target")
            }
        }
        if record.state == IssueCreationIntentState::Confirmed
            && (record.created.is_none()
                || record.observed_hash.as_ref()
                    != record
                        .approved
                        .as_ref()
                        .map(|approved| &approved.approval.shared_copy_hash))
        {
            bail!("the issue creation journal contains an unverified confirmed result")
        }
        if record.restore_link_finalized && record.state != IssueCreationIntentState::Confirmed {
            bail!("the issue creation journal contains an unconfirmed restore link")
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::process::Command;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::adapter::outbound::filesystem::secure_fs;
    use crate::domain::issue_creation::{
        IssueCreationApproval, IssueCreationApprovedCopy, IssueCreationIntentRecord,
        IssueCreationIntentState, IssueRepositoryReference,
    };
    use crate::domain::issue_work::{
        IssueFieldHash, IssueProjectScope, IssueProvider, IssueSharedCopy,
    };

    struct Fixture {
        root: PathBuf,
        workspace: PathBuf,
        runtime: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "akra-create-journal-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&root).unwrap();
            let root = fs::canonicalize(root).unwrap();
            let workspace = root.join("workspace");
            let runtime = root.join("runtime");
            fs::create_dir(&workspace).unwrap();
            fs::create_dir(&runtime).unwrap();
            fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
            Self {
                root,
                workspace,
                runtime,
            }
        }

        fn adapter(&self) -> IssueCreationJournalAdapter {
            let runtime = self.runtime.clone();
            IssueCreationJournalAdapter::new(move |_| Ok(runtime.clone()))
        }

        fn record(&self) -> IssueCreationIntentRecord {
            IssueCreationIntentRecord {
                intent_id: "intent-1".to_string(),
                repository: IssueRepositoryReference {
                    provider: IssueProvider::GitHub,
                    server_id: "github.com".to_string(),
                    repository_id: "R_1".to_string(),
                    project_scope: IssueProjectScope {
                        workspace_dir: self.workspace.to_str().unwrap().to_string(),
                    },
                },
                state: IssueCreationIntentState::Prepared,
                approved: None,
                pending_vault_fields: Vec::new(),
                created: None,
                observed_hash: None,
                write_response_mismatch: false,
                restore_link_finalized: false,
                shared_copy_only: false,
                predecessor_intent_id: None,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn approve(record: &mut IssueCreationIntentRecord) {
        let copy = IssueSharedCopy {
            title: "T".to_string(),
            body: "masked".to_string(),
        };
        record.approved = Some(IssueCreationApprovedCopy {
            approval: IssueCreationApproval {
                repository: record.repository.clone(),
                intent_id: record.intent_id.clone(),
                shared_copy_hash: IssueFieldHash::of_fields(&copy.title, &copy.body),
                policy_version: "policy-1".to_string(),
                template_version: None,
            },
            shared_copy: copy,
        });
        record.state = IssueCreationIntentState::Recorded;
    }

    mod workspace_identity {
        use std::sync::Arc;

        use super::*;
        use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
        use crate::application::port::inbound::issue_creation_draft::IssueCreationTarget;
        use crate::application::port::outbound::evidence_vault_port::{
            EvidenceVaultStorePort, VaultPlaintext, VaultRecordLoadRequest, VaultRecordSaveRequest,
            VaultStoreError,
        };
        use crate::application::port::outbound::issue_create_port::{
            IssueCreatePort, IssueCreateRequest, IssueCreateResponse, IssueRepositoryResolveRequest,
        };
        use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
        use crate::application::port::outbound::issue_snapshot_port::{
            IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest,
            IssueSnapshotResponse,
        };
        use crate::application::port::outbound::issue_update_port::{
            IssueUpdatePort, IssueUpdateRequest, IssueUpdateResponse,
        };
        use crate::application::service::issue_creation::IssueCreationService;
        use crate::application::service::issue_publish_journal::IssueJournaledPublishService;
        use crate::domain::issue_publish_intent::IssuePublishIntentRecord;

        struct RepositoryOnly;

        impl IssueCreatePort for RepositoryOnly {
            fn resolve_repository(
                &self,
                request: IssueRepositoryResolveRequest,
            ) -> Result<IssueRepositoryReference> {
                Ok(IssueRepositoryReference {
                    provider: IssueProvider::GitHub,
                    server_id: "github.com".to_string(),
                    repository_id: "R_1".to_string(),
                    project_scope: request.project_scope,
                })
            }
            fn create_issue(&self, _: IssueCreateRequest) -> Result<IssueCreateResponse> {
                panic!("opening a draft must never create an issue")
            }
        }

        impl IssueSnapshotPort for RepositoryOnly {
            fn resolve(&self, _: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
                panic!("opening a creation intent must not resolve an existing issue")
            }
            fn read(&self, _: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
                panic!("opening a creation intent must not read an existing issue")
            }
        }

        impl IssueUpdatePort for RepositoryOnly {
            fn update_title_and_body(&self, _: IssueUpdateRequest) -> Result<IssueUpdateResponse> {
                panic!("opening a draft must never update an issue")
            }
        }

        impl EvidenceVaultStorePort for RepositoryOnly {
            fn save(&self, _: VaultRecordSaveRequest) -> Result<(), VaultStoreError> {
                panic!("opening a draft must not write the Vault")
            }
            fn load(&self, _: VaultRecordLoadRequest) -> Result<VaultPlaintext, VaultStoreError> {
                panic!("opening a draft must not read the Vault")
            }
        }

        impl IssuePublishIntentJournalPort for RepositoryOnly {
            fn save(&self, _: &str, _: &IssuePublishIntentRecord) -> Result<()> {
                panic!("opening a draft must not write the publish journal")
            }
            fn load(&self, _: &str) -> Result<Vec<IssuePublishIntentRecord>> {
                panic!("opening a draft must not read the publish journal")
            }
        }

        fn service(fixture: &Fixture) -> (IssueCreationService, Arc<IssueCreationJournalAdapter>) {
            let runtime = fixture.runtime.clone();
            let journal = Arc::new(
                IssueCreationJournalAdapter::new(move |_| Ok(runtime.clone()))
                    .with_workspace_resolver(|cwd| {
                        SqlitePlanningAuthorityAdapter::resolve_primary_checkout_root(cwd)?
                            .and_then(|path| path.to_str().map(str::to_string))
                            .ok_or_else(|| {
                                anyhow::anyhow!("the repository has no primary checkout")
                            })
                    }),
            );
            let ports = Arc::new(RepositoryOnly);
            let publisher = Arc::new(IssueJournaledPublishService::new(
                ports.clone(),
                ports.clone(),
                ports.clone(),
                ports.clone(),
            ));
            (
                IssueCreationService::new(
                    ports.clone(),
                    ports.clone(),
                    journal.clone(),
                    ports,
                    publisher,
                ),
                journal,
            )
        }

        fn git(workspace: &Path, args: &[&str]) {
            let output = crate::test_utils::git_command()
                .current_dir(workspace)
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "user.name=Akra Scope Test",
                    "-c",
                    "user.email=scope@example.invalid",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        #[test]
        fn linked_worktree_subdirectory_and_alias_reopen_one_durable_creation_intent() {
            let fixture = Fixture::new("kernel-project-scope");
            git(&fixture.workspace, &["init", "--quiet"]);
            git(
                &fixture.workspace,
                &["commit", "--allow-empty", "--quiet", "-m", "scope fixture"],
            );
            let linked = fixture.root.join("linked-worktree");
            git(
                &fixture.workspace,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "--detach",
                    linked.to_str().unwrap(),
                    "HEAD",
                ],
            );
            let nested = linked.join("nested");
            fs::create_dir(&nested).unwrap();
            let alias = fixture.root.join("linked-alias");
            symlink(&linked, &alias).unwrap();
            let target = |path: &Path| IssueCreationTarget {
                cwd: path.to_str().unwrap().to_string(),
                owner: "sample".to_string(),
                repository: "widgets".to_string(),
            };
            let (first, journal) = service(&fixture);
            let opened = first.open(&target(&fixture.workspace)).unwrap();
            for workspace in [&linked, &nested, &alias] {
                assert_eq!(first.open(&target(workspace)).unwrap(), opened);
            }
            let (restarted, _) = service(&fixture);
            assert_eq!(restarted.open(&target(&alias)).unwrap(), opened);
            assert_eq!(
                opened.repository.project_scope.workspace_dir,
                fixture.workspace.to_str().unwrap()
            );
            assert_eq!(
                journal
                    .begin(alias.to_str().unwrap())
                    .unwrap()
                    .records()
                    .unwrap(),
                vec![opened],
            );
        }
    }

    #[test]
    fn durable_states_survive_a_new_adapter_and_aliases_share_workspace_identity() {
        let fixture = Fixture::new("restart");
        let alias = fixture.root.join("workspace-alias");
        symlink(&fixture.workspace, &alias).unwrap();
        let adapter = fixture.adapter();
        let workspace = fixture.workspace.to_str().unwrap();
        assert_eq!(
            adapter
                .canonical_workspace(alias.to_str().unwrap())
                .unwrap(),
            adapter.canonical_workspace(workspace).unwrap()
        );
        let mut record = fixture.record();
        {
            let mut transaction = adapter.begin(alias.to_str().unwrap()).unwrap();
            transaction.save(&record).unwrap();
            approve(&mut record);
            transaction.save(&record).unwrap();
            record.state = IssueCreationIntentState::Sent;
            transaction.save(&record).unwrap();
        }
        let restarted = fixture.adapter();
        let records = restarted.begin(workspace).unwrap().records().unwrap();
        assert_eq!(records, vec![record]);
    }

    #[test]
    fn corrupt_oversized_and_approval_mismatched_journals_fail_before_transaction() {
        let fixture = Fixture::new("corrupt");
        let adapter = fixture.adapter();
        let path = fixture.runtime.join(unix::JOURNAL_NAME);
        fs::write(&path, "{\"unexpected-remote-plaintext\":").unwrap();
        let error = adapter
            .begin(fixture.workspace.to_str().unwrap())
            .err()
            .unwrap();
        assert!(!format!("{error:#}").contains("unexpected-remote-plaintext"));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\"unexpected-remote-plaintext\":"
        );

        let oversized = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        oversized.set_len(8 * 1024 * 1024 + 1).unwrap();
        assert!(adapter.begin(fixture.workspace.to_str().unwrap()).is_err());
        assert_eq!(fs::metadata(&path).unwrap().len(), 8 * 1024 * 1024 + 1);

        let mut record = fixture.record();
        approve(&mut record);
        record.approved.as_mut().unwrap().shared_copy.body = "different".to_string();
        fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(adapter.begin(fixture.workspace.to_str().unwrap()).is_err());
    }

    #[test]
    fn approved_copy_cannot_be_replaced_even_if_a_new_hash_is_supplied() {
        let fixture = Fixture::new("frozen");
        let adapter = fixture.adapter();
        let mut transaction = adapter.begin(fixture.workspace.to_str().unwrap()).unwrap();
        let mut record = fixture.record();
        approve(&mut record);
        transaction.save(&record).unwrap();
        let before = transaction.records().unwrap();
        let approved = record.approved.as_mut().unwrap();
        approved.shared_copy.body.push_str("changed");
        approved.approval.shared_copy_hash =
            IssueFieldHash::of_fields(&approved.shared_copy.title, &approved.shared_copy.body);
        assert!(transaction.save(&record).is_err());
        assert_eq!(transaction.records().unwrap(), before);
    }

    #[test]
    fn held_copy_can_resume_shared_only_and_keep_sent_evidence_when_restarted() {
        let fixture = Fixture::new("held-resume");
        let adapter = fixture.adapter();
        let mut transaction = adapter.begin(fixture.workspace.to_str().unwrap()).unwrap();
        let mut held = fixture.record();
        approve(&mut held);
        held.state = IssueCreationIntentState::Held;
        held.pending_vault_fields =
            vec![crate::domain::issue_publish_intent::IssuePublishVaultField::Body];
        transaction.save(&held).unwrap();
        let frozen = held.approved.clone();
        held.pending_vault_fields.clear();
        held.shared_copy_only = true;
        held.state = IssueCreationIntentState::Recorded;
        transaction.save(&held).unwrap();
        held.state = IssueCreationIntentState::Sent;
        transaction.save(&held).unwrap();
        let mut next = fixture.record();
        next.intent_id = "intent-2".to_string();
        next.predecessor_intent_id = Some(held.intent_id.clone());
        transaction.save(&next).unwrap();
        let records = transaction.records().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].approved, frozen);
        assert_eq!(records[0].state, IssueCreationIntentState::Sent);
        assert_eq!(records[1], next);
    }

    #[test]
    fn duplicate_intents_and_foreign_workspace_writes_are_rejected() {
        let fixture = Fixture::new("identity");
        let adapter = fixture.adapter();
        let record = fixture.record();
        {
            let mut transaction = adapter.begin(fixture.workspace.to_str().unwrap()).unwrap();
            let mut other = record.clone();
            other.repository.project_scope.workspace_dir = "/different-workspace".to_string();
            assert!(transaction.save(&other).is_err());
            transaction.save(&record).unwrap();
        }
        let line = serde_json::to_string(&record).unwrap();
        fs::write(
            fixture.runtime.join(unix::JOURNAL_NAME),
            format!("{line}\n{line}\n"),
        )
        .unwrap();
        assert!(adapter.begin(fixture.workspace.to_str().unwrap()).is_err());
    }

    #[test]
    fn a_saved_state_remains_readable_after_a_post_replacement_error() {
        let fixture = Fixture::new("durability");
        let adapter = fixture.adapter();
        let mut transaction = adapter.begin(fixture.workspace.to_str().unwrap()).unwrap();
        let record = fixture.record();
        secure_fs::install_after_atomic_replace_error_hook("synthetic sync failure");
        assert!(transaction.save(&record).is_err());
        assert_eq!(transaction.records().unwrap(), vec![record]);
    }

    #[test]
    fn lock_symlinks_fifos_hardlinks_and_nonprivate_runtime_are_rejected() {
        for kind in ["symlink", "fifo", "hardlink", "permissions"] {
            let fixture = Fixture::new(kind);
            let lock = fixture.runtime.join(unix::LOCK_NAME);
            match kind {
                "symlink" => {
                    let target = fixture.root.join("external");
                    fs::write(&target, "unchanged").unwrap();
                    symlink(target, &lock).unwrap();
                }
                "fifo" => {
                    use std::ffi::CString;
                    use std::os::unix::ffi::OsStrExt;
                    let path = CString::new(lock.as_os_str().as_bytes()).unwrap();
                    // SAFETY: the fixture path is a live NUL-terminated string.
                    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
                }
                "hardlink" => {
                    let target = fixture.root.join("external");
                    fs::write(&target, "unchanged").unwrap();
                    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
                    fs::hard_link(target, &lock).unwrap();
                }
                _ => fs::set_permissions(&fixture.runtime, fs::Permissions::from_mode(0o755))
                    .unwrap(),
            }
            let adapter = fixture.adapter();
            assert!(
                adapter.begin(fixture.workspace.to_str().unwrap()).is_err(),
                "{kind}"
            );
        }
    }

    #[test]
    fn journal_symlinks_fifos_and_runtime_directory_replacement_are_rejected() {
        let fixture = Fixture::new("journal-alias");
        let adapter = fixture.adapter();
        let journal = fixture.runtime.join(unix::JOURNAL_NAME);
        let outside = fixture.root.join("external");
        fs::write(&outside, "unchanged").unwrap();
        symlink(&outside, &journal).unwrap();
        assert!(adapter.begin(fixture.workspace.to_str().unwrap()).is_err());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "unchanged");
        fs::remove_file(&journal).unwrap();

        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let fifo = CString::new(journal.as_os_str().as_bytes()).unwrap();
        // SAFETY: the fixture path is a live NUL-terminated string.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(adapter.begin(fixture.workspace.to_str().unwrap()).is_err());
        fs::remove_file(&journal).unwrap();

        let mut transaction = adapter.begin(fixture.workspace.to_str().unwrap()).unwrap();
        let previous = fixture.root.join("previous-runtime");
        fs::rename(&fixture.runtime, &previous).unwrap();
        fs::create_dir(&fixture.runtime).unwrap();
        fs::set_permissions(&fixture.runtime, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(transaction.save(&fixture.record()).is_err());
        assert!(!fixture.runtime.join(unix::JOURNAL_NAME).exists());
    }

    #[test]
    fn another_process_cannot_enter_until_the_transaction_is_dropped() {
        let fixture = Fixture::new("process-lease");
        let adapter = fixture.adapter();
        let transaction = adapter.begin(fixture.workspace.to_str().unwrap()).unwrap();
        for (locked, held) in [(true, Some(transaction)), (false, None)] {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "adapter::outbound::filesystem::issue_creation_journal::tests::child_checks_creation_lease",
                    "--nocapture",
                ])
                .env("AKRA_TEST_CREATE_LEASE_RUNTIME", &fixture.runtime)
                .env("AKRA_TEST_CREATE_LEASE_WORKSPACE", &fixture.workspace)
                .env("AKRA_TEST_CREATE_LEASE_EXPECT_LOCKED", if locked { "1" } else { "0" })
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("creation-lease-child-completed")
            );
            drop(held);
        }
    }

    #[test]
    fn child_checks_creation_lease() {
        let Some(runtime) = std::env::var_os("AKRA_TEST_CREATE_LEASE_RUNTIME") else {
            return;
        };
        let workspace = std::env::var("AKRA_TEST_CREATE_LEASE_WORKSPACE").unwrap();
        let expected_locked = std::env::var("AKRA_TEST_CREATE_LEASE_EXPECT_LOCKED").unwrap() == "1";
        let result = unix::Transaction::open(PathBuf::from(runtime), workspace, Duration::ZERO);
        assert_eq!(result.is_err(), expected_locked);
        println!("creation-lease-child-completed");
    }
}

#[cfg(all(test, not(unix)))]
mod unsupported_tests {
    use super::*;

    #[test]
    fn unsupported_platform_does_not_create_a_weaker_journal() {
        let workspace = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let runtime = workspace.join(format!("akra-create-unsupported-{}", std::process::id()));
        let selected = runtime.clone();
        let adapter = IssueCreationJournalAdapter::new(move |_| Ok(selected.clone()));
        assert!(adapter.begin(workspace.to_str().unwrap()).is_err());
        assert!(!runtime.exists());
    }
}

#[cfg(test)]
mod workspace_resolver_tests {
    use super::*;

    #[test]
    fn injected_workspace_resolution_is_checked_and_never_falls_back_on_failure() {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-create-workspace-resolver-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir_all(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let selected = root.join("primary");
        std::fs::create_dir(&selected).unwrap();
        let file = root.join("regular-file");
        std::fs::write(&file, "fixture").unwrap();
        let adapter = |selected: String| {
            IssueCreationJournalAdapter::new(|_| {
                panic!("workspace lookup must not open the journal")
            })
            .with_workspace_resolver(move |_| Ok(selected.clone()))
        };
        let input = root.to_str().unwrap();
        for invalid in [
            "relative-workspace".to_string(),
            root.join("missing").to_str().unwrap().to_string(),
            file.to_str().unwrap().to_string(),
        ] {
            assert!(adapter(invalid).canonical_workspace(input).is_err());
        }
        let valid = adapter(selected.to_str().unwrap().to_string());
        assert_eq!(
            valid.canonical_workspace(input).unwrap(),
            std::fs::canonicalize(selected).unwrap().to_str().unwrap()
        );
        assert!(valid.canonical_workspace("relative-input").is_err());
        let failed = IssueCreationJournalAdapter::new(|_| panic!("must not open the journal"))
            .with_workspace_resolver(|_| anyhow::bail!("primary checkout unavailable"));
        assert!(failed.canonical_workspace(input).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
