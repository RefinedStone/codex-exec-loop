/*
 * git 작업 엔진(plan 27 G3)의 값 타입과 규칙이다. I/O 가 없다.
 *
 * 모든 쓰기 작업은 계획이다 (§3.1). 계획은 기대 상태와 영향을 담는다. 실행은 계획을 다시 만들어 기대 상태가 같은지
 * 본다. 브랜치를 옮기는 작업은 먼저 백업 ref 를 쓴다 (§3.2). 끝난 작업은 일지에 남고, 저장소가 작업 후의 상태와
 * 같을 때만 되돌린다 (§4.2).
 */
pub mod cherry_pick;
use cherry_pick::PickPreview;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::git_graph::GIT_BACKUP_REF_PREFIX;

/// 병렬 작업 슬롯의 브랜치 이름의 앞부분이다. 이 브랜치와 이 브랜치를 체크아웃한 작업 트리는 병렬 런타임의 것이다.
/// 작업 엔진은 고치지 않는다 (§4).
pub const GIT_AGENT_BRANCH_PREFIX: &str = "akra-agent/";
/// 브랜치마다 이 수의 최근 백업은 나이와 관계없이 남긴다 (Q-G3).
pub const GIT_BACKUP_KEEP_PER_BRANCH: usize = 20;
/// 이 기간이 지나지 않은 백업은 수와 관계없이 남긴다 (Q-G3).
pub const GIT_BACKUP_KEEP_SECONDS: i64 = 30 * 24 * 60 * 60;
/// 일지 조회가 돌려주는 작업 수의 최댓값이다.
pub const GIT_OPERATION_LIST_LIMIT: usize = 50;

const OPERATION_ID_TIME_FORMAT: &str = "%Y%m%dT%H%M%SZ";

/// 작업 하나이다. 커밋은 전체 sha 이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GitOperation {
    CherryPick {
        commits: Vec<String>,
    },
    /// 현재 브랜치를 `target` 으로 옮긴다. 작업 트리도 그 커밋이 된다.
    Reset {
        target: String,
    },
    /// 어느 작업 트리도 체크아웃하지 않은 브랜치를 `target` 으로 옮긴다. 작업 트리를 건드리지 않는다.
    MoveBranch {
        branch: String,
        target: String,
    },
    /// 브랜치를 체크아웃한다.
    CheckoutBranch {
        branch: String,
    },
    /// 커밋을 브랜치 없이 체크아웃한다.
    CheckoutCommit {
        target: String,
    },
}

/// HEAD 나 브랜치의 자리이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHeadPosition {
    pub commit: String,
    /// 브랜치의 짧은 이름이다. 브랜치 없이 체크아웃한 HEAD 는 없다.
    pub branch: Option<String>,
}

/// 계획을 만들 때 본 상태이다. 실행할 때 같아야 한다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOperationExpectation {
    /// 이 작업 트리의 HEAD 이다. 커밋이 없는 저장소는 없다.
    pub head: Option<GitHeadPosition>,
    /// `MoveBranch` 가 옮기는 브랜치, 또는 `CheckoutBranch` 가 체크아웃하는 브랜치의 지금 커밋이다.
    pub branch_commit: Option<String>,
}

/// 브랜치를 옮기는 작업의 영향이다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOperationImpact {
    /// 브랜치에서 빠지는 커밋 수이다.
    pub dropped: usize,
    /// 빠지는 커밋 가운데 다른 브랜치와 태그가 닿지 못하는 수이다. 백업 ref 만 이 커밋을 지킨다.
    pub unreachable: usize,
    /// 브랜치에 새로 들어오는 커밋 수이다.
    pub gained: usize,
}

/// 계획을 실행하지 못하는 이유의 종류이다. 화면은 이 값으로 글을 고른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitOperationBlockerCode {
    NotRepository,
    NoCommits,
    /// 저장소 설정이 프로그램을 실행한다 (필터, hook 대체 등). Akra 는 그런 저장소에서 쓰기 명령을 돌리지 않는다.
    ExecutionConfig,
    /// merge, rebase, cherry-pick, revert, bisect 가 끝나지 않았다.
    OperationInProgress,
    UnknownCommit,
    UnknownBranch,
    /// HEAD 가 브랜치가 아니다. 옮길 현재 브랜치가 없다.
    DetachedHead,
    /// 추적하는 파일에 커밋하지 않은 변경이 있다.
    UncommittedChanges,
    /// 이미 그 자리이다.
    NothingToDo,
    /// 브랜치를 어느 작업 트리가 체크아웃했다.
    BranchCheckedOut,
    /// 병렬 작업 슬롯의 브랜치나 작업 트리이다.
    AgentWorktree,
    /// 이 작업 트리에 폴더가 있는 스레드의 턴이 실행 중이다 (#2850).
    TurnRunning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitOperationBlocker {
    pub code: GitOperationBlockerCode,
    /// 영어 한 문장이다. 화면이 `code` 를 모를 때 보인다.
    pub message: String,
}

