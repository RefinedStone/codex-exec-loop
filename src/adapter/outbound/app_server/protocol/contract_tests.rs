use std::collections::BTreeSet;
use std::sync::mpsc::{Receiver, channel};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{
    AccountReadResponse, AppServerNotification, InitializeResponse, ThreadListResponse,
    ThreadReadResponse, TurnNotificationHandling, handle_turn_notification, initialize_detail,
    to_conversation_snapshot, to_session_summary,
};
use crate::application::port::outbound::startup_probe_port::AppServerStartupContext;
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationMessageKind,
    ConversationToolActivity, ConversationToolActivityKind,
};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

const HANDLED_NOTIFICATION_METHODS: &[&str] = &[
    "error",
    "item/agentMessage/delta",
    "item/autoApprovalReview/completed",
    "item/autoApprovalReview/started",
    "item/completed",
    "thread/status/changed",
    "turn/completed",
    "turn/started",
];

const DEFERRED_NOTIFICATION_METHODS: &[&str] = &[
    "item/commandExecution/outputDelta",
    "item/commandExecution/terminalInteraction",
    "item/fileChange/outputDelta",
    "item/mcpToolCall/progress",
    "item/plan/delta",
    "item/reasoning/summaryPartAdded",
    "item/reasoning/summaryTextDelta",
    "item/reasoning/textDelta",
    "item/started",
    "turn/diff/updated",
    "turn/plan/updated",
];

const DIAGNOSTIC_ONLY_NOTIFICATION_METHODS: &[&str] = &[
    "account/login/completed",
    "account/rateLimits/updated",
    "account/updated",
    "app/list/updated",
    "command/exec/outputDelta",
    "configWarning",
    "deprecationNotice",
    "fs/changed",
    "fuzzyFileSearch/sessionCompleted",
    "fuzzyFileSearch/sessionUpdated",
    "hook/completed",
    "hook/started",
    "mcpServer/oauthLogin/completed",
    "mcpServer/startupStatus/updated",
    "model/rerouted",
    "serverRequest/resolved",
    "skills/changed",
    "thread/tokenUsage/updated",
    "windows/worldWritableWarning",
    "windowsSandbox/setupCompleted",
];

const IGNORED_NOTIFICATION_METHODS: &[&str] = &[
    "thread/archived",
    "thread/closed",
    "thread/compacted",
    "thread/name/updated",
    "thread/realtime/closed",
    "thread/realtime/error",
    "thread/realtime/itemAdded",
    "thread/realtime/outputAudio/delta",
    "thread/realtime/started",
    "thread/realtime/transcriptUpdated",
    "thread/started",
    "thread/unarchived",
];

