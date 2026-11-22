//! 원격 이슈별 개입 명령의 애플리케이션 진입점이다.
use super::stored_command_port::StoredCommandContext;
use crate::domain::issue_automation::intervention::{
    IssueInterventionReceipt, IssueInterventionRequest,
};
use anyhow::Result;

pub trait IssueInterventionPort: Send + Sync {
    fn intervene(
        &self,
        request: IssueInterventionRequest,
        context: &StoredCommandContext,
    ) -> Result<IssueInterventionReceipt>;
}
