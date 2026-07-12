use std::collections::BTreeSet;
use std::sync::mpsc::{Receiver, channel};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{
    AccountReadResponse, ActiveTurnNotificationState, AppServerNotification, InitializeResponse,
    ThreadListResponse, ThreadReadResponse, TurnNotificationHandling, handle_turn_notification,
    initialize_detail, to_conversation_snapshot, to_session_summary,
};
use crate::application::port::outbound::startup_probe_port::AppServerStartupContext;
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationMessageKind,
    ConversationToolActivity, ConversationToolActivityKind,
};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
use crate::domain::turn_terminal::{ConversationTurnItemsView, ConversationTurnTerminalOutcome};

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
    "item/fileChange/patchUpdated",
    "item/fileChange/outputDelta",
    "item/mcpToolCall/progress",
    "item/plan/delta",
    "item/reasoning/summaryPartAdded",
    "item/reasoning/summaryTextDelta",
    "item/reasoning/textDelta",
    "item/started",
    "turn/diff/updated",
    "turn/moderationMetadata",
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
    "externalAgentConfig/import/completed",
    "externalAgentConfig/import/progress",
    "fs/changed",
    "fuzzyFileSearch/sessionCompleted",
    "fuzzyFileSearch/sessionUpdated",
    "guardianWarning",
    "hook/completed",
    "hook/started",
    "mcpServer/oauthLogin/completed",
    "mcpServer/startupStatus/updated",
    "model/rerouted",
    "model/safetyBuffering/updated",
    "model/verification",
    "process/exited",
    "process/outputDelta",
    "remoteControl/status/changed",
    "serverRequest/resolved",
    "skills/changed",
    "thread/tokenUsage/updated",
    "windows/worldWritableWarning",
    "windowsSandbox/setupCompleted",
    "warning",
];

const IGNORED_NOTIFICATION_METHODS: &[&str] = &[
    "thread/archived",
    "thread/closed",
    "thread/compacted",
    "thread/deleted",
    "thread/goal/cleared",
    "thread/goal/updated",
    "thread/name/updated",
    "thread/realtime/closed",
    "thread/realtime/error",
    "thread/realtime/itemAdded",
    "thread/realtime/outputAudio/delta",
    "thread/realtime/sdp",
    "thread/realtime/started",
    "thread/realtime/transcript/delta",
    "thread/realtime/transcript/done",
    "thread/settings/updated",
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
        ]
    );
    let receipt = outcome
        .terminal_receipt
        .expect("fixture should preserve its terminal receipt");
    assert_eq!(receipt.turn_id, "turn-live");
    assert!(matches!(
        receipt.outcome,
        ConversationTurnTerminalOutcome::Completed
    ));
    assert_eq!(receipt.items_view, ConversationTurnItemsView::NotLoaded);
    assert_eq!(receipt.started_at, Some(1_783_842_000));
    assert_eq!(receipt.completed_at, Some(1_783_842_002));
    assert_eq!(receipt.duration_ms, Some(2_500));
    assert_eq!(
        receipt.observations.changed_planning_file_paths,
        vec![RESULT_OUTPUT_FILE_PATH.to_string()]
    );
}

#[test]
fn released_authenticated_terminal_capture_preserves_completed_and_interrupted_shapes() {
    let capture: Value = serde_json::from_str(include_str!(
        "../../../../../docs/competitive/upstream-codex/captures/terminal-truth-v0.144.1-linux.json"
    ))
    .expect("released terminal capture should remain valid JSON");

    assert_eq!(
        capture["binary"]["sha256"],
        "b68de8340b2ccb8aeb23b6f33a4b4dff4f203ff84f6d82b6feedf334aba9a4fb"
    );
    assert_eq!(
        capture["binary"]["name"],
        "codex-app-server-x86_64-unknown-linux-musl"
    );
    assert_eq!(capture["binary"]["bytes"], 248_062_016);
    assert_eq!(capture["authenticatedAccountPresent"], true);
    assert_eq!(capture["captureVersion"], 1);
    assert_eq!(capture["platform"], "linux");
    assert_eq!(capture["architecture"], "x64");
    assert_eq!(capture["model"], "gpt-5.3-codex-spark");
    assert_eq!(capture["interruptMode"], "exact-turn-id");
    assert_eq!(
        capture["initializedResultFields"],
        json!(["codexHome", "platformFamily", "platformOs", "userAgent"])
    );
    assert_eq!(capture["errorNotifications"], json!([]));
    assert_eq!(capture["completed"]["status"], "completed");
    assert_eq!(capture["interrupted"]["status"], "interrupted");
    for terminal in ["completed", "interrupted"] {
        assert_eq!(capture[terminal]["method"], "turn/completed");
        assert_eq!(
            capture[terminal]["paramsFields"],
            json!(["threadId", "turn"])
        );
        assert_eq!(capture[terminal]["threadIdPresent"], true);
        assert_eq!(capture[terminal]["turnIdPresent"], true);
        assert_eq!(
            capture[terminal]["turnFields"],
            json!([
                "completedAt",
                "durationMs",
                "error",
                "id",
                "items",
                "itemsView",
                "startedAt",
                "status"
            ])
        );
        assert_eq!(capture[terminal]["itemsView"], "notLoaded");
        assert_eq!(capture[terminal]["itemCount"], 0);
        assert_eq!(capture[terminal]["startedAtType"], "number");
        assert_eq!(capture[terminal]["completedAtType"], "number");
        assert_eq!(capture[terminal]["durationMsType"], "number");
        assert!(capture[terminal]["error"].is_null());
    }

    let encoded = serde_json::to_string(&capture).expect("capture should serialize");
    for forbidden in [
        "\"threadId\":",
        "\"turnId\":",
        "\"id\":",
        "\"account\":",
        "\"auth\":",
        "\"token\":",
        "\"message\":",
        "Bearer ",
        "access_token",
        "refresh_token",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "released capture leaked forbidden raw field or credential marker: {forbidden}"
        );
    }
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
        vec![ConversationStreamEvent::AgentMessageCompleted {
            item_id: "agent-after-malformed".to_string(),
            phase: Some("final_answer".to_string()),
            text: "active stream survived malformed item".to_string(),
        },]
    );
    assert!(outcome.terminal_receipt.is_some());
}