#[test]
fn startup_catalog_and_snapshot_payloads_reduce_to_adapter_contracts() {
    let initialize_response =
        fixture::<InitializeResponse>("fixtures/startup_initialize_response.json");
    let account_response = fixture::<AccountReadResponse>("fixtures/account_read_response.json");
    let startup_context = AppServerStartupContext {
        attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server_launch(),
        initialize_detail: initialize_detail(&initialize_response),
        account_detail: account_response.to_summary_text(),
        account_ok: account_response.is_authenticated(),
        warnings: vec!["config warning from fixture".to_string()],
    };

    assert_eq!(
        startup_context.initialize_detail,
        "linux-x64 / unix / codex-app-server/2.0"
    );
    assert_eq!(
        startup_context.account_detail,
        "chatgpt / operator@example.com / plus"
    );
    assert!(startup_context.account_ok);
    assert_eq!(
        startup_context.attachment_profile,
        TerminalBridgeAttachmentProfile::codex_app_server_launch()
    );

    let list_response = fixture::<ThreadListResponse>("fixtures/thread_list_response.json");
    assert_eq!(
        list_response.next_cursor.as_deref(),
        Some("cursor-after-newer")
    );

    let summaries = list_response
        .data
        .into_iter()
        .map(to_session_summary)
        .collect::<Vec<_>>();

    assert_eq!(
        summaries
            .iter()
            .map(|summary| summary.id.as_str())
            .collect::<Vec<_>>(),
        vec!["thread-newer", "thread-older"]
    );
    assert_eq!(summaries[0].updated_at_epoch, 1_777_910_700);
    assert!(summaries[0].updated_at_epoch > summaries[1].updated_at_epoch);
    assert_eq!(summaries[0].status_type, "running");
    assert_eq!(
        summaries[0].git_branch.as_deref(),
        Some("feature/wire-contract")
    );
    assert_eq!(summaries[0].path, "");
    assert_eq!(
        summaries[1].path,
        "/home/akra/.codex/sessions/thread-older.json"
    );

    let read_response = fixture::<ThreadReadResponse>("fixtures/thread_read_response.json");
    let snapshot = to_conversation_snapshot(
        read_response.thread,
        vec![
            "conversation warning from app-server".to_string(),
            "shared runtime restarted for conversation snapshot request".to_string(),
            "retry opened isolated app-server connection while a turn stream was active"
                .to_string(),
        ],
    );

    assert_eq!(snapshot.thread_id, "thread-snapshot");
    assert_eq!(snapshot.title, "Snapshot fallback title");
    assert_eq!(
        snapshot.warnings,
        vec!["conversation warning from app-server".to_string()]
    );
    assert_eq!(
        snapshot.runtime_notices,
        vec![
            "shared runtime restarted for conversation snapshot request".to_string(),
            "retry opened isolated app-server connection while a turn stream was active"
                .to_string(),
        ]
    );
    assert_eq!(snapshot.messages.len(), 4);
    assert_eq!(snapshot.messages[0].kind, ConversationMessageKind::User);
    assert_eq!(snapshot.messages[0].text, "summarize the current state");
    assert_eq!(snapshot.messages[1].kind, ConversationMessageKind::Agent);
    assert_eq!(
        snapshot.messages[1].text,
        "final answer stored by item/completed"
    );
    assert_eq!(snapshot.messages[1].phase.as_deref(), Some("final_answer"));
    assert_eq!(snapshot.messages[2].kind, ConversationMessageKind::Tool);
    assert_eq!(
        snapshot.messages[2].text,
        "file change: update .codex-exec-loop/planning/result-output.md, update src/main.rs"
    );
    assert_eq!(snapshot.messages[3].kind, ConversationMessageKind::Tool);
    assert_eq!(
        snapshot.messages[3].text,
        "command: cargo test app_server::protocol [completed]"
    );
}

#[test]
fn live_turn_notification_sequence_reduces_to_stream_events() {
    let outcome = reduce_notification_sequence("fixtures/live_turn_notifications.json", true);

    assert!(outcome.warnings.is_empty());
    assert!(outcome.completed);
    assert_eq!(
        outcome.events,
        vec![
            ConversationStreamEvent::TurnStarted {
                turn_id: "turn-live".to_string(),
            },
            ConversationStreamEvent::AgentMessageDelta {
                item_id: "agent-live".to_string(),
                phase: Some("commentary".to_string()),
                delta: "delta text should stay live-only".to_string(),
            },
            ConversationStreamEvent::AgentMessageCompleted {
                item_id: "agent-live".to_string(),
                phase: Some("final_answer".to_string()),
                text: "completed answer from item/completed".to_string(),
            },
            ConversationStreamEvent::ToolActivity {
                activity: ConversationToolActivity {
                    kind: ConversationToolActivityKind::FileChange,
                    text: "file change: update .codex-exec-loop/planning/result-output.md, update /tmp/workspace/.codex-exec-loop/planning/result-output.md, update src/main.rs".to_string(),
                    file_change_count: 3,
                },
            },
            ConversationStreamEvent::ApprovalReviewUpdated {
                review: ConversationApprovalReview {
                    target_item_id: "file-change-live".to_string(),
                    status: ConversationApprovalReviewStatus::InProgress,
                    risk_level: Some("medium".to_string()),
                    rationale: Some("planning file write".to_string()),
                },
            },
            ConversationStreamEvent::ApprovalReviewUpdated {
                review: ConversationApprovalReview {
                    target_item_id: "file-change-live".to_string(),
                    status: ConversationApprovalReviewStatus::Approved,
                    risk_level: Some("medium".to_string()),
                    rationale: Some("allowed by test fixture".to_string()),
                },
            },
            ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-live".to_string(),
                changed_planning_file_paths: vec![RESULT_OUTPUT_FILE_PATH.to_string()],
            },
        ]
    );
}