impl GitOperationBlocker {
    pub fn new(code: GitOperationBlockerCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// 작업 하나의 계획이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOperationPlan {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub picks: Vec<PickPreview>,
    pub operation: GitOperation,
    pub expectation: GitOperationExpectation,
    /// 옮기는 브랜치이다. 체크아웃은 없다.
    pub branch: Option<String>,
    /// 작업 전의 자리이다. 브랜치를 옮기는 작업은 그 브랜치의 자리이고, 체크아웃은 HEAD 의 자리이다.
    pub before: Option<GitHeadPosition>,
    /// 작업 뒤의 자리이다. 대상 커밋이나 브랜치를 찾지 못하면 없다.
    pub after: Option<GitHeadPosition>,
    /// 브랜치를 옮기는 작업에만 있다.
    pub impact: Option<GitOperationImpact>,
    /// 비어 있어야 실행한다.
    pub blockers: Vec<GitOperationBlocker>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitOperationStatus {
    /// 시작을 적었고 끝을 적지 못했다. 데몬이 그 사이에 죽었다.
    Started,
    Paused,
    Aborted,
    Done,
    Failed,
}

/// 일지의 작업 하나이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOperationRecord {
    /// Last journalled index in an ordered cherry-pick. `after` is the last verified tip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pick_index: Option<usize>,
    pub op_id: String,
    pub operation: GitOperation,
    /// 작업을 실행한 폴더이다.
    pub worktree: String,
    pub before: GitHeadPosition,
    pub after: GitHeadPosition,
    /// 작업 전의 커밋을 가리키는 백업 ref 의 전체 이름이다. 체크아웃은 없다.
    pub backup_ref: Option<String>,
    pub status: GitOperationStatus,
    /// Unix 초이다.
    pub started_at: i64,
    pub finished_at: Option<i64>,
    /// 실패한 작업에 git 이 쓴 글이다.
    pub error: Option<String>,
    /// 이 작업이 되돌린 작업의 id 이다.
    pub undo_of: Option<String>,
    /// 이 작업을 되돌린 작업의 id 이다.
    pub undone_by: Option<String>,
}

impl GitOperationRecord {
    /// 이 작업이 작업 트리 하나의 HEAD 를 바꿨는가. 그런 작업은 그 작업 트리에서만 되돌린다. 브랜치 이동은 ref 만
    /// 바꾸므로 어느 작업 트리에서나 되돌린다.
    pub fn is_bound_to_worktree(&self) -> bool {
        !matches!(self.operation, GitOperation::MoveBranch { .. })
    }

    /// 되돌릴 수 있는 기록이다: 끝났고, 아직 되돌리지 않았고, 되돌리기 자신이 아니다. 저장소가 작업 후의 상태와
    /// 같은지는 따로 본다.
    pub fn can_undo(&self) -> bool {
        self.status == GitOperationStatus::Done
            && self.undone_by.is_none()
            && self.undo_of.is_none()
    }

    /// 이 작업을 되돌리는 작업이다.
    pub fn inverse(&self) -> GitOperation {
        match (&self.operation, &self.before.branch) {
            (GitOperation::Reset { .. } | GitOperation::CherryPick { .. }, _) => {
                GitOperation::Reset {
                    target: self.before.commit.clone(),
                }
            }
            (GitOperation::MoveBranch { branch, .. }, _) => GitOperation::MoveBranch {
                branch: branch.clone(),
                target: self.before.commit.clone(),
            },
            (
                GitOperation::CheckoutBranch { .. } | GitOperation::CheckoutCommit { .. },
                Some(branch),
            ) => GitOperation::CheckoutBranch {
                branch: branch.clone(),
            },
            (GitOperation::CheckoutBranch { .. } | GitOperation::CheckoutCommit { .. }, None) => {
                GitOperation::CheckoutCommit {
                    target: self.before.commit.clone(),
                }
            }
        }
    }