#[test]
fn non_retry_error_is_a_typed_terminal_candidate() {
    let notification = notification_from_value(json!({
        "method": "error",
        "params": {
            "threadId": "thread-live",
            "turnId": "turn-live",
            "willRetry": false,
            "error": {
                "message": "fatal app-server stream error"
            }
        }
    }));
    let (sender, receiver) = channel();
    let mut state = ActiveTurnNotificationState::new();

    let handling = handle_turn_notification(
        &notification,
        "thread-live",
        "turn-live",
        &mut state,
        &sender,
    )
    .expect("typed error candidate should not fail the transport reducer");

    assert!(matches!(
        handling,
        TurnNotificationHandling::NonRetryErrorCandidate { error }
            if error.message == "fatal app-server stream error"
    ));
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
fn schema_fixed_width_integer_bounds_match_rust_ranges() {
    let integer_fields = schema_integer_fields();
    let mut missing_bounds = Vec::new();
    let mut out_of_range = Vec::new();

    for field in &integer_fields {
        let Some((expected_min, expected_max)) = schema_format_bounds(field.format.as_str()) else {
            continue;
        };
        match (field.minimum, field.maximum) {
            (Some(minimum), Some(maximum)) => {
                if minimum < expected_min || maximum > expected_max {
                    out_of_range.push(format!(
                        "{} [{}] => minimum={minimum:?}, maximum={maximum:?}, expected=[{expected_min}, {expected_max}]",
                        field.path, field.format
                    ));
                }
            }
            _ => missing_bounds.push(format!(
                "{} [{}] => minimum={:?}, maximum={:?}",
                field.path, field.format, field.minimum, field.maximum
            )),
        }
    }

    assert!(
        missing_bounds.is_empty() && out_of_range.is_empty(),
        "checked-in schema snapshot must carry explicit Rust integer bounds for every fixed-width integer field. missing={missing_bounds:?}; out_of_range={out_of_range:?}"
    );

    let rollback_turns = integer_fields
        .iter()
        .find(|field| field.path == "$/definitions/ThreadRollbackParams/properties/numTurns")
        .expect("thread rollback field should stay in the schema snapshot");
    assert_eq!(rollback_turns.minimum, Some(1));
}

#[test]
fn schema_snapshot_carries_provenance_and_reviewable_format() {
    let body = include_str!("../../../../../schema/codex_app_server_protocol.v2.schemas.json");
    assert!(
        body.lines().count() > 100,
        "checked-in schema snapshot should stay pretty-printed for reviewable diffs"
    );

    let schema = schema_root();
    assert_eq!(
        schema.pointer("/$id").and_then(Value::as_str),
        Some("urn:codex-exec-loop-native:app-server-protocol:v2:snapshot")
    );
    assert_eq!(schema.get("version").and_then(Value::as_str), Some("v2"));
    assert_eq!(
        schema.get("x-generated-from").and_then(Value::as_str),
        Some("codex app-server generate-json-schema --experimental")
    );
    assert!(
        schema
            .get("x-source-cli-version")
            .and_then(Value::as_str)
            .is_some_and(|version| version.starts_with("codex-cli "))
    );
    assert!(
        schema
            .get("x-generated-content-sha256")
            .and_then(Value::as_str)
            .is_some_and(
                |value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            ),
        "checked-in schema snapshot should identify the pristine generated bundle"
    );
    assert!(
        schema
            .get("description")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty()),
        "checked-in schema snapshot should explain its provenance role"
    );
}