#[test]
fn stale_and_malformed_notifications_do_not_leak_into_active_stream() {
    let outcome = reduce_notification_sequence("fixtures/drift_recovery_notifications.json", true);

    assert!(outcome.completed);
    assert_eq!(outcome.warnings.len(), 3);
    assert!(
        outcome
            .warnings
            .iter()
            .all(|warning| warning.contains("did not match the active turn stream"))
    );
    assert_eq!(
        outcome.events,
        vec![
            ConversationStreamEvent::AgentMessageCompleted {
                item_id: "agent-after-malformed".to_string(),
                phase: Some("final_answer".to_string()),
                text: "active stream survived malformed item".to_string(),
            },
            ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-live".to_string(),
                changed_planning_file_paths: Vec::new(),
            },
        ]
    );
}

#[test]
fn error_notification_is_the_stream_failure_boundary() {
    let notification = notification_from_value(json!({
        "method": "error",
        "params": {
            "message": "fatal app-server stream error"
        }
    }));
    let (sender, receiver) = channel();
    let mut changed_planning_file_paths = Vec::new();

    let result = handle_turn_notification(
        &notification,
        "thread-live",
        "turn-live",
        &mut changed_planning_file_paths,
        &sender,
    );
    let error = match result {
        Ok(_) => panic!("error notification should terminate the reducer"),
        Err(error) => error,
    };

    assert_eq!(error.to_string(), "fatal app-server stream error");
    assert!(receiver.try_iter().next().is_none());
}

#[test]
fn schema_notification_vocabulary_requires_adapter_classification() {
    let schema_methods = schema_notification_methods();
    let classified_methods = classified_notification_methods();

    let unclassified = schema_methods
        .difference(&classified_methods)
        .cloned()
        .collect::<Vec<_>>();
    let stale_classifications = classified_methods
        .difference(&schema_methods)
        .cloned()
        .collect::<Vec<_>>();

    assert!(
        unclassified.is_empty() && stale_classifications.is_empty(),
        "review app-server notification method vocabulary; classify new schema methods as handled, deferred, diagnostic-only, or ignored. unclassified={unclassified:?}; stale_classifications={stale_classifications:?}"
    );
}

#[test]
fn notification_classification_matches_reducer_ownership() {
    for method in HANDLED_NOTIFICATION_METHODS
        .iter()
        .chain(DEFERRED_NOTIFICATION_METHODS.iter())
    {
        assert!(
            notification_with_method(method).should_defer_to_turn_stream(),
            "`{method}` should stay owned by the active turn reducer"
        );
    }

    for method in DIAGNOSTIC_ONLY_NOTIFICATION_METHODS
        .iter()
        .chain(IGNORED_NOTIFICATION_METHODS.iter())
    {
        assert!(
            !notification_with_method(method).should_defer_to_turn_stream(),
            "`{method}` should stay outside active turn reducer ownership"
        );
    }
}

struct ReducedSequence {
    events: Vec<ConversationStreamEvent>,
    warnings: Vec<String>,
    completed: bool,
}

