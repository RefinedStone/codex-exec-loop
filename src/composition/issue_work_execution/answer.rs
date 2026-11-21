use super::AkraIssueWorkExecutionAdapter;
use crate::application::port::outbound::issue_work_execution_port::{
    IssueWorkAnswerDelivery, IssueWorkAnswerReceipt, IssueWorkAnswerRequest,
};
use crate::application::port::outbound::planning_authority_port::{
    PlanningWorkerContinuationDelivery, PlanningWorkerContinuationReceipt,
    PlanningWorkerContinuationRequest, PlanningWorkerQuestionIdentity,
};
use anyhow::Result;

pub(super) fn enqueue(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    request: &IssueWorkAnswerRequest,
) -> Result<IssueWorkAnswerReceipt> {
    let question = &request.question;
    let continuation = PlanningWorkerContinuationRequest {
        authorization_epoch: request.authorization_epoch,
        command_id: request.command_id.clone(),
        work_order_id: request.work_order_id.clone(),
        message: request.message.clone(),
        question: PlanningWorkerQuestionIdentity {
            question_id: question.question_id.clone(),
            task_id: question.task_id.clone(),
            session_key: question.session_key.clone(),
            thread_id: question.thread_id.clone(),
            slot_id: question.slot_id.clone(),
            worktree_path: question.worktree_path.clone(),
            branch_name: question.branch_name.clone(),
            lease_generation: question.lease_generation.clone(),
            awaiting_task_updated_at: question.awaiting_task_updated_at.clone(),
        },
    };
    Ok(map_receipt(
        adapter
            .authority
            .enqueue_worker_continuation(workspace, &continuation)?,
    ))
}

pub(super) fn receipt(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    work_order_id: &str,
    command_id: &str,
) -> Result<Option<IssueWorkAnswerReceipt>> {
    Ok(adapter
        .authority
        .worker_continuation_receipt(workspace, work_order_id, command_id)?
        .map(map_receipt))
}

fn map_receipt(receipt: PlanningWorkerContinuationReceipt) -> IssueWorkAnswerReceipt {
    IssueWorkAnswerReceipt {
        command_id: receipt.command_id,
        question_id: receipt.question_id,
        delivery: match receipt.delivery {
            PlanningWorkerContinuationDelivery::Queued => IssueWorkAnswerDelivery::Queued,
            PlanningWorkerContinuationDelivery::Unknown => IssueWorkAnswerDelivery::Unknown,
            PlanningWorkerContinuationDelivery::Accepted {
                thread_id,
                turn_id,
                lease_generation,
            } => IssueWorkAnswerDelivery::Accepted {
                thread_id,
                turn_id,
                lease_generation,
            },
            PlanningWorkerContinuationDelivery::RejectedBeforeStart { reason } => {
                IssueWorkAnswerDelivery::RejectedBeforeStart {
                    reason: reason.code().into(),
                }
            }
        },
    }
}
