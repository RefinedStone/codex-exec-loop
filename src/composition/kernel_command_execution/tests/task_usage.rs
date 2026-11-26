use super::{fixture, planning_revision};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::application::port::inbound::kernel_command_port::KernelCommandPort;
use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::domain::conversation_progressive_activity::ConversationProgressiveTokenUsageBreakdown as Tokens;
use crate::domain::kernel_event::{KernelEvent, RecordedKernelEvent};
use crate::domain::kernel_spec::KernelCommandKind;
use serde_json::json;

#[test]
fn task_usage_query_reads_persisted_tokens_without_starting_work_or_changing_task_revision() {
    let (_directory, kernel) = fixture();
    let created = kernel
        .command(
            KernelCommandKind::CreateTask,
            json!({"type":"create_task","task":{"title":"usage fixture"}}),
            None,
        )
        .unwrap();
    let task = created["result"]["task_id"].as_str().unwrap();
    let before = planning_revision(&kernel);
    let empty = kernel
        .query(json!({"type":"task_usage","task_id":task}), false)
        .unwrap();
    assert_eq!(empty["status"], "available");
    assert!(empty["observed_tokens"].is_null());
    let store = SqlitePlanningAuthorityAdapter::new();
    let sequence = store.last_kernel_event_sequence(&kernel.workspace).unwrap();
    let tokens = Tokens {
        input_tokens: 10,
        total_tokens: 10,
        ..Tokens::default()
    };
    let records = vec![
        KernelEvent::TaskExecutionLinked {
            task_id: task.into(),
            thread_id: "thread".into(),
            op_id: 91,
        },
        KernelEvent::TurnStarted {
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            op_id: Some(91),
        },
        KernelEvent::TurnUsageObserved {
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            op_id: Some(91),
            last: tokens,
            total: tokens,
        },
        KernelEvent::TurnUsageObserved {
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            op_id: Some(91),
            last: tokens,
            total: tokens,
        },
    ]
    .into_iter()
    .enumerate()
    .map(|(offset, event)| RecordedKernelEvent {
        sequence: sequence + 1 + offset as i64,
        recorded_at: "2026-11-26T00:00:00Z".into(),
        event,
    })
    .collect::<Vec<_>>();
    store
        .append_kernel_events(&kernel.workspace, &records)
        .unwrap();
    let observed = kernel
        .query(json!({"type":"task_usage","task_id":task}), false)
        .unwrap();
    assert_eq!(observed["status"], "available");
    assert_eq!(observed["observed_tokens"]["total_tokens"], 10);
    assert_eq!(observed["executions"].as_array().unwrap().len(), 1);
    assert_eq!(observed["usage_available"], true);
    let missing = kernel
        .query(json!({"type":"task_usage","task_id":"missing-task"}), false)
        .unwrap();
    assert_eq!(missing["status"], "missing");
    assert!(missing["observed_tokens"].is_null());
    assert_eq!(planning_revision(&kernel), before);
}
