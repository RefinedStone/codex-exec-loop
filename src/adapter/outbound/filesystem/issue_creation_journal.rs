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

impl crate::application::port::outbound::issue_creation_provenance_port::IssueCreationProvenancePort
    for IssueCreationJournalAdapter
{
    fn load(
        &self,
        repository: &crate::domain::issue_automation::source::IssueSourceRepository,
    ) -> Result<crate::application::port::outbound::issue_creation_provenance_port::IssueCreationProvenanceSnapshot>{
        use crate::application::port::outbound::issue_creation_provenance_port::IssueCreationProvenanceSnapshot;
        use crate::domain::issue_creation::IssueCreationIntentState;

        repository.validate()?;
        let workspace = self.canonical_workspace(&repository.project_scope.workspace_dir)?;
        if workspace != repository.project_scope.workspace_dir {
            bail!("the creation provenance workspace identity changed")
        }
        let runtime = (self.resolve_runtime_dir)(&workspace)?;
        // 생성은 Sent를 영속화한 뒤에만 원격으로 보낸다. 읽기는 atomic journal
        // snapshot만 사용하므로 원격 요청 동안 잡힌 writer lease를 기다리지 않는다.
        #[cfg(unix)]
        let records = unix::read_validated_records(&runtime)?;
        #[cfg(not(unix))]
        let records = {
            match std::fs::symlink_metadata(runtime.join("issue-create-intents.jsonl")) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                // 이 플랫폼에는 안전한 생성 journal 읽기 구현이 없다.
                // 파일이 있으면 없다고 간주하거나 일반 파일 읽기로 우회하지 않는다.
                _ => bail!("the creation provenance journal cannot be safely inspected"),
            }
        };
        let mut snapshot = IssueCreationProvenanceSnapshot {
            repository: repository.clone(),
            created_issue_ids: std::collections::BTreeSet::new(),
            unresolved: false,
        };
        for record in records {
            let record: crate::domain::issue_creation::IssueCreationIntentRecord = record;
            if record.repository.project_scope.workspace_dir != workspace {
                bail!("the creation provenance journal belongs to another workspace")
            }
            if record.repository.provider != repository.provider
                || record.repository.server_id != repository.server_id
                || record.repository.repository_id != repository.repository_id
            {
                continue;
            }
            if let Some(created) = &record.created {
                if !record.repository.contains(&created.issue) {
                    bail!("the creation provenance identity does not match")
                }
                snapshot
                    .created_issue_ids
                    .insert(created.issue.issue_id.clone());
            } else if matches!(
                record.state,
                IssueCreationIntentState::Sent | IssueCreationIntentState::NeedsConfirmation
            ) {
                snapshot.unresolved = true;
            }
        }
        Ok(snapshot)
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
            let records = read_validated_records(&self.runtime)?;
            self.verify_identity()?;
            Ok(records)
        }
    }

    pub(super) fn read_validated_records(runtime: &Path) -> Result<Vec<IssueCreationIntentRecord>> {
        let text = secure_fs::read_optional_file_with_limit(
            runtime,
            Path::new(JOURNAL_NAME),
            MAX_JOURNAL_BYTES,
        )?
        .unwrap_or_default();
        let mut records = Vec::new();
        let mut ids = HashSet::new();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            if records.len() >= MAX_RECORDS {
                bail!("the issue creation journal contains too many records")
            }
            // Do not include source text in the parse error.
            let record: IssueCreationIntentRecord = serde_json::from_str(line)
                .map_err(|_| anyhow!("the issue creation journal contains an invalid record"))?;
            validate_record(&record)?;
            if !ids.insert(record.intent_id.clone()) {
                bail!("the issue creation journal contains duplicate intent IDs")
            }
            records.push(record);
        }
        Ok(records)
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

#[cfg(test)]
mod tests;

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

#[cfg(all(test, not(unix)))]
mod provenance_portable_tests;
