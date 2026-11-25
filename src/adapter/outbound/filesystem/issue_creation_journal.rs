//! 새 이슈 생성 의도의 영속 장부이다 (#2436).
//! 저장소 runtime 의 OS lease를 조회부터 원격 생성까지 유지한다. 잠금 파일은 삭제하지 않는다.
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::application::port::outbound::issue_creation_journal_port::{
    IssueCreationJournalPort, IssueCreationJournalTransaction,
};

#[cfg(any(unix, windows))]
mod records;
#[cfg(windows)]
mod windows;

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

    fn read_records(
        &self,
        workspace_dir: &str,
    ) -> Result<Vec<crate::domain::issue_creation::IssueCreationIntentRecord>> {
        let workspace = self.canonical_workspace(workspace_dir)?;
        let runtime = (self.resolve_runtime_dir)(&workspace)?;
        // Read committed data without waiting for the remote creation writer.
        #[cfg(unix)]
        let records = unix::read_validated_records(&runtime)?;
        #[cfg(windows)]
        let records = windows::read_validated_records(&runtime)?;
        #[cfg(not(any(unix, windows)))]
        let records = {
            match std::fs::symlink_metadata(runtime.join("issue-create-intents.jsonl")) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                _ => bail!("the creation provenance journal cannot be safely inspected"),
            }
        };
        if records.iter().any(
            |record: &crate::domain::issue_creation::IssueCreationIntentRecord| {
                record.repository.project_scope.workspace_dir != workspace
            },
        ) {
            bail!("the creation provenance journal belongs to another workspace")
        }
        Ok(records)
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
        #[cfg(windows)]
        {
            Ok(Box::new(windows::Transaction::open(
                runtime,
                workspace,
                std::time::Duration::from_secs(30),
            )?))
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (runtime, workspace);
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
        let records = self.read_records(&workspace)?;
        let mut snapshot = IssueCreationProvenanceSnapshot {
            repository: repository.clone(),
            created_issue_ids: std::collections::BTreeSet::new(),
            unresolved: false,
        };
        for record in records {
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
    use std::time::Duration;

    use super::records::{MAX_JOURNAL_BYTES, decode_records, encode_records, upsert};
    use super::*;
    use crate::adapter::outbound::filesystem::private_runtime_lease::PrivateRuntimeLease;
    use crate::adapter::outbound::filesystem::secure_fs;
    use crate::domain::issue_creation::IssueCreationIntentRecord;

    pub(super) const JOURNAL_NAME: &str = "issue-create-intents.jsonl";
    pub(super) const LOCK_NAME: &str = "issue-create-intents.lock";

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
        decode_records(text.as_bytes())
    }

    impl IssueCreationJournalTransaction for Transaction {
        fn records(&mut self) -> Result<Vec<IssueCreationIntentRecord>> {
            self.read_records()
        }

        fn save(&mut self, record: &IssueCreationIntentRecord) -> Result<()> {
            let mut records = self.read_records()?;
            upsert(&mut records, &self.workspace, record)?;
            let body = encode_records(&records)?;
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
}

#[cfg(test)]
mod tests;

#[cfg(all(test, not(any(unix, windows))))]
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

#[cfg(all(test, not(any(unix, windows))))]
mod provenance_portable_tests;
