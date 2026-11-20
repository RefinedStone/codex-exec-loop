//! 이슈 게시 의도 일지이다 (#2424). 저장소 runtime 디렉터리의 `issue-publish-intents.jsonl` 파일 하나이다.
//! 한 줄이 의도 하나이다 (`IssuePublishIntentRecord` 의 JSON). 저장할 때마다 파일 전체를 원자적으로 바꾼다.
//!
//! git 작업 일지와 다른 점: 읽지 못한 줄은 오류이다. 건너뛰면 결과를 모르는 의도를 놓치고 같은 글을 다시 보낸다.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};

use super::private_runtime_lease::PrivateRuntimeLease;
use super::secure_fs;
use crate::application::port::outbound::issue_publish_intent_journal_port::{
    IssuePublicationLease, IssuePublishIntentJournalPort,
};
use crate::domain::issue_publish_intent::IssuePublishIntentRecord;

const JOURNAL_FILE_NAME: &str = "issue-publish-intents.jsonl";
const PUBLICATION_LOCK_NAME: &str = "issue-publish-intents.lock";

/// 작업 폴더에서 그 저장소의 runtime 디렉터리를 찾는 함수이다.
type RuntimeDirectoryResolver = dyn Fn(&str) -> Result<PathBuf> + Send + Sync;

pub struct IssuePublishIntentJournalAdapter {
    resolve_runtime_dir: Box<RuntimeDirectoryResolver>,
    // 단일 파일 read-modify-write를 직렬화한다. 전체 게시의 프로세스 공유 lease는 서비스가 보관한다.
    write: Mutex<()>,
}

impl IssuePublishIntentJournalAdapter {
    pub fn new(
        resolve_runtime_dir: impl Fn(&str) -> Result<PathBuf> + Send + Sync + 'static,
    ) -> Self {
        Self {
            resolve_runtime_dir: Box::new(resolve_runtime_dir),
            write: Mutex::new(()),
        }
    }
}

fn read_records(runtime_dir: &Path) -> Result<Vec<IssuePublishIntentRecord>> {
    let body = secure_fs::read_optional_file(runtime_dir, Path::new(JOURNAL_FILE_NAME))?
        .unwrap_or_default();
    body.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).context("the issue publish intent journal has a broken line")
        })
        .collect()
}

impl IssuePublicationLease for PrivateRuntimeLease {
    fn verify_identity(&self) -> Result<()> {
        PrivateRuntimeLease::verify_identity(self)
    }
}