    /// 저장소가 이 작업 후의 상태 그대로인가. `head` 는 지금의 HEAD 이고, `branch_commit` 은 `MoveBranch` 가 옮긴
    /// 브랜치의 지금 커밋이다.
    pub fn still_applies(
        &self,
        head: Option<&GitHeadPosition>,
        branch_commit: Option<&str>,
    ) -> bool {
        match &self.operation {
            GitOperation::Reset { .. } | GitOperation::CherryPick { .. } => {
                head == Some(&self.after)
            }
            GitOperation::MoveBranch { .. } => branch_commit == Some(self.after.commit.as_str()),
            // 체크아웃한 브랜치에 커밋을 더해도 되돌릴 수 있다. 돌아가도 잃는 것이 없다.
            GitOperation::CheckoutBranch { .. } => {
                head.is_some_and(|head| head.branch == self.after.branch)
            }
            GitOperation::CheckoutCommit { .. } => head == Some(&self.after),
        }
    }
}

/// 두 글이 같은 작업 트리 폴더인가. 구분자와 끝의 구분자를 가리지 않는다. Windows 의 드라이브 경로는 대소문자도
/// 가리지 않는다. 파일 시스템을 보지 않는다: 링크로 이어진 두 이름은 다른 폴더로 본다.
pub fn same_git_worktree_path(left: &str, right: &str) -> bool {
    fn comparable(path: &str) -> String {
        let slashed = path.replace('\\', "/");
        let trimmed = slashed.trim_end_matches('/');
        let drive = trimmed.as_bytes().get(1) == Some(&b':');
        if drive || trimmed.starts_with("//") {
            trimmed.to_ascii_lowercase()
        } else {
            trimmed.to_string()
        }
    }
    comparable(left) == comparable(right)
}

/// `folder` 를 담은 작업 트리이다. 겹치면 가장 깊은 것이다. `/repo-other` 는 `/repo` 안이 아니다.
pub fn git_worktree_containing<'a>(
    worktrees: &'a [crate::domain::git_graph::GitGraphWorktree],
    folder: &str,
) -> Option<&'a crate::domain::git_graph::GitGraphWorktree> {
    worktrees
        .iter()
        .filter(|worktree| git_path_contains(&worktree.path, folder))
        .max_by_key(|worktree| worktree.path.len())
}

fn git_path_contains(root: &str, folder: &str) -> bool {
    let root = root.replace('\\', "/");
    let root = root.trim_end_matches('/');
    let folder = folder.replace('\\', "/");
    if same_git_worktree_path(root, &folder) {
        return true;
    }
    // Compare the root-length prefix with the same case rule, then require a separator.
    folder
        .get(..root.len())
        .is_some_and(|prefix| same_git_worktree_path(prefix, root))
        && folder.as_bytes().get(root.len()) == Some(&b'/')
}

/// 일지의 줄들을 작업마다 하나로 접는다. 같은 id 의 뒤 줄이 앞 줄을 대신한다. 순서는 처음 나온 순서이다.
pub fn fold_git_operation_records(lines: Vec<GitOperationRecord>) -> Vec<GitOperationRecord> {
    let mut folded: Vec<GitOperationRecord> = Vec::new();
    for line in lines {
        match folded.iter_mut().find(|record| record.op_id == line.op_id) {
            Some(record) => *record = line,
            None => folded.push(line),
        }
    }
    folded
}

/// 작업 id 이다. 시각이 앞에 있어서 이름순이 시간순이다. `nonce` 는 같은 초의 작업을 가른다.
pub fn git_operation_id(unix_seconds: i64, nonce: u32) -> String {
    let time = DateTime::<Utc>::from_timestamp(unix_seconds, 0).unwrap_or_default();
    format!("{}-{nonce:08x}", time.format(OPERATION_ID_TIME_FORMAT))
}

/// 작업 id 에서 시각을 읽는다. Unix 초이다.
pub fn git_operation_id_time(op_id: &str) -> Option<i64> {
    let (time, _) = op_id.split_once('-')?;
    NaiveDateTime::parse_from_str(time, OPERATION_ID_TIME_FORMAT)
        .ok()
        .map(|time| time.and_utc().timestamp())
}

/// 백업 ref 의 전체 이름이다.
pub fn git_backup_ref_name(branch: &str, op_id: &str) -> String {
    format!("{GIT_BACKUP_REF_PREFIX}{branch}/{op_id}")
}