#[test]
fn server_request_schema_snapshot_carries_provenance_and_reviewable_format() {
    let body =
        include_str!("../../../../../schema/codex_app_server_protocol.server_request.schema.json");
    assert!(
        body.lines().count() > 100,
        "checked-in server-request schema should stay pretty-printed"
    );

    let schema = server_request_schema_root();
    assert_eq!(
        schema.pointer("/$id").and_then(Value::as_str),
        Some("urn:codex-exec-loop-native:app-server-protocol:server-request:snapshot")
    );
    assert_eq!(
        schema.get("version").and_then(Value::as_str),
        Some("server-request")
    );
    assert_eq!(
        schema.get("x-generated-artifact").and_then(Value::as_str),
        Some("ServerRequest.json")
    );
    assert!(
        schema
            .get("x-source-cli-version")
            .and_then(Value::as_str)
            .is_some_and(|version| version.starts_with("codex-cli "))
    );
    assert_eq!(
        schema
            .get("x-generated-content-sha256")
            .and_then(Value::as_str)
            .map(str::len),
        Some(64)
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
    terminal_receipt: Option<crate::domain::turn_terminal::ConversationTurnTerminalReceipt>,
}

fn reduce_notification_sequence(path: &str, stop_on_completion: bool) -> ReducedSequence {
    let notifications = notifications_fixture(path);
    let (sender, receiver) = channel();
    let mut state = ActiveTurnNotificationState::new();
    let mut warnings = Vec::new();
    let mut completed = false;
    let mut terminal_receipt = None;

    for notification in notifications {
        let handling = handle_turn_notification(
            &notification,
            "thread-live",
            "turn-live",
            &mut state,
            &sender,
        )
        .expect("fixture notification should reduce without fatal stream error");

        match handling {
            TurnNotificationHandling::Consumed | TurnNotificationHandling::RetryObserved { .. } => {
            }
            TurnNotificationHandling::NonRetryErrorCandidate { .. } => {}
            TurnNotificationHandling::Terminal { receipt }
            | TurnNotificationHandling::DuplicateTerminal { receipt } => {
                completed = true;
                terminal_receipt = Some(receipt);
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
        terminal_receipt,
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
    schema_root()
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

#[derive(Debug)]
struct SchemaIntegerField {
    path: String,
    format: String,
    minimum: Option<i128>,
    maximum: Option<i128>,
}

fn schema_root() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../schema/codex_app_server_protocol.v2.schemas.json"
    ))
    .expect("checked-in app-server protocol schema should parse")
}

fn server_request_schema_root() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../schema/codex_app_server_protocol.server_request.schema.json"
    ))
    .expect("checked-in app-server server-request schema should parse")
}

fn schema_integer_fields() -> Vec<SchemaIntegerField> {
    let mut fields = Vec::new();
    collect_schema_integer_fields(&schema_root(), "$", &mut fields);
    collect_schema_integer_fields(
        &server_request_schema_root(),
        "$server-request",
        &mut fields,
    );
    fields
}

fn collect_schema_integer_fields(node: &Value, path: &str, fields: &mut Vec<SchemaIntegerField>) {
    match node {
        Value::Object(object) => {
            if schema_node_is_integer(object)
                && let Some(format) = object.get("format").and_then(Value::as_str)
            {
                fields.push(SchemaIntegerField {
                    path: path.to_string(),
                    format: format.to_string(),
                    minimum: json_integer(object.get("minimum")),
                    maximum: json_integer(object.get("maximum")),
                });
            }
            for (key, value) in object {
                collect_schema_integer_fields(value, &format!("{path}/{key}"), fields);
            }
        }
        Value::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                collect_schema_integer_fields(value, &format!("{path}/{index}"), fields);
            }
        }
        _ => {}
    }
}

fn json_integer(value: Option<&Value>) -> Option<i128> {
    value.and_then(|value| {
        value
            .as_i64()
            .map(|number| number as i128)
            .or_else(|| value.as_u64().map(|number| number as i128))
    })
}

fn schema_node_is_integer(object: &serde_json::Map<String, Value>) -> bool {
    match object.get("type") {
        Some(Value::String(kind)) => kind == "integer",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind.as_str() == Some("integer")),
        _ => false,
    }
}

fn schema_format_bounds(format: &str) -> Option<(i128, i128)> {
    match format {
        "uint" | "uint64" => Some((0, u64::MAX as i128)),
        "uint32" => Some((0, u32::MAX as i128)),
        "uint16" => Some((0, u16::MAX as i128)),
        "uint8" => Some((0, u8::MAX as i128)),
        "int32" => Some((i32::MIN as i128, i32::MAX as i128)),
        "int64" => Some((i64::MIN as i128, i64::MAX as i128)),
        _ => None,
    }
}