impl IssuePublishIntentJournalPort for IssuePublishIntentJournalAdapter {
    fn acquire_publication_lease(
        &self,
        workspace_dir: &str,
    ) -> Result<Box<dyn IssuePublicationLease + '_>> {
        let runtime_dir = (self.resolve_runtime_dir)(workspace_dir)?;
        let lease = PrivateRuntimeLease::acquire(
            runtime_dir,
            PUBLICATION_LOCK_NAME,
            std::time::Duration::from_secs(30),
        )?;
        Ok(Box::new(lease))
    }

    fn save(&self, workspace_dir: &str, record: &IssuePublishIntentRecord) -> Result<()> {
        let runtime_dir = (self.resolve_runtime_dir)(workspace_dir)?;
        let _write = self
            .write
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut records = read_records(&runtime_dir)?;
        match records
            .iter_mut()
            .find(|existing| existing.intent_id == record.intent_id)
        {
            Some(existing) => *existing = record.clone(),
            None => records.push(record.clone()),
        }
        // shortcut: 끝난 의도도 계속 남는다 (게시 한 번에 한 줄), 파일이 커지면 오래된 끝난 의도를 버린다.
        let mut body = String::new();
        for record in &records {
            body.push_str(&serde_json::to_string(record)?);
            body.push('\n');
        }
        secure_fs::write_file_atomic(&runtime_dir, Path::new(JOURNAL_FILE_NAME), body.as_bytes())
    }

    fn load(&self, workspace_dir: &str) -> Result<Vec<IssuePublishIntentRecord>> {
        read_records(&(self.resolve_runtime_dir)(workspace_dir)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issue_publish_intent::IssuePublishIntentState;
    use crate::domain::issue_work::{
        IssueFieldHash, IssueProjectScope, IssueProvider, IssueReference,
    };

    fn temp_runtime_dir(label: &str) -> PathBuf {
        let directory = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-issue-publish-journal-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn journal(directory: &Path) -> IssuePublishIntentJournalAdapter {
        let resolved = directory.to_path_buf();
        IssuePublishIntentJournalAdapter::new(move |_| Ok(resolved.clone()))
    }

    fn record(intent_id: &str, state: IssuePublishIntentState) -> IssuePublishIntentRecord {
        IssuePublishIntentRecord {
            intent_id: intent_id.to_string(),
            issue: IssueReference {
                provider: IssueProvider::GitHub,
                server_id: "github.com".to_string(),
                repository_id: "R_1".to_string(),
                issue_id: "I_1".to_string(),
                project_scope: IssueProjectScope {
                    workspace_dir: "/work/project".to_string(),
                },
            },
            previous_hash: IssueFieldHash::of_fields("Old", "old"),
            approved_hash: IssueFieldHash::of_fields("New", "new"),
            pending_vault_fields: Vec::new(),
            state,
            observed_hash: None,
            restore_link_finalized: false,
            write_response_mismatch: false,
            write_attempted: None,
            sequence: 0,
        }
    }

    #[cfg(unix)]
    #[test]
    fn publication_lease_is_shared_across_adapters_and_separate_from_creation() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;

        let directory = temp_runtime_dir("publication-lease");
        let directory = std::fs::canonicalize(directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let first = journal(&directory);
        let lease = first.acquire_publication_lease("/work/project").unwrap();
        lease.verify_identity().unwrap();
        assert!(
            PrivateRuntimeLease::acquire(directory.clone(), PUBLICATION_LOCK_NAME, Duration::ZERO)
                .is_err()
        );
        let creation = PrivateRuntimeLease::acquire(
            directory.clone(),
            "issue-create-intents.lock",
            Duration::ZERO,
        )
        .unwrap();
        creation.verify_identity().unwrap();
        drop(lease);
        let second = journal(&directory);
        let lease = second.acquire_publication_lease("/work/project").unwrap();
        lease.verify_identity().unwrap();
        drop(lease);
        drop(creation);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn publication_lease_detects_replaced_lock_before_mutation() {
        use std::os::unix::fs::PermissionsExt;

        let directory = temp_runtime_dir("publication-lease-replacement");
        let directory = std::fs::canonicalize(directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let adapter = journal(&directory);
        let lease = adapter.acquire_publication_lease("/work/project").unwrap();
        let lock = directory.join(PUBLICATION_LOCK_NAME);
        std::fs::rename(&lock, directory.join("old-lock")).unwrap();
        std::fs::write(&lock, "").unwrap();
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(lease.verify_identity().is_err());
        drop(lease);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn records_survive_a_new_adapter_and_a_later_save_replaces_the_intent() {
        let directory = temp_runtime_dir("replace");
        let first = journal(&directory);
        assert!(first.load("/work/project").unwrap().is_empty());
        for (id, state) in [
            ("a", IssuePublishIntentState::Recorded),
            ("b", IssuePublishIntentState::Sent),
            ("a", IssuePublishIntentState::Confirmed),
        ] {
            first.save("/work/project", &record(id, state)).unwrap();
        }

        // 데몬이 다시 시작한 뒤이다.
        let records = journal(&directory).load("/work/project").unwrap();

        assert_eq!(
            records,
            [
                record("a", IssuePublishIntentState::Confirmed),
                record("b", IssuePublishIntentState::Sent)
            ]
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_broken_line_is_an_error_and_is_not_overwritten() {
        let directory = temp_runtime_dir("broken");
        let journal = journal(&directory);
        journal
            .save("/work/project", &record("a", IssuePublishIntentState::Sent))
            .unwrap();
        let path = directory.join(JOURNAL_FILE_NAME);
        let broken = format!("{}{{ not json\n", std::fs::read_to_string(&path).unwrap());
        std::fs::write(&path, &broken).unwrap();

        assert!(journal.load("/work/project").is_err());
        assert!(
            journal
                .save("/work/project", &record("b", IssuePublishIntentState::Sent))
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_folder_without_a_runtime_directory_is_an_error() {
        let journal =
            IssuePublishIntentJournalAdapter::new(|_| anyhow::bail!("no runtime directory here"));
        assert!(journal.load("/work/project").is_err());
        assert!(
            journal
                .save("/work/project", &record("a", IssuePublishIntentState::Sent))
                .is_err()
        );
    }
}
