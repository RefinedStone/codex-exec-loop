use anyhow::Result;
use serde::Serialize;

use crate::domain::git_operation::{
    GitOperation, GitOperationExpectation, GitOperationPlan, GitOperationRecord,
};

// git 작업 엔진 inbound 계약이다 (plan 27 G3). Gateway 의 `akra/git/plan`, `execute`, `operations`, `undo` 가 쓴다.

/// 실행이나 되돌리기의 결과이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GitOperationOutcome {
    /// 저장소를 고쳤다.
    Done { record: GitOperationRecord },
    /// git 명령이 실패했다. `record.error` 가 git 의 글이다. 백업 ref 는 남아 있을 수 있다.
    Failed { record: GitOperationRecord },
    /// 저장소가 계획과 다르거나 계획에 막는 이유가 있다. 고치지 않았다. `plan` 은 지금 상태의 계획이다.
    Stale { plan: GitOperationPlan },
    /// 되돌릴 수 없는 기록이다. 고치지 않았다.
    Refused { message: String },
}

pub trait GitOperationPort: Send + Sync {
    /// 작업의 계획을 만든다. 저장소를 고치지 않는다. 커밋 id 가 전체 sha 가 아니면 오류이다.
    fn plan(&self, repository_dir: &str, operation: GitOperation) -> Result<GitOperationPlan>;

    /// 계획을 다시 만들고, 기대 상태가 `expectation` 과 같고 막는 이유가 없을 때만 실행한다.
    fn execute(
        &self,
        repository_dir: &str,
        operation: GitOperation,
        expectation: GitOperationExpectation,
    ) -> Result<GitOperationOutcome>;

    /// 일지의 최근 작업들이다. 새 것이 앞이다.
    fn operations(&self, repository_dir: &str) -> Result<Vec<GitOperationRecord>>;

    /// 일지의 작업 하나를 되돌린다. 저장소가 그 작업 후의 상태와 같을 때만 한다.
    fn undo(&self, repository_dir: &str, op_id: &str) -> Result<GitOperationOutcome>;
}
