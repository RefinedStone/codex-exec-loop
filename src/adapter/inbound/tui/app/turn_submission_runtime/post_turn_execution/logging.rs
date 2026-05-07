use serde_json::{Map, Value, json};

use super::super::super::ConversationViewModel;
use super::super::super::conversation_runtime::ConversationPostTurnAction;
use super::PostTurnEvaluationRequest;
use crate::application::service::planning::{PlanningQueueRefreshMode, PlanningRuntimeSnapshot};

pub(super) fn post_turn_event_detail<I>(
    conversation: &ConversationViewModel,
    request: &PostTurnEvaluationRequest,
    operation: &str,
    phase: &str,
    decision: Option<&str>,
    runtime: Option<&PlanningRuntimeSnapshot>,
    fields: I,
) -> Value
where
    I: IntoIterator<Item = (&'static str, Value)>,
{
    let mut detail = core_post_turn_fields(
        Some(conversation.thread_id.as_str()),
        request.completed_turn_id.as_str(),
        request.workspace_directory.as_str(),
        operation,
        phase,
        decision,
    );
    if let Some(runtime) = runtime {
        detail.insert("runtime".to_string(), runtime_snapshot_log_detail(runtime));
    }
    extend_detail(&mut detail, fields);
    Value::Object(detail)
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PostTurnWorkerLogContext<'a> {
    pub(super) thread_id: &'a str,
    pub(super) completed_turn_id: &'a str,
    pub(super) workspace_directory: &'a str,
}

impl<'a> PostTurnWorkerLogContext<'a> {
    pub(super) fn new(
        thread_id: &'a str,
        completed_turn_id: &'a str,
        workspace_directory: &'a str,
    ) -> Self {
        Self {
            thread_id,
            completed_turn_id,
            workspace_directory,
        }
    }
}

pub(super) fn post_turn_worker_event_detail<I>(
    context: PostTurnWorkerLogContext<'_>,
    operation: &str,
    phase: &str,
    decision: Option<&str>,
    runtime: Option<&PlanningRuntimeSnapshot>,
    fields: I,
) -> Value
where
    I: IntoIterator<Item = (&'static str, Value)>,
{
    let mut detail = core_post_turn_fields(
        Some(context.thread_id),
        context.completed_turn_id,
        context.workspace_directory,
        operation,
        phase,
        decision,
    );
    if let Some(runtime) = runtime {
        detail.insert("runtime".to_string(), runtime_snapshot_log_detail(runtime));
    }
    extend_detail(&mut detail, fields);
    Value::Object(detail)
}

pub(super) fn planning_worker_refresh_skipped_detail(
    conversation: &ConversationViewModel,
    request: &PostTurnEvaluationRequest,
    reason: &str,
    runtime: &PlanningRuntimeSnapshot,
) -> Value {
    post_turn_event_detail(
        conversation,
        request,
        "refresh",
        "skipped",
        Some("skip"),
        Some(runtime),
        [("reason", json!(reason))],
    )
}

fn core_post_turn_fields(
    thread_id: Option<&str>,
    completed_turn_id: &str,
    workspace_directory: &str,
    operation: &str,
    phase: &str,
    decision: Option<&str>,
) -> Map<String, Value> {
    let mut detail = Map::new();
    detail.insert("thread_id".to_string(), json!(thread_id));
    detail.insert("completed_turn_id".to_string(), json!(completed_turn_id));
    detail.insert(
        "workspace_directory".to_string(),
        json!(workspace_directory),
    );
    detail.insert("operation".to_string(), json!(operation));
    detail.insert("phase".to_string(), json!(phase));
    detail.insert("decision".to_string(), json!(decision));
    detail
}

fn extend_detail<I>(detail: &mut Map<String, Value>, fields: I)
where
    I: IntoIterator<Item = (&'static str, Value)>,
{
    for (key, value) in fields {
        detail.insert(key.to_string(), value);
    }
}

pub(super) fn runtime_snapshot_log_detail(snapshot: &PlanningRuntimeSnapshot) -> Value {
    let queue_head = snapshot.queue_head().map(|task| {
        json!({
            "task_id": task.task_id,
            "task_title": task.task_title,
            "status": format!("{:?}", task.status),
            "rank": task.rank,
            "combined_priority": task.combined_priority,
        })
    });
    json!({
        "workspace_present": snapshot.workspace_present(),
        "workspace_status": format!("{:?}", snapshot.workspace_status()),
        "queue_idle_policy": format!("{:?}", snapshot.queue_idle_policy()),
        "queue_summary": snapshot.queue_summary(),
        "proposal_summary": snapshot.proposal_summary(),
        "queue_head": queue_head,
        "failure_reason": snapshot.failure_reason(),
        "pause_reason": snapshot.auto_followup_pause_reason(),
        "has_actionable_queue_head": snapshot.has_actionable_queue_head(),
        "has_proposal_candidates": snapshot.has_proposal_candidates(),
    })
}

pub(super) fn planning_refresh_mode_label(mode: &PlanningQueueRefreshMode<'_>) -> &'static str {
    match mode {
        PlanningQueueRefreshMode::FromLatestReply => "from_latest_reply",
        PlanningQueueRefreshMode::DeriveNextTaskWhenQueueIdle { .. } => {
            "derive_next_task_when_queue_idle"
        }
    }
}

pub(super) fn post_turn_action_log_detail(action: &ConversationPostTurnAction) -> Value {
    match action {
        ConversationPostTurnAction::QueueAutoPrompt(prompt) => json!({
            "type": "queue_auto_prompt",
            "completed_turn_id": prompt.completed_turn_id,
            "mode_label": prompt.mode_label,
            "prompt_chars": prompt.prompt.chars().count(),
            "transcript_text_chars": prompt.transcript_text.chars().count(),
            "handoff_task_id": prompt
                .handoff_task
                .as_ref()
                .map(|task| task.task_id.as_str()),
        }),
        ConversationPostTurnAction::SkipAutoFollowup { reason } => json!({
            "type": "skip_auto_followup",
            "reason": format!("{:?}", reason),
        }),
    }
}

pub(super) fn post_turn_action_decision(action: &ConversationPostTurnAction) -> &'static str {
    match action {
        ConversationPostTurnAction::QueueAutoPrompt(_) => "queue_auto_prompt",
        ConversationPostTurnAction::SkipAutoFollowup { .. } => "skip_auto_followup",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::conversation_model::AutoFollowupSkipReason;
    use crate::adapter::inbound::tui::app::conversation_runtime::ConversationPostTurnAction;
    use crate::application::service::planning::PlanningRuntimeSnapshot;

    #[test]
    fn post_turn_worker_event_detail_keeps_standard_fields_stable() {
        let detail = post_turn_worker_event_detail(
            PostTurnWorkerLogContext::new("thread-1", "turn-1", "/tmp/workspace"),
            "refresh",
            "skipped",
            Some("skip"),
            None,
            [("reason", json!("latest_main_reply_empty"))],
        );

        assert_eq!(detail["thread_id"], json!("thread-1"));
        assert_eq!(detail["completed_turn_id"], json!("turn-1"));
        assert_eq!(detail["workspace_directory"], json!("/tmp/workspace"));
        assert_eq!(detail["operation"], json!("refresh"));
        assert_eq!(detail["phase"], json!("skipped"));
        assert_eq!(detail["decision"], json!("skip"));
        assert_eq!(detail["reason"], json!("latest_main_reply_empty"));
    }

    #[test]
    fn runtime_snapshot_log_detail_keeps_runtime_summary_shape_stable() {
        let detail = runtime_snapshot_log_detail(&PlanningRuntimeSnapshot::invalid(
            "planning state unavailable",
        ));

        assert_eq!(detail["workspace_status"], json!("Invalid"));
        assert_eq!(
            detail["failure_reason"],
            json!("planning state unavailable")
        );
        assert_eq!(detail["has_actionable_queue_head"], json!(false));
        assert!(detail.get("queue_head").is_some());
    }

    #[test]
    fn post_turn_action_detail_hides_full_queued_prompt_text() {
        let action = ConversationPostTurnAction::SkipAutoFollowup {
            reason: AutoFollowupSkipReason::PlanningQueueIdlePolicyStop,
        };

        let detail = post_turn_action_log_detail(&action);

        assert_eq!(detail["type"], json!("skip_auto_followup"));
        assert_eq!(detail["reason"], json!("PlanningQueueIdlePolicyStop"));
        assert_eq!(post_turn_action_decision(&action), "skip_auto_followup");
    }
}