fn reduce_notification_sequence(path: &str, stop_on_completion: bool) -> ReducedSequence {
    let notifications = notifications_fixture(path);
    let (sender, receiver) = channel();
    let mut changed_planning_file_paths = Vec::new();
    let mut warnings = Vec::new();
    let mut completed = false;

    for notification in notifications {
        let handling = handle_turn_notification(
            &notification,
            "thread-live",
            "turn-live",
            &mut changed_planning_file_paths,
            &sender,
        )
        .expect("fixture notification should reduce without fatal stream error");

        match handling {
            TurnNotificationHandling::Consumed => {}
            TurnNotificationHandling::Completed => {
                completed = true;
                if stop_on_completion {
                    break;
                }
            }
            TurnNotificationHandling::Dropped(warning) => warnings.push(warning),
        }
    }

    ReducedSequence {
        events: collect_events(receiver),
        warnings,
        completed,
    }
}

fn collect_events(receiver: Receiver<ConversationStreamEvent>) -> Vec<ConversationStreamEvent> {
    receiver.try_iter().collect()
}

fn fixture<T>(path: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(fixture_body(path)).expect("fixture JSON should match protocol type")
}

fn notifications_fixture(path: &str) -> Vec<AppServerNotification> {
    serde_json::from_str::<Vec<Value>>(fixture_body(path))
        .expect("notification fixture should be a JSON array")
        .into_iter()
        .map(notification_from_value)
        .collect()
}

fn fixture_body(path: &str) -> &'static str {
    match path {
        "fixtures/account_read_response.json" => {
            include_str!("fixtures/account_read_response.json")
        }
        "fixtures/drift_recovery_notifications.json" => {
            include_str!("fixtures/drift_recovery_notifications.json")
        }
        "fixtures/live_turn_notifications.json" => {
            include_str!("fixtures/live_turn_notifications.json")
        }
        "fixtures/startup_initialize_response.json" => {
            include_str!("fixtures/startup_initialize_response.json")
        }
        "fixtures/thread_list_response.json" => {
            include_str!("fixtures/thread_list_response.json")
        }
        "fixtures/thread_read_response.json" => {
            include_str!("fixtures/thread_read_response.json")
        }
        _ => panic!("unknown protocol fixture path: {path}"),
    }
}

fn notification_from_value(value: Value) -> AppServerNotification {
    AppServerNotification::from_value(value).expect("fixture should be a JSON-RPC notification")
}

fn notification_with_method(method: &str) -> AppServerNotification {
    notification_from_value(json!({
        "method": method,
        "params": {}
    }))
}

fn classified_notification_methods() -> BTreeSet<String> {
    let mut methods = BTreeSet::new();
    let mut duplicates = Vec::new();

    for method in HANDLED_NOTIFICATION_METHODS
        .iter()
        .chain(DEFERRED_NOTIFICATION_METHODS.iter())
        .chain(DIAGNOSTIC_ONLY_NOTIFICATION_METHODS.iter())
        .chain(IGNORED_NOTIFICATION_METHODS.iter())
    {
        if !methods.insert((*method).to_string()) {
            duplicates.push((*method).to_string());
        }
    }

    assert!(
        duplicates.is_empty(),
        "notification method classifications must be disjoint: {duplicates:?}"
    );
    methods
}

fn schema_notification_methods() -> BTreeSet<String> {
    let schema = serde_json::from_str::<Value>(include_str!(
        "../../../../../schema/codex_app_server_protocol.v2.schemas.json"
    ))
    .expect("checked-in app-server protocol schema should parse");

    schema
        .pointer("/definitions/ServerNotification/oneOf")
        .and_then(Value::as_array)
        .expect("schema should expose ServerNotification.oneOf")
        .iter()
        .map(|notification_schema| {
            notification_schema
                .pointer("/properties/method/enum/0")
                .and_then(Value::as_str)
                .expect("notification schema should carry a method enum")
                .to_string()
        })
        .collect()
}
