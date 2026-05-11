use serde_json::{Map, Value, json};

use super::super::super::conversation_runtime::{
    PostTurnContinuationAction, PostTurnEvaluationProvenance,
};
use crate::application::service::planning::PlanningRuntimeProjection;

pub(super) fn post_turn_event_detail<I>(
    context: PostTurnWorkerLogContext<'_>,
    operation: &str,
    phase: &str,
    decision: Option<&str>,
    runtime: Option<&PlanningRuntimeProjection>,
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
        detail.insert(
            "runtime".to_string(),
            runtime_projection_log_detail(runtime),
        );
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
    runtime: Option<&PlanningRuntimeProjection>,
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
        detail.insert(
            "runtime".to_string(),
            runtime_projection_log_detail(runtime),
        );
    }
    extend_detail(&mut detail, fields);
    Value::Object(detail)
}

pub(super) fn planning_worker_refresh_skipped_detail(
    context: PostTurnWorkerLogContext<'_>,
    reason: &str,
    runtime: &PlanningRuntimeProjection,
) -> Value {
    post_turn_event_detail(
        context,
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

pub(super) fn runtime_projection_log_detail(projection: &PlanningRuntimeProjection) -> Value {
    let queue_head = projection.queue_head().map(|task| {
        json!({
            "task_id": task.task_id,
            "task_title": task.task_title,
            "status": format!("{:?}", task.status),
            "rank": task.rank,
            "combined_priority": task.combined_priority,
        })
    });
    json!({
        "workspace_present": projection.workspace_present(),
        "workspace_status": format!("{:?}", projection.workspace_status()),
        "queue_idle_policy": format!("{:?}", projection.queue_idle_policy()),
        "queue_summary": projection.queue_summary(),
        "proposal_summary": projection.proposal_summary(),
        "queue_head": queue_head,
        "failure_reason": projection.failure_reason(),
        "pause_reason": projection.auto_follow_pause_reason(),
        "has_actionable_queue_head": projection.has_actionable_queue_head(),
        "has_proposal_candidates": projection.has_proposal_candidates(),
    })
}

pub(super) fn post_turn_action_log_detail(
    action: &PostTurnContinuationAction,
    provenance: &PostTurnEvaluationProvenance,
) -> Value {
    match action {
        PostTurnContinuationAction::QueueAutoPrompt(prompt) => json!({
            "type": "queue_auto_prompt",
            "completed_turn_id": provenance.completed_turn_id,
            "mode_label": prompt.mode_label,
            "prompt_chars": prompt.prompt.chars().count(),
            "transcript_text_chars": prompt.transcript_text.chars().count(),
            "handoff_task_id": provenance
                .handoff_task
                .as_ref()
                .map(|task| task.task_id.as_str()),
        }),
        PostTurnContinuationAction::SkipAutoFollow { reason } => json!({
            "type": "skip_auto_followup",
            "reason": format!("{:?}", reason),
        }),
    }
}

pub(super) fn post_turn_action_decision(action: &PostTurnContinuationAction) -> &'static str {
    match action {
        PostTurnContinuationAction::QueueAutoPrompt(_) => "queue_auto_prompt",
        PostTurnContinuationAction::SkipAutoFollow { .. } => "skip_auto_followup",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::conversation_model::AutoFollowSkipReason;
    use crate::adapter::inbound::tui::app::conversation_runtime::PostTurnContinuationAction;
    use crate::application::service::planning::PlanningRuntimeProjection;

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
    fn runtime_projection_log_detail_keeps_runtime_summary_shape_stable() {
        let detail = runtime_projection_log_detail(&PlanningRuntimeProjection::invalid(
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
        let action = PostTurnContinuationAction::SkipAutoFollow {
            reason: AutoFollowSkipReason::PlanningQueueIdlePolicyStop,
        };

        let detail = post_turn_action_log_detail(
            &action,
            &PostTurnEvaluationProvenance::new("turn-1".to_string()),
        );

        assert_eq!(detail["type"], json!("skip_auto_followup"));
        assert_eq!(detail["reason"], json!("PlanningQueueIdlePolicyStop"));
        assert_eq!(post_turn_action_decision(&action), "skip_auto_followup");
    }
}
