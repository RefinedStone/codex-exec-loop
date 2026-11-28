//! 주의 항목 조회이다 (plan 23 §6 1단계, #2481 A02 첫 조각).
//!
//! 사람이 지금 답해야 하는 일을 커널 스냅샷의 원본에서 조회할 때마다 계산한다. 원본을 복사해 저장하지 않는다.
//! 첫 원인은 세 종류이다: 승인 대기, 수락을 기다리는 제안 task, 사람이 풀어야 하는 PR 검증. 읽음과 미루기는 없다.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::next::{NextStep, attention_next};
use crate::core::app::ThreadActivitySummary;
use crate::domain::conversation::ConversationApprovalRequest;
use crate::domain::issue_automation::decision::{IssueDecisionState, StoredIssueDecision};
use crate::domain::parallel_mode::{PrValidationOperatorState, PrValidationOperatorSummary};
use crate::domain::planning::{PriorityQueueTask, RuntimeProjection, TaskStatus};

mod stored;
pub(crate) use stored::{delivery_wait_items, terminal_turn_item};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FleetAttentionRequest {
    pub workspace: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct FleetAttention {
    pub kernel: String,
    pub items: Vec<AttentionItem>,
}

#[cfg(test)]
mod tests;

/// 사람이 답해야 하는 일 하나이다. 답하면 원본이 바뀌어 목록에서 나간다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AttentionItem {
    /// `<subject.kind>:<subject.id>:<revision>` 이다. 같은 원본 판이면 같은 값이다.
    pub id: String,
    pub cause: AttentionCause,
    pub subject: AttentionSubject,
    /// 원본의 판이다: 승인은 `approval_id`, task 는 `updated_at`, PR 검증은 `observation_revision` 이다.
    pub revision: String,
    /// 원본이 가진 사람이 읽는 글이다. 판단에는 `cause` 를 쓴다.
    pub summary: String,
    /// 원본이 기록한 마지막 변경 시각이다. 원본에 시각이 없으면(승인, PR 검증) 보내지 않는다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// 이 원인 뒤의 다음 행동이다.
    pub next: Vec<NextStep>,
}

/// 주의 항목의 원인이다. 닫힌 목록이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttentionCause {
    /// thread 의 승인 요청이 결정을 기다린다.
    ApprovalPending,
    /// planning 의 제안 task 가 수락을 기다린다.
    TaskProposed,
    /// PR 검증이 막혔다. PR 을 다시 열거나 바꿔야 한다.
    PrValidationBlocked,
    /// PR 검증이 실패했다. 새 기록으로 다시 해야 한다.
    PrValidationFailed,
    /// The durable issue decision still needs an answer.
    IssueDecisionPending,
    /// The planning authority cannot be read or validated.
    PlanningInvalid,
    /// Planning has withheld automatic continuation.
    AutoFollowPaused,
    /// A task explicitly awaits the user.
    TaskAwaitingUser,
    /// The latest kernel-owned turn failed, was interrupted or was lost.
    TurnFailed,
    TurnInterrupted,
    TurnLost,
    /// Delivery is currently held on a typed evidence wait.
    CriteriaNotMet,
    EvidenceStale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AttentionSubject {
    pub kind: AttentionSubjectKind,
    /// thread id, task id, 또는 PR 번호이다.
    pub id: String,
    /// Durable navigation coordinate for decisions opened before this client attached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<crate::domain::kernel_event::IssueEventReference>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttentionSubjectKind {
    Thread,
    Task,
    PullRequest,
    IssueDecision,
    Repository,
}

impl AttentionSubjectKind {
    fn name(self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::Task => "task",
            Self::PullRequest => "pull_request",
            Self::IssueDecision => "issue_decision",
            Self::Repository => "repository",
        }
    }
}

/// 원본 세 곳에서 주의 항목을 만든다. 순서는 승인, PR 검증, 제안 task 이다.
pub fn attention_items(
    threads: &[ThreadActivitySummary],
    pr_validation: Option<&PrValidationOperatorSummary>,
    proposed_tasks: &[PriorityQueueTask],
) -> Vec<AttentionItem> {
    let approvals = threads.iter().filter_map(|thread| {
        thread
            .approval
            .as_ref()
            .map(|approval| approval_item(&thread.thread_id, approval))
    });
    let pull_request = pr_validation.and_then(pr_validation_item);
    let tasks = proposed_tasks.iter().map(proposed_task_item);
    approvals.chain(pull_request).chain(tasks).collect()
}

