//! 사람의 추가 지시를 이미 실행 중인 동일 작업의 턴으로 전달하는 경계다.
//! 새 모델 턴을 시작하지 않으며, 전송 영수증의 내구성/중복 방지는 호출 서비스가 소유한다.
use crate::domain::issue_automation::intervention::IssueSteerDelivery;
use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueWorkSteerRequest {
    pub workspace_dir: String,
    pub work_order_id: String,
    pub task_ids: Vec<String>,
    pub command_id: String,
    pub message: String,
}

pub trait IssueWorkSteeringPort: Send + Sync {
    fn steer(&self, request: &IssueWorkSteerRequest) -> Result<IssueSteerDelivery>;
}

/// 살아 있는 worker를 소유하지 않은 명령 전용 커널은 전송 성공을 추정하지 않는다.
pub struct UnavailableIssueWorkSteering;
impl IssueWorkSteeringPort for UnavailableIssueWorkSteering {
    fn steer(&self, _request: &IssueWorkSteerRequest) -> Result<IssueSteerDelivery> {
        anyhow::bail!("no live issue worker is connected to this kernel")
    }
}