/// 지울 백업 ref 들의 짧은 이름(`<branch>/<op-id>`)이다. 브랜치마다 최근 `GIT_BACKUP_KEEP_PER_BRANCH` 개와
/// `GIT_BACKUP_KEEP_SECONDS` 안의 것은 남긴다. 둘 다 벗어난 것만 지운다. id 에서 시각을 읽지 못한 ref 는 남긴다.
pub fn git_backups_to_prune(backup_names: &[String], now_unix_seconds: i64) -> Vec<String> {
    let mut by_branch: std::collections::BTreeMap<&str, Vec<(i64, &String)>> = Default::default();
    for name in backup_names {
        let Some((branch, op_id)) = name.rsplit_once('/') else {
            continue;
        };
        if let Some(time) = git_operation_id_time(op_id) {
            by_branch.entry(branch).or_default().push((time, name));
        }
    }
    let mut prune = Vec::new();
    for backups in by_branch.values_mut() {
        backups.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(left.1)));
        prune.extend(
            backups
                .iter()
                .skip(GIT_BACKUP_KEEP_PER_BRANCH)
                .filter(|(time, _)| now_unix_seconds - time > GIT_BACKUP_KEEP_SECONDS)
                .map(|(_, name)| (*name).clone()),
        );
    }
    prune
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(commit: &str, branch: Option<&str>) -> GitHeadPosition {
        GitHeadPosition {
            commit: commit.to_string(),
            branch: branch.map(str::to_string),
        }
    }

    fn record(
        op_id: &str,
        operation: GitOperation,
        before: GitHeadPosition,
        after: GitHeadPosition,
    ) -> GitOperationRecord {
        GitOperationRecord {
            pick_index: None,
            op_id: op_id.to_string(),
            operation,
            worktree: "/repo".to_string(),
            before,
            after,
            backup_ref: None,
            status: GitOperationStatus::Done,
            started_at: 1,
            finished_at: Some(2),
            error: None,
            undo_of: None,
            undone_by: None,
        }
    }

    #[test]
    fn operation_wire_shape_is_tagged_by_kind() {
        let reset = serde_json::to_value(GitOperation::Reset {
            target: "abc".to_string(),
        })
        .expect("serialize");
        assert_eq!(
            reset,
            serde_json::json!({ "kind": "reset", "target": "abc" })
        );
        let moved: GitOperation = serde_json::from_value(serde_json::json!({
            "kind": "move_branch", "branch": "topic", "target": "abc"
        }))
        .expect("deserialize");
        assert_eq!(
            moved,
            GitOperation::MoveBranch {
                branch: "topic".to_string(),
                target: "abc".to_string()
            }
        );
    }

    #[test]
    fn inverse_returns_each_operation_to_its_before_position() {
        let reset = record(
            "a",
            GitOperation::Reset {
                target: "new".to_string(),
            },
            position("old", Some("main")),
            position("new", Some("main")),
        );
        assert_eq!(
            reset.inverse(),
            GitOperation::Reset {
                target: "old".to_string()
            }
        );

        let moved = record(
            "b",
            GitOperation::MoveBranch {
                branch: "topic".to_string(),
                target: "new".to_string(),
            },
            position("old", Some("topic")),
            position("new", Some("topic")),
        );
        assert_eq!(
            moved.inverse(),
            GitOperation::MoveBranch {
                branch: "topic".to_string(),
                target: "old".to_string()
            }
        );

        let from_branch = record(
            "c",
            GitOperation::CheckoutCommit {
                target: "new".to_string(),
            },
            position("old", Some("main")),
            position("new", None),
        );
        assert_eq!(
            from_branch.inverse(),
            GitOperation::CheckoutBranch {
                branch: "main".to_string()
            }
        );

        let from_detached = record(
            "d",
            GitOperation::CheckoutBranch {
                branch: "main".to_string(),
            },
            position("old", None),
            position("new", Some("main")),
        );
        assert_eq!(
            from_detached.inverse(),
            GitOperation::CheckoutCommit {
                target: "old".to_string()
            }
        );
    }

    #[test]
    fn undo_needs_the_state_the_operation_left() {
        let reset = record(
            "a",
            GitOperation::Reset {
                target: "new".to_string(),
            },
            position("old", Some("main")),
            position("new", Some("main")),
        );
        assert!(reset.still_applies(Some(&position("new", Some("main"))), None));
        // 그 뒤에 커밋을 더했거나 다른 브랜치로 갔다.
        assert!(!reset.still_applies(Some(&position("newer", Some("main"))), None));
        assert!(!reset.still_applies(Some(&position("new", Some("topic"))), None));

        let moved = record(
            "b",
            GitOperation::MoveBranch {
                branch: "topic".to_string(),
                target: "new".to_string(),
            },
            position("old", Some("topic")),
            position("new", Some("topic")),
        );
        assert!(moved.still_applies(None, Some("new")));
        assert!(!moved.still_applies(None, Some("other")));
        assert!(!moved.still_applies(None, None));

        let checkout = record(
            "c",
            GitOperation::CheckoutBranch {
                branch: "topic".to_string(),
            },
            position("old", Some("main")),
            position("new", Some("topic")),
        );
        assert!(checkout.still_applies(Some(&position("newer", Some("topic"))), None));
        assert!(!checkout.still_applies(Some(&position("new", Some("main"))), None));
    }

    #[test]
    fn worktree_paths_compare_without_separator_and_drive_case_differences() {
        assert!(same_git_worktree_path("C:\\dev\\repo\\", "c:/dev/repo"));
        assert!(same_git_worktree_path("/home/u/repo/", "/home/u/repo"));
        assert!(!same_git_worktree_path("/home/u/Repo", "/home/u/repo"));
        assert!(!same_git_worktree_path("C:/dev/repo", "C:/dev/repo-topic"));
    }

    #[test]
    fn only_a_branch_move_is_free_of_a_worktree() {
        let reset = record(
            "a",
            GitOperation::Reset {
                target: "new".to_string(),
            },
            position("old", Some("main")),
            position("new", Some("main")),
        );
        assert!(reset.is_bound_to_worktree());
        let moved = record(
            "b",
            GitOperation::MoveBranch {
                branch: "topic".to_string(),
                target: "new".to_string(),
            },
            position("old", Some("topic")),
            position("new", Some("topic")),
        );
        assert!(!moved.is_bound_to_worktree());
    }

    #[test]
    fn only_a_finished_operation_that_is_not_an_undo_can_be_undone() {
        let mut done = record(
            "a",
            GitOperation::CheckoutBranch {
                branch: "topic".to_string(),
            },
            position("old", Some("main")),
            position("new", Some("topic")),
        );
        assert!(done.can_undo());
        done.undone_by = Some("b".to_string());
        assert!(!done.can_undo());
        done.undone_by = None;
        done.undo_of = Some("z".to_string());
        assert!(!done.can_undo());
        done.undo_of = None;
        done.status = GitOperationStatus::Failed;
        assert!(!done.can_undo());
        done.status = GitOperationStatus::Started;
        assert!(!done.can_undo());
    }

    #[test]
    fn a_later_journal_line_replaces_the_earlier_line_of_the_same_operation() {
        let mut started = record(
            "a",
            GitOperation::CheckoutBranch {
                branch: "topic".to_string(),
            },
            position("old", Some("main")),
            position("new", Some("topic")),
        );
        started.status = GitOperationStatus::Started;
        let other = record(
            "b",
            GitOperation::CheckoutBranch {
                branch: "main".to_string(),
            },
            position("new", Some("topic")),
            position("old", Some("main")),
        );
        let mut done = started.clone();
        done.status = GitOperationStatus::Done;

        let folded = fold_git_operation_records(vec![started, other.clone(), done.clone()]);

        assert_eq!(folded, vec![done, other]);
    }

    #[test]
    fn operation_id_sorts_by_time_and_gives_the_time_back() {
        let id = git_operation_id(1_795_000_000, 0xab);
        assert_eq!(id, "20261118T110640Z-000000ab");
        assert_eq!(git_operation_id_time(&id), Some(1_795_000_000));
        assert!(git_operation_id(1_795_000_001, 0) > id);
        assert_eq!(git_operation_id_time("op-1"), None);
        assert_eq!(
            git_backup_ref_name("feature/a", &id),
            "refs/akra/backup/feature/a/20261118T110640Z-000000ab"
        );
    }

    #[test]
    fn prune_drops_only_backups_that_are_both_old_and_beyond_the_kept_count() {
        let now = 1_800_000_000;
        let day = 24 * 60 * 60;
        let old = |index: i64| format!("main/{}", git_operation_id(now - 60 * day - index, 0));
        let recent = |index: i64| format!("main/{}", git_operation_id(now - index, 0));
        // 오래된 것 22 개: 최근 20 개에 들지 못한 2 개만 지운다.
        let many_old: Vec<String> = (0..22).map(old).collect();
        let pruned = git_backups_to_prune(&many_old, now);
        assert_eq!(pruned, vec![old(20), old(21)]);

        // 새 것 25 개: 수를 넘어도 기간 안이므로 지우지 않는다.
        let many_recent: Vec<String> = (0..25).map(recent).collect();
        assert!(git_backups_to_prune(&many_recent, now).is_empty());

        // 브랜치마다 따로 센다. 읽지 못한 id 는 남긴다.
        let mut mixed = many_old.clone();
        mixed.push(format!("feature/a/{}", git_operation_id(now - 90 * day, 0)));
        mixed.push("main/op-1".to_string());
        assert_eq!(git_backups_to_prune(&mixed, now), vec![old(20), old(21)]);
    }
}
