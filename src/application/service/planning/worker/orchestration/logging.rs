use serde_json::{Map, Value, json};

use crate::application::port::outbound::planning_worker_port::PlanningWorkerOperation;
use crate::application::service::planning::runtime::prompt::PlanningRuntimeSnapshot;

pub(super) fn operation_label(operation: PlanningWorkerOperation) -> &'static str {
    match operation {
        PlanningWorkerOperation::RefreshQueue => "refresh",
        PlanningWorkerOperation::RepairTaskAuthority => "repair",
    }
}

pub(super) fn orchestration_event_detail<I>(
    workspace_directory: &str,
    orchestration_id: &str,
    operation: PlanningWorkerOperation,
    phase: &str,
    decision: Option<&str>,
    runtime: Option<&PlanningRuntimeSnapshot>,
    fields: I,
) -> Value
where
    I: IntoIterator<Item = (&'static str, Value)>,
{
    let mut detail = Map::new();
    detail.insert(
        "workspace_directory".to_string(),
        json!(workspace_directory),
    );
    detail.insert("orchestration_id".to_string(), json!(orchestration_id));
    detail.insert("operation".to_string(), json!(operation_label(operation)));
    detail.insert("phase".to_string(), json!(phase));
    detail.insert("decision".to_string(), json!(decision));
    if let Some(runtime) = runtime {
        detail.insert("runtime".to_string(), runtime_snapshot_log_detail(runtime));
    }
    for (key, value) in fields {
        detail.insert(key.to_string(), value);
    }
    Value::Object(detail)
}

fn runtime_snapshot_log_detail(snapshot: &PlanningRuntimeSnapshot) -> Value {
    json!({
        "workspace_present": snapshot.workspace_present(),
        "workspace_status": format!("{:?}", snapshot.workspace_status()),
        "queue_idle_policy": format!("{:?}", snapshot.queue_idle_policy()),
        "queue_summary": snapshot.queue_summary(),
        "proposal_summary": snapshot.proposal_summary(),
        "failure_reason": snapshot.failure_reason(),
        "pause_reason": snapshot.auto_followup_pause_reason(),
        "has_actionable_queue_head": snapshot.has_actionable_queue_head(),
        "has_proposal_candidates": snapshot.has_proposal_candidates(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orchestration_event_detail_keeps_core_planning_fields_stable() {
        let detail = orchestration_event_detail(
            "/tmp/workspace",
            "planning-worker-refresh-turn-1",
            PlanningWorkerOperation::RefreshQueue,
            "run_planning_session",
            Some("abort"),
            None,
            [("error", json!("worker failed"))],
        );

        assert_eq!(detail["workspace_directory"], json!("/tmp/workspace"));
        assert_eq!(detail["orchestration_id"], json!("planning-worker-refresh-turn-1"));
        assert_eq!(detail["operation"], json!("refresh"));
        assert_eq!(detail["phase"], json!("run_planning_session"));
        assert_eq!(detail["decision"], json!("abort"));
        assert_eq!(detail["error"], json!("worker failed"));
    }

    #[test]
    fn orchestration_event_detail_embeds_runtime_under_standard_key() {
        let runtime = PlanningRuntimeSnapshot::invalid("planning invalid");
        let detail = orchestration_event_detail(
            "/tmp/workspace",
            "planning-worker-repair-turn-1-1",
            PlanningWorkerOperation::RepairTaskAuthority,
            "completed",
            Some("return_outcome"),
            Some(&runtime),
            [],
        );

        assert_eq!(detail["runtime"]["workspace_status"], json!("Invalid"));
        assert_eq!(
            detail["runtime"]["failure_reason"],
            json!("planning invalid")
        );
    }
}
