//! git 작업 일지이다 (plan 27 G3). 저장소마다 파일 하나이다: 저장소 runtime 디렉터리의 `git-operations.jsonl`.
//! 한 줄이 기록 하나이다. 같은 작업의 뒤 줄이 앞 줄을 대신한다.
//!
//! 파일은 저장소 밖에 있다. runtime 디렉터리는 현재 사용자 전용이고, 조립하는 쪽이 준비해서 넘긴다 (커널 잠금
//! 파일과 같다). 쓰기는 옆 파일에 쓴 뒤 이름을 바꾼다: 쓰는 도중에 데몬이 죽어도 앞의 일지가 남는다.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};

use crate::application::port::outbound::git_operation_journal_port::GitOperationJournalPort;
use crate::domain::git_operation::{GitOperationRecord, fold_git_operation_records};

const JOURNAL_FILE_NAME: &str = "git-operations.jsonl";
const JOURNAL_NEXT_FILE_NAME: &str = "git-operations.jsonl.next";
/// 일지가 가지는 줄 수의 최댓값이다. 넘으면 오래된 줄부터 버린다. 작업 하나가 두세 줄을 쓴다.
const JOURNAL_LINE_LIMIT: usize = 2000;

/// 저장소 안의 폴더에서 그 저장소의 runtime 디렉터리를 찾는 함수이다.
type RuntimeDirectoryResolver = dyn Fn(&str) -> Result<PathBuf> + Send + Sync;

pub(crate) struct GitOperationJournalAdapter {
    resolve_runtime_dir: Box<RuntimeDirectoryResolver>,
    // 읽고 고쳐 쓰는 사이에 다른 쓰기가 끼지 않게 한다. 일지를 쓰는 프로세스는 Gateway 하나이다.
    write: Mutex<()>,
}

impl GitOperationJournalAdapter {
    pub(crate) fn new(
        resolve_runtime_dir: impl Fn(&str) -> Result<PathBuf> + Send + Sync + 'static,
    ) -> Self {
        Self {
            resolve_runtime_dir: Box::new(resolve_runtime_dir),
            write: Mutex::new(()),
        }
    }

    fn read_lines(&self, runtime_dir: &Path) -> Result<Vec<String>> {
        let path = runtime_dir.join(JOURNAL_FILE_NAME);
        let body = match std::fs::read_to_string(&path) {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read `{}`", path.display()));
            }
        };
        Ok(body
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string)
            .collect())
    }
}

impl GitOperationJournalPort for GitOperationJournalAdapter {
    fn append(&self, repository_dir: &str, record: &GitOperationRecord) -> Result<()> {
        let runtime_dir = (self.resolve_runtime_dir)(repository_dir)?;
        let _write = self
            .write
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut lines = self.read_lines(&runtime_dir)?;
        lines.push(serde_json::to_string(record)?);
        let keep_from = lines.len().saturating_sub(JOURNAL_LINE_LIMIT);
        let mut body = lines[keep_from..].join("\n");
        body.push('\n');
        let next = runtime_dir.join(JOURNAL_NEXT_FILE_NAME);
        std::fs::write(&next, body)
            .with_context(|| format!("failed to write `{}`", next.display()))?;
        std::fs::rename(&next, runtime_dir.join(JOURNAL_FILE_NAME))
            .context("failed to replace the git operation journal")
    }

