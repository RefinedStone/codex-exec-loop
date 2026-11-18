use anyhow::Result;

use crate::domain::git_operation::GitOperationRecord;

/*
 * git 작업 일지의 바깥 경계다 (plan 27 G3). 일지는 저장소마다 하나이다. 같은 저장소의 모든 작업 트리가 같은 일지를
 * 쓴다. 저장소 안이나 `.git` 안에 두지 않는다: 에이전트가 작업 트리에서 고칠 수 있다.
 */
pub trait GitOperationJournalPort: Send + Sync {
    /// 기록 한 줄을 더한다. 같은 `op_id` 의 뒤 줄이 앞 줄을 대신한다.
    fn append(&self, repository_dir: &str, record: &GitOperationRecord) -> Result<()>;

    /// 작업마다 하나로 접은 기록이다. 오래된 것이 앞이다.
    fn load(&self, repository_dir: &str) -> Result<Vec<GitOperationRecord>>;
}