/// These projections already belong to the planning authority. Do not infer
/// awaiting-user from free-form skip text or mark ordinary idle work as attention.
pub(crate) fn snapshot_attention(
    planning: &RuntimeProjection,
    threads: &[ThreadActivitySummary],
    pr: Option<&PrValidationOperatorSummary>,
) -> Vec<AttentionItem> {
    let mut items = attention_items(
        threads,
        pr,
        planning
            .queue_projection()
            .map_or(&[], |q| q.proposed_tasks.as_slice()),
    );
    items.extend(planning_attention(planning));
    items
}

fn planning_attention(projection: &RuntimeProjection) -> Vec<AttentionItem> {
    let mut items = Vec::new();
    if let Some(reason) = projection.failure_reason() {
        items.push(planning_reason_item(
            AttentionCause::PlanningInvalid,
            "planning",
            reason,
            projection.planning_revision(),
            "Planning cannot be read; review the planning authority.",
        ));
    }
    if let Some(reason) = projection.auto_follow_pause_reason() {
        items.push(planning_reason_item(
            AttentionCause::AutoFollowPaused,
            "auto_follow",
            reason,
            projection.planning_revision(),
            "Automatic continuation is withheld; review the planning authority.",
        ));
    }
    if let Some(queue) = projection.queue_projection() {
        for task in &queue.skipped_tasks {
            if task.status == TaskStatus::AwaitingUser {
                items.push(item(
                    AttentionCause::TaskAwaitingUser,
                    AttentionSubjectKind::Task,
                    task.task_id.clone(),
                    revision_for(&task.reason, projection.planning_revision()),
                    task.task_title.clone(),
                    None,
                ));
            }
        }
    }
    items
}

fn revision_for(reason: &str, revision: Option<i64>) -> String {
    // A stable digest distinguishes changed authority even without a numeric
    // revision. Do not include potentially sensitive error detail in the item.
    let digest = Sha256::digest(reason.as_bytes());
    format!("{}:{digest:x}", revision.unwrap_or_default())
}

fn planning_reason_item(
    cause: AttentionCause,
    id: &str,
    reason: &str,
    revision: Option<i64>,
    summary: &str,
) -> AttentionItem {
    item(
        cause,
        AttentionSubjectKind::Repository,
        id.to_owned(),
        revision_for(reason, revision),
        summary.to_owned(),
        None,
    )
}

/// The id/revision is the existing decision's own authority, not a new request.
pub(crate) fn issue_decision_attention(stored: &StoredIssueDecision) -> Option<AttentionItem> {
    let request = &stored.request;
    if request.state() != IssueDecisionState::Open {
        return None;
    }
    let mut decision = item(
        AttentionCause::IssueDecisionPending,
        AttentionSubjectKind::IssueDecision,
        request.id().as_str().to_owned(),
        stored.revision.to_string(),
        request.definition().question.as_str().to_owned(),
        None,
    );
    decision.subject.issue = Some(request.definition().subject.issue().into());
    Some(decision)
}

fn approval_item(thread_id: &str, approval: &ConversationApprovalRequest) -> AttentionItem {
    item(
        AttentionCause::ApprovalPending,
        AttentionSubjectKind::Thread,
        thread_id.to_string(),
        approval.approval_id.clone(),
        approval.summary.clone(),
        None,
    )
}

fn proposed_task_item(task: &PriorityQueueTask) -> AttentionItem {
    item(
        AttentionCause::TaskProposed,
        AttentionSubjectKind::Task,
        task.task_id.clone(),
        task.updated_at.clone(),
        task.task_title.clone(),
        Some(task.updated_at.clone()),
    )
}

/// 사람이 풀어야 하는 상태만 항목이 된다. 대기와 자동 수정 중(`blocking`, `remediation`)은 커널이 처리한다.
fn pr_validation_item(summary: &PrValidationOperatorSummary) -> Option<AttentionItem> {
    let cause = match summary.state {
        PrValidationOperatorState::Blocked => AttentionCause::PrValidationBlocked,
        PrValidationOperatorState::Failed => AttentionCause::PrValidationFailed,
        PrValidationOperatorState::Pending
        | PrValidationOperatorState::Blocking
        | PrValidationOperatorState::Remediation
        | PrValidationOperatorState::Terminal => return None,
    };
    Some(item(
        cause,
        AttentionSubjectKind::PullRequest,
        summary.pull_request_number.to_string(),
        summary.observation_revision.to_string(),
        summary
            .blocker
            .clone()
            .unwrap_or_else(|| summary.reason.clone()),
        None,
    ))
}

fn item(
    cause: AttentionCause,
    kind: AttentionSubjectKind,
    subject_id: String,
    revision: String,
    summary: String,
    updated_at: Option<String>,
) -> AttentionItem {
    AttentionItem {
        id: format!("{}:{subject_id}:{revision}", kind.name()),
        cause,
        subject: AttentionSubject {
            kind,
            id: subject_id,
            issue: None,
        },
        revision,
        summary,
        updated_at,
        next: attention_next(cause),
    }
}