    fn load(&self, repository_dir: &str) -> Result<Vec<GitOperationRecord>> {
        let runtime_dir = (self.resolve_runtime_dir)(repository_dir)?;
        let records = self
            .read_lines(&runtime_dir)?
            .iter()
            // 읽지 못한 줄은 건너뛴다. 한 줄이 깨져도 다른 작업은 보이고 되돌릴 수 있어야 한다.
            .filter_map(|line| serde_json::from_str::<GitOperationRecord>(line).ok())
            .collect();
        Ok(fold_git_operation_records(records))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::git_operation::{GitHeadPosition, GitOperation, GitOperationStatus};

    fn temp_runtime_dir(label: &str) -> PathBuf {
        let directory = crate::test_utils::platform_safe_temp_dir()
            .join(format!("akra-git-journal-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn record(op_id: &str, status: GitOperationStatus) -> GitOperationRecord {
        let position = |commit: &str| GitHeadPosition {
            commit: commit.to_string(),
            branch: Some("main".to_string()),
        };
        GitOperationRecord {
            pick_index: None,
            op_id: op_id.to_string(),
            operation: GitOperation::Reset {
                target: "new".to_string(),
            },
            worktree: "/repo".to_string(),
            before: position("old"),
            after: position("new"),
            backup_ref: Some(format!("refs/akra/backup/main/{op_id}")),
            status,
            started_at: 10,
            finished_at: None,
            error: None,
            undo_of: None,
            undone_by: None,
        }
    }

    #[test]
    fn records_survive_a_new_adapter_and_the_last_line_of_an_operation_wins() {
        let directory = temp_runtime_dir("fold");
        let resolved = directory.clone();
        let journal = GitOperationJournalAdapter::new(move |_| Ok(resolved.clone()));
        assert!(journal.load("/repo").unwrap().is_empty());

        journal
            .append("/repo", &record("a", GitOperationStatus::Started))
            .unwrap();
        journal
            .append("/repo", &record("b", GitOperationStatus::Done))
            .unwrap();
        journal
            .append("/repo", &record("a", GitOperationStatus::Done))
            .unwrap();

        // 데몬이 다시 시작한 뒤이다.
        let resolved = directory.clone();
        let reopened = GitOperationJournalAdapter::new(move |_| Ok(resolved.clone()));
        let records = reopened.load("/repo").unwrap();
        assert_eq!(
            records
                .iter()
                .map(|record| (record.op_id.as_str(), record.status))
                .collect::<Vec<_>>(),
            [
                ("a", GitOperationStatus::Done),
                ("b", GitOperationStatus::Done)
            ]
        );
        assert_eq!(records[0], record("a", GitOperationStatus::Done));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_broken_line_does_not_hide_the_other_operations() {
        let directory = temp_runtime_dir("broken");
        let resolved = directory.clone();
        let journal = GitOperationJournalAdapter::new(move |_| Ok(resolved.clone()));
        journal
            .append("/repo", &record("a", GitOperationStatus::Done))
            .unwrap();
        let path = directory.join(JOURNAL_FILE_NAME);
        let mut body = std::fs::read_to_string(&path).unwrap();
        body.push_str("{ not json\n");
        std::fs::write(&path, body).unwrap();
        journal
            .append("/repo", &record("b", GitOperationStatus::Done))
            .unwrap();

        let ids: Vec<String> = journal
            .load("/repo")
            .unwrap()
            .into_iter()
            .map(|record| record.op_id)
            .collect();

        assert_eq!(ids, ["a", "b"]);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_journal_keeps_only_the_newest_lines() {
        let directory = temp_runtime_dir("limit");
        let resolved = directory.clone();
        let journal = GitOperationJournalAdapter::new(move |_| Ok(resolved.clone()));
        let old_lines: Vec<String> = (0..JOURNAL_LINE_LIMIT)
            .map(|index| {
                serde_json::to_string(&record(&format!("old-{index}"), GitOperationStatus::Done))
                    .unwrap()
            })
            .collect();
        std::fs::write(
            directory.join(JOURNAL_FILE_NAME),
            format!("{}\n", old_lines.join("\n")),
        )
        .unwrap();

        journal
            .append("/repo", &record("new", GitOperationStatus::Done))
            .unwrap();

        let records = journal.load("/repo").unwrap();
        assert_eq!(records.len(), JOURNAL_LINE_LIMIT);
        assert_eq!(records[0].op_id, "old-1");
        assert_eq!(records.last().unwrap().op_id, "new");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_folder_without_a_runtime_directory_is_an_error() {
        let journal =
            GitOperationJournalAdapter::new(|_| anyhow::bail!("no runtime directory here"));
        assert!(journal.load("/repo").is_err());
        assert!(
            journal
                .append("/repo", &record("a", GitOperationStatus::Done))
                .is_err()
        );
    }
}
