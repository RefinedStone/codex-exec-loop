use std::collections::BTreeSet;
use std::sync::mpsc::{Receiver, channel};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{
    AccountReadResponse, ActiveTurnNotificationState, AppServerNotification,
    ITEM_PROJECTION_MANIFEST, InitializeResponse, ItemProjectionManifestRow,
    MAX_RETAINED_ITEM_EFFECT_IDENTITIES, SUBAGENT_ACTIVITY_OUTCOME_KINDS, ThreadListResponse,
    ThreadReadResponse, TurnNotificationHandling, UNKNOWN_ITEM_PROJECTION_DECISION,
    handle_turn_notification, initialize_detail, to_conversation_snapshot, to_session_summary,
};
use crate::application::port::outbound::startup_probe_port::AppServerStartupContext;
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationMessageKind,
    ConversationToolActivity, ConversationToolActivityKind,
};
use crate::domain::conversation_item_lifecycle::{
    ConversationItemKind, ConversationItemLifecycleConsistency,
    ConversationItemLifecycleObservation, ConversationItemLifecyclePhase,
    ConversationItemLifecycleSource, ConversationItemOutcome,
    MAX_CONVERSATION_ITEM_IDENTIFIER_BYTES, MAX_CONVERSATION_ITEM_SUMMARY_BYTES,
    MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS,
};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
use crate::domain::turn_terminal::{ConversationTurnItemsView, ConversationTurnTerminalOutcome};

const HANDLED_NOTIFICATION_METHODS: &[&str] = &[
    "error",
    "item/agentMessage/delta",
    "item/autoApprovalReview/completed",
    "item/autoApprovalReview/started",
    "item/completed",
    "item/commandExecution/outputDelta",
    "item/commandExecution/terminalInteraction",
    "item/fileChange/patchUpdated",
    "item/mcpToolCall/progress",
    "item/plan/delta",
    "item/reasoning/summaryPartAdded",
    "item/reasoning/summaryTextDelta",
    "item/reasoning/textDelta",
    "item/started",
    "guardianWarning",
    "model/rerouted",
    "thread/settings/updated",
    "thread/status/changed",
    "thread/tokenUsage/updated",
    "turn/completed",
    "turn/diff/updated",
    "turn/moderationMetadata",
    "turn/plan/updated",
    "turn/started",
];

const DEFERRED_NOTIFICATION_METHODS: &[&str] = &[
    "item/fileChange/outputDelta",
    "model/safetyBuffering/updated",
    "model/verification",
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
    "hook/completed",
    "hook/started",
    "mcpServer/oauthLogin/completed",
    "mcpServer/startupStatus/updated",
    "process/exited",
    "process/outputDelta",
    "remoteControl/status/changed",
    "serverRequest/resolved",
    "skills/changed",
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
    assert_eq!(snapshot.messages[2].display_label.as_deref(), Some("patch"));
    assert_eq!(
        snapshot.messages[2].text,
        "file change: update .codex-exec-loop/planning/result-output.md, update src/main.rs"
    );
    assert_eq!(snapshot.messages[3].kind, ConversationMessageKind::Tool);
    assert_eq!(
        snapshot.messages[3].display_label.as_deref(),
        Some("command")
    );
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
    let lifecycle = outcome
        .events
        .iter()
        .filter_map(|event| match event {
            ConversationStreamEvent::ItemLifecycleObserved { observation } => {
                Some((observation.item_id.as_str(), observation.phase))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        lifecycle,
        vec![
            ("agent-live", ConversationItemLifecyclePhase::Started),
            ("agent-live", ConversationItemLifecyclePhase::Completed),
            (
                "file-change-live",
                ConversationItemLifecyclePhase::Completed
            ),
        ]
    );
    let mut visible_events = outcome
        .events
        .iter()
        .filter(|event| !matches!(event, ConversationStreamEvent::ItemLifecycleObserved { .. }))
        .cloned()
        .collect::<Vec<_>>();
    let progressive = visible_events.remove(0);
    let ConversationStreamEvent::ProgressiveActivityObserved { batch } = progressive else {
        panic!("agent delta should use the bounded progressive activity rail")
    };
    assert_eq!(batch.records().len(), 1);
    assert!(matches!(
        &batch.records()[0].observation().payload,
        crate::domain::conversation_progressive_activity::ConversationProgressiveActivityPayload::AgentMessageDelta {
            phase: Some(phase),
            text,
            ..
        } if phase == "commentary" && text == "delta text should stay live-only"
    ));
    assert_eq!(
        visible_events,
        vec![
            ConversationStreamEvent::AgentMessageCompleted {
                item_id: "agent-live".to_string(),
                phase: Some("final_answer".to_string()),
                text: "completed answer from item/completed".to_string(),
            },
            ConversationStreamEvent::ToolActivity {
                activity: ConversationToolActivity {
                    kind: ConversationToolActivityKind::FileChange,
                    text: "file change: update .codex-exec-loop/planning/result-output.md, update /tmp/workspace/.codex-exec-loop/planning/result-output.md, update src/main.rs".to_string(),
                    display_label: Some("patch".to_string()),
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
    assert_eq!(outcome.warnings.len(), 4);
    assert!(
        outcome
            .warnings
            .iter()
            .filter(|warning| warning.contains("did not match the active turn stream"))
            .count()
            == 3
    );
    assert!(
        outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("item lifecycle field"))
    );
    let visible_events = outcome
        .events
        .iter()
        .filter(|event| !matches!(event, ConversationStreamEvent::ItemLifecycleObserved { .. }))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        visible_events,
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
fn thread_item_schema_requires_an_explicit_field_and_outcome_projection_manifest() {
    let schema = schema_root();
    let item_schemas = schema
        .pointer("/definitions/ThreadItem/oneOf")
        .and_then(Value::as_array)
        .expect("schema should expose ThreadItem.oneOf");
    let schema_types = item_schemas
        .iter()
        .map(thread_item_wire_type)
        .collect::<BTreeSet<_>>();
    let manifest_types = ITEM_PROJECTION_MANIFEST
        .iter()
        .map(|row| row.wire_type.to_string())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        manifest_types, schema_types,
        "new or removed ThreadItem variants require an explicit projection decision"
    );
    assert_eq!(
        ITEM_PROJECTION_MANIFEST.len(),
        manifest_types.len(),
        "item projection manifest wire types must be unique"
    );

    for row in ITEM_PROJECTION_MANIFEST {
        let item_schema = item_schemas
            .iter()
            .find(|item_schema| thread_item_wire_type(item_schema) == row.wire_type)
            .expect("manifest row should match a ThreadItem schema");
        assert_item_fields_are_explicitly_classified(row, item_schema);
        assert_item_required_fields_match_schema(row, item_schema);
        assert_item_outcome_statuses_match_schema(row, item_schema, &schema);
    }

    assert!(
        ITEM_PROJECTION_MANIFEST
            .iter()
            .any(|row| !row.bounded_redacted_fields.is_empty())
    );
    assert!(
        ITEM_PROJECTION_MANIFEST
            .iter()
            .any(|row| !row.ignored_fields.is_empty())
    );
    assert_eq!(
        UNKNOWN_ITEM_PROJECTION_DECISION.preserved_fields,
        &["id", "type"]
    );
    let subagent_schema = item_schemas
        .iter()
        .find(|item_schema| thread_item_wire_type(item_schema) == "subAgentActivity")
        .unwrap();
    assert_eq!(
        resolved_schema_enum(
            subagent_schema.pointer("/properties/kind").unwrap(),
            &schema,
        )
        .unwrap(),
        SUBAGENT_ACTIVITY_OUTCOME_KINDS
            .iter()
            .map(|kind| (*kind).to_string())
            .collect::<BTreeSet<_>>()
    );
}

#[test]
fn item_lifecycle_notification_schema_requires_exact_identity_item_and_timestamp_shape() {
    let schema = schema_root();
    for (method, definition, timestamp_field) in [
        ("item/started", "ItemStartedNotification", "startedAtMs"),
        (
            "item/completed",
            "ItemCompletedNotification",
            "completedAtMs",
        ),
    ] {
        let notification = schema
            .pointer("/definitions/ServerNotification/oneOf")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|notification| {
                notification
                    .pointer("/properties/method/enum/0")
                    .and_then(Value::as_str)
                    == Some(method)
            })
            .expect("lifecycle method should remain in ServerNotification");
        assert_eq!(
            notification
                .pointer("/properties/params/$ref")
                .and_then(Value::as_str),
            Some(format!("#/definitions/{definition}").as_str())
        );
        assert_eq!(
            json_string_set(notification.get("required")),
            BTreeSet::from(["method".to_string(), "params".to_string()])
        );

        let payload = schema
            .pointer(format!("/definitions/{definition}").as_str())
            .expect("lifecycle payload definition should remain present");
        assert_eq!(
            json_string_set(payload.get("required")),
            BTreeSet::from([
                "item".to_string(),
                "threadId".to_string(),
                "turnId".to_string(),
                timestamp_field.to_string(),
            ])
        );
        assert_eq!(
            payload
                .pointer("/properties/item/$ref")
                .and_then(Value::as_str),
            Some("#/definitions/ThreadItem")
        );
        for identity_field in ["threadId", "turnId"] {
            assert_eq!(
                payload
                    .pointer(format!("/properties/{identity_field}/type").as_str())
                    .and_then(Value::as_str),
                Some("string")
            );
        }
        let timestamp = payload
            .pointer(format!("/properties/{timestamp_field}").as_str())
            .expect("lifecycle timestamp should remain present");
        assert_eq!(
            timestamp.get("type").and_then(Value::as_str),
            Some("integer")
        );
        assert_eq!(
            timestamp.get("format").and_then(Value::as_str),
            Some("int64")
        );
        assert_eq!(
            timestamp.get("minimum").and_then(Value::as_i64),
            Some(i64::MIN)
        );
        assert_eq!(
            timestamp.get("maximum").and_then(Value::as_i64),
            Some(i64::MAX)
        );
    }
}

#[test]
fn all_stable_item_kinds_replay_to_bounded_redacted_snapshot_identity() {
    let response =
        fixture::<ThreadReadResponse>("fixtures/item_lifecycle_thread_read_response.json");

    let snapshot = to_conversation_snapshot(response.thread, Vec::new());
    let lifecycle = snapshot.item_lifecycle;

    assert_eq!(lifecycle.records.len(), 18);
    assert!(lifecycle.is_complete());
    assert_eq!(
        lifecycle
            .records
            .iter()
            .filter_map(|record| record.observation.kind.stable_wire_label())
            .collect::<Vec<_>>(),
        ITEM_PROJECTION_MANIFEST
            .iter()
            .map(|row| row.wire_type)
            .collect::<Vec<_>>()
    );
    assert!(lifecycle.records.iter().all(|record| {
        record.observation.phase == ConversationItemLifecyclePhase::SnapshotObserved
            && record.observation.source == ConversationItemLifecycleSource::Snapshot
            && record.observation.thread_id == "thread-item-lifecycle"
            && record.observation.turn_id == "turn-item-lifecycle"
            && record.observation.summary.len() <= MAX_CONVERSATION_ITEM_SUMMARY_BYTES
    }));
    assert!(!format!("{lifecycle:?}").contains("AKRA_SECRET"));
    assert_eq!(
        lifecycle.records[8].observation.outcome,
        ConversationItemOutcome::Unknown("completed-with-success-false".to_string())
    );
    assert_eq!(
        lifecycle.records[15].observation.kind,
        ConversationItemKind::EnteredReviewMode
    );
    assert!(lifecycle.records.iter().any(|record| {
        record.observation.item_id == "item-file"
            && record.observation.outcome == ConversationItemOutcome::Declined
    }));
    assert!(
        snapshot
            .messages
            .iter()
            .all(|message| message.item_id.as_deref() != Some("item-file")),
        "declined snapshot file changes must remain lifecycle facts, not tool transcript entries"
    );
}

#[test]
fn in_progress_snapshot_item_remains_observed_without_inventing_a_completion_boundary() {
    let mut fixture = fixture_value("fixtures/item_lifecycle_thread_read_response.json");
    *fixture
        .pointer_mut("/thread/turns/0/status")
        .expect("fixture should contain turn status") = json!("inProgress");
    *fixture
        .pointer_mut("/thread/turns/0/items/5/status")
        .expect("fixture should contain command status") = json!("inProgress");
    let response: ThreadReadResponse =
        serde_json::from_value(fixture).expect("in-progress thread snapshot should deserialize");

    let snapshot = to_conversation_snapshot(response.thread, Vec::new());
    let command = snapshot
        .item_lifecycle
        .records
        .iter()
        .find(|record| record.observation.item_id == "item-command")
        .expect("command identity should remain in the snapshot projection");

    assert_eq!(
        command.observation.phase,
        ConversationItemLifecyclePhase::SnapshotObserved
    );
    assert_eq!(
        command.observation.outcome,
        ConversationItemOutcome::InProgress
    );
    assert_eq!(
        command.consistency,
        ConversationItemLifecycleConsistency::SnapshotObserved
    );
}

#[test]
fn oversized_snapshot_thread_identity_is_rejected_before_display_bounding() {
    let mut fixture = fixture_value("fixtures/item_lifecycle_thread_read_response.json");
    let oversized_thread_id = "t".repeat(MAX_CONVERSATION_ITEM_IDENTIFIER_BYTES + 1);
    *fixture
        .pointer_mut("/thread/id")
        .expect("fixture should contain thread identity") = json!(oversized_thread_id.clone());
    let response: ThreadReadResponse =
        serde_json::from_value(fixture).expect("oversized source identity is still valid JSON");

    let snapshot = to_conversation_snapshot(response.thread, Vec::new());

    assert_ne!(snapshot.thread_id, oversized_thread_id);
    assert!(snapshot.item_lifecycle.records.is_empty());
    assert_eq!(snapshot.item_lifecycle.invalid_record_count, 18);
    assert!(!snapshot.item_lifecycle.is_complete());
}

#[test]
fn all_stable_item_kinds_emit_ordered_live_start_and_completion_identity() {
    let fixture = fixture_value("fixtures/item_lifecycle_thread_read_response.json");
    let expected_response: ThreadReadResponse =
        serde_json::from_value(fixture.clone()).expect("lifecycle fixture should deserialize");
    let expected_lifecycle =
        to_conversation_snapshot(expected_response.thread, Vec::new()).item_lifecycle;
    let items = fixture
        .pointer("/thread/turns/0/items")
        .and_then(Value::as_array)
        .expect("item lifecycle fixture should contain turn items");
    let (sender, receiver) = channel();
    let mut state = ActiveTurnNotificationState::new();

    for (index, item) in items.iter().enumerate() {
        let item_id = item.get("id").and_then(Value::as_str).unwrap();
        let started_item = started_item_fixture(item);
        let started = notification_from_value(json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-item-lifecycle",
                "turnId": "turn-item-lifecycle",
                "startedAtMs": (index as i64) * 2,
                "item": started_item
            }
        }));
        assert_eq!(
            handle_turn_notification(
                &started,
                "thread-item-lifecycle",
                "turn-item-lifecycle",
                &mut state,
                &sender,
            )
            .unwrap(),
            TurnNotificationHandling::Consumed
        );
        let started_observation = assert_item_lifecycle_event(
            receiver.recv().unwrap(),
            item_id,
            ConversationItemLifecyclePhase::Started,
        );
        if item_id == "item-agent" {
            assert!(
                started_observation
                    .summary
                    .contains("agent message bytes=0")
            );
        }
        assert_eq!(started_observation.observed_at_ms, Some((index as i64) * 2));

        let completed = notification_from_value(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-item-lifecycle",
                "turnId": "turn-item-lifecycle",
                "completedAtMs": (index as i64) * 2 + 1,
                "item": item.clone()
            }
        }));
        assert_eq!(
            handle_turn_notification(
                &completed,
                "thread-item-lifecycle",
                "turn-item-lifecycle",
                &mut state,
                &sender,
            )
            .unwrap(),
            TurnNotificationHandling::Consumed
        );
        let completed_observation = assert_item_lifecycle_event(
            receiver.recv().unwrap(),
            item_id,
            ConversationItemLifecyclePhase::Completed,
        );
        let expected = &expected_lifecycle.records[index].observation;
        assert_eq!(completed_observation.kind, expected.kind);
        assert_eq!(completed_observation.outcome, expected.outcome);
        assert_eq!(completed_observation.summary, expected.summary);
        assert_eq!(completed_observation.thread_id, expected.thread_id);
        assert_eq!(completed_observation.turn_id, expected.turn_id);
        assert_eq!(
            completed_observation.observed_at_ms,
            Some((index as i64) * 2 + 1)
        );
        while receiver.try_recv().is_ok() {}
    }
}

#[test]
fn invalid_started_or_mismatched_item_identity_is_dropped_without_application_event() {
    let cases = [
        json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "item": { "type": "contextCompaction" }
            }
        }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-other",
                "turnId": "turn-1",
                "completedAtMs": 2,
                "item": { "id": "item-1", "type": "contextCompaction" }
            }
        }),
        json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "item": { "id": "item-1", "type": "contextCompaction" }
            }
        }),
        json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": "1",
                "item": { "id": "item-1", "type": "contextCompaction" }
            }
        }),
        json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-other",
                "startedAtMs": 1,
                "item": { "id": "item-1", "type": "contextCompaction" }
            }
        }),
        json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "item": { "id": "item-1", "type": "" }
            }
        }),
        json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "item": { "id": "", "type": "contextCompaction" }
            }
        }),
        json!({
            "method": "item/started",
            "params": {
                "turnId": "turn-1",
                "startedAtMs": 1,
                "item": { "id": "item-1", "type": "contextCompaction" }
            }
        }),
        json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-1",
                "startedAtMs": 1,
                "item": { "id": "item-1", "type": "contextCompaction" }
            }
        }),
        json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "item": { "id": "item-1" }
            }
        }),
    ];

    for value in cases {
        let (sender, receiver) = channel();
        let handling = handle_turn_notification(
            &notification_from_value(value),
            "thread-1",
            "turn-1",
            &mut ActiveTurnNotificationState::new(),
            &sender,
        )
        .unwrap();

        assert!(matches!(handling, TurnNotificationHandling::Dropped(_)));
        assert!(receiver.try_recv().is_err());
    }

    for (thread_id, turn_id) in [
        (String::new(), "turn-1".to_string()),
        ("thread-1".to_string(), String::new()),
        (
            "t".repeat(MAX_CONVERSATION_ITEM_IDENTIFIER_BYTES + 1),
            "turn-1".to_string(),
        ),
        (
            "thread-1".to_string(),
            "t".repeat(MAX_CONVERSATION_ITEM_IDENTIFIER_BYTES + 1),
        ),
    ] {
        let (sender, receiver) = channel();
        let notification = notification_from_value(json!({
            "method": "item/started",
            "params": {
                "threadId": thread_id.clone(),
                "turnId": turn_id.clone(),
                "startedAtMs": 1,
                "item": { "id": "item-1", "type": "contextCompaction" }
            }
        }));
        let handling = handle_turn_notification(
            &notification,
            &thread_id,
            &turn_id,
            &mut ActiveTurnNotificationState::new(),
            &sender,
        )
        .unwrap();
        assert!(matches!(handling, TurnNotificationHandling::Dropped(_)));
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn malformed_active_item_completion_fails_closed_without_application_event() {
    let cases = [
        json!({
            "method": "item/completed",
            "params": {
                "turnId": "turn-1",
                "completedAtMs": 1,
                "item": {
                    "id": "agent-missing-thread",
                    "type": "agentMessage",
                    "text": "must not be silently lost"
                }
            }
        }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "completedAtMs": 1,
                "item": {
                    "id": "agent-missing-turn",
                    "type": "agentMessage",
                    "text": "must not be silently lost"
                }
            }
        }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": 7,
                "turnId": "turn-1",
                "completedAtMs": 1,
                "item": {
                    "id": "agent-invalid-thread",
                    "type": "agentMessage",
                    "text": "must not be silently lost"
                }
            }
        }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "",
                "completedAtMs": 1,
                "item": {
                    "id": "agent-empty-turn",
                    "type": "agentMessage",
                    "text": "must not be silently lost"
                }
            }
        }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 1,
                "item": {
                    "id": "x".repeat(MAX_CONVERSATION_ITEM_IDENTIFIER_BYTES + 1),
                    "type": "agentMessage",
                    "text": "must not be silently lost"
                }
            }
        }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "item": {
                    "id": "agent-missing-timestamp",
                    "type": "agentMessage",
                    "text": "must not be silently lost"
                }
            }
        }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 1,
                "item": {
                    "id": "agent-missing-text",
                    "type": "agentMessage"
                }
            }
        }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 1,
                "item": {
                    "type": "contextCompaction"
                }
            }
        }),
    ];

    for value in cases {
        let (sender, receiver) = channel();
        let error = handle_turn_notification(
            &notification_from_value(value),
            "thread-1",
            "turn-1",
            &mut ActiveTurnNotificationState::new(),
            &sender,
        )
        .expect_err("malformed active completion must fail the stream");

        assert!(error.to_string().contains("active item/completed"));
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn duplicate_item_boundaries_remain_visible_without_repeating_tool_side_effects() {
    let item = json!({
        "id": "command-1",
        "type": "commandExecution",
        "command": "cargo test",
        "commandActions": [],
        "cwd": "/repo",
        "status": "completed"
    });
    let (sender, receiver) = channel();
    let mut state = ActiveTurnNotificationState::new();

    for timestamp in [1, 2] {
        let notification = notification_from_value(json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": timestamp,
                "item": item
            }
        }));
        handle_turn_notification(&notification, "thread-1", "turn-1", &mut state, &sender).unwrap();
    }
    for timestamp in [3, 4] {
        let notification = notification_from_value(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": timestamp,
                "item": item
            }
        }));
        handle_turn_notification(&notification, "thread-1", "turn-1", &mut state, &sender).unwrap();
    }

    let events = receiver.try_iter().collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ConversationStreamEvent::ItemLifecycleObserved { .. }))
            .count(),
        4
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ConversationStreamEvent::ToolActivity { .. }))
            .count(),
        1
    );
}

#[test]
fn first_timestamp_regressed_completion_preserves_authoritative_agent_text() {
    let item = json!({
        "id": "agent-clock-regression",
        "type": "agentMessage",
        "text": "authoritative final answer"
    });
    let (sender, receiver) = channel();
    let mut state = ActiveTurnNotificationState::new();

    handle_turn_notification(
        &notification_from_value(json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 10,
                "item": item.clone()
            }
        })),
        "thread-1",
        "turn-1",
        &mut state,
        &sender,
    )
    .unwrap();
    handle_turn_notification(
        &notification_from_value(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 9,
                "item": item
            }
        })),
        "thread-1",
        "turn-1",
        &mut state,
        &sender,
    )
    .unwrap();

    let events = receiver.try_iter().collect::<Vec<_>>();
    assert!(matches!(
        events.as_slice(),
        [
            ConversationStreamEvent::ItemLifecycleObserved { .. },
            ConversationStreamEvent::ItemLifecycleObserved { .. },
            ConversationStreamEvent::AgentMessageCompleted { text, .. }
        ] if text == "authoritative final answer"
    ));
}

#[test]
fn truncated_identity_window_suppresses_completion_only_replay_side_effects() {
    let command = json!({
        "id": "command-completion-only",
        "type": "commandExecution",
        "command": "cargo test",
        "commandActions": [],
        "cwd": "/repo",
        "status": "completed"
    });
    let (sender, receiver) = channel();
    let mut state = ActiveTurnNotificationState::new();
    let completed = |timestamp: i64, item: Value| {
        notification_from_value(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": timestamp,
                "item": item
            }
        }))
    };

    handle_turn_notification(
        &completed(1, command.clone()),
        "thread-1",
        "turn-1",
        &mut state,
        &sender,
    )
    .unwrap();
    for index in 0..MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS {
        handle_turn_notification(
            &completed(
                index as i64 + 2,
                json!({
                    "id": format!("completion-only-filler-{index}"),
                    "type": "contextCompaction"
                }),
            ),
            "thread-1",
            "turn-1",
            &mut state,
            &sender,
        )
        .unwrap();
    }
    handle_turn_notification(
        &completed(10_000, command),
        "thread-1",
        "turn-1",
        &mut state,
        &sender,
    )
    .unwrap();

    let events = receiver.try_iter().collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ConversationStreamEvent::ToolActivity { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                ConversationStreamEvent::ItemLifecycleObserved { observation }
                    if observation.item_id == "command-completion-only"
            ))
            .count(),
        2
    );
}

#[test]
fn truncated_identity_window_suppresses_replayed_boundary_pair_side_effects() {
    let command = json!({
        "id": "command-old",
        "type": "commandExecution",
        "command": "cargo test",
        "commandActions": [],
        "cwd": "/repo",
        "status": "completed"
    });
    let (sender, receiver) = channel();
    let mut state = ActiveTurnNotificationState::new();
    let completed = |timestamp: i64, item: Value| {
        notification_from_value(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": timestamp,
                "item": item
            }
        }))
    };

    handle_turn_notification(
        &completed(1, command.clone()),
        "thread-1",
        "turn-1",
        &mut state,
        &sender,
    )
    .unwrap();
    for index in 0..MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS {
        handle_turn_notification(
            &completed(
                index as i64 + 2,
                json!({
                    "id": format!("compaction-{index}"),
                    "type": "contextCompaction"
                }),
            ),
            "thread-1",
            "turn-1",
            &mut state,
            &sender,
        )
        .unwrap();
    }
    let replayed_start = notification_from_value(json!({
        "method": "item/started",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "startedAtMs": 9_999,
            "item": command.clone()
        }
    }));
    handle_turn_notification(&replayed_start, "thread-1", "turn-1", &mut state, &sender).unwrap();
    handle_turn_notification(
        &completed(10_000, command),
        "thread-1",
        "turn-1",
        &mut state,
        &sender,
    )
    .unwrap();
    handle_turn_notification(
        &completed(
            10_001,
            json!({
                "id": "command-new",
                "type": "commandExecution",
                "command": "cargo check",
                "commandActions": [],
                "cwd": "/repo",
                "status": "completed"
            }),
        ),
        "thread-1",
        "turn-1",
        &mut state,
        &sender,
    )
    .unwrap();

    let events = receiver.try_iter().collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ConversationStreamEvent::ToolActivity { .. }))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                ConversationStreamEvent::ItemLifecycleObserved { observation }
                    if observation.item_id == "command-old"
            ))
            .count(),
        3
    );
}

#[test]
fn item_identity_capacity_fails_stream_before_losing_final_agent_text() {
    let (sender, receiver) = channel();
    let mut state = ActiveTurnNotificationState::new();
    let completed = |index: usize| {
        notification_from_value(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": index as i64,
                "item": {
                    "id": format!("command-{index}"),
                    "type": "commandExecution",
                    "command": "true",
                    "commandActions": [],
                    "cwd": "/repo",
                    "status": "completed"
                }
            }
        }))
    };

    for index in 0..MAX_RETAINED_ITEM_EFFECT_IDENTITIES {
        handle_turn_notification(&completed(index), "thread-1", "turn-1", &mut state, &sender)
            .unwrap();
    }
    let final_agent = notification_from_value(json!({
        "method": "item/completed",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "completedAtMs": MAX_RETAINED_ITEM_EFFECT_IDENTITIES as i64,
            "item": {
                "id": "agent-final-after-capacity",
                "type": "agentMessage",
                "text": "authoritative final answer"
            }
        }
    }));
    let error = handle_turn_notification(&final_agent, "thread-1", "turn-1", &mut state, &sender)
        .unwrap_err();

    let events = receiver.try_iter().collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ConversationStreamEvent::ItemLifecycleObserved { .. }))
            .count(),
        MAX_RETAINED_ITEM_EFFECT_IDENTITIES + 1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ConversationStreamEvent::ToolActivity { .. }))
            .count(),
        MAX_RETAINED_ITEM_EFFECT_IDENTITIES
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, ConversationStreamEvent::AgentMessageCompleted { .. }))
    );
    assert!(error.to_string().contains("item identity ledger exhausted"));
}

#[test]
fn declined_file_change_cannot_reappear_as_applied_after_projection_eviction() {
    let (sender, receiver) = channel();
    let mut state = ActiveTurnNotificationState::new();
    let file_change = |status: &str, timestamp: i64| {
        notification_from_value(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": timestamp,
                "item": {
                    "id": "file-replayed",
                    "type": "fileChange",
                    "changes": [{
                        "path": RESULT_OUTPUT_FILE_PATH,
                        "kind": { "type": "update" }
                    }],
                    "status": status
                }
            }
        }))
    };

    handle_turn_notification(
        &file_change("declined", 1),
        "thread-1",
        "turn-1",
        &mut state,
        &sender,
    )
    .unwrap();
    for index in 0..MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS {
        let compaction = notification_from_value(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": index as i64 + 2,
                "item": {
                    "id": format!("compaction-declined-{index}"),
                    "type": "contextCompaction"
                }
            }
        }));
        handle_turn_notification(&compaction, "thread-1", "turn-1", &mut state, &sender).unwrap();
    }
    handle_turn_notification(
        &file_change("completed", 10_000),
        "thread-1",
        "turn-1",
        &mut state,
        &sender,
    )
    .unwrap();

    assert!(state.changed_planning_file_paths().is_empty());
    assert_eq!(
        receiver
            .try_iter()
            .filter(|event| matches!(event, ConversationStreamEvent::ToolActivity { .. }))
            .count(),
        0
    );
}

#[test]
fn non_effect_item_kind_drift_fails_closed_after_projection_eviction() {
    let (sender, receiver) = channel();
    let mut state = ActiveTurnNotificationState::new();
    let completed = |timestamp: i64, item: Value| {
        notification_from_value(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": timestamp,
                "item": item
            }
        }))
    };
    handle_turn_notification(
        &completed(
            1,
            json!({
                "id": "kind-drift",
                "type": "contextCompaction"
            }),
        ),
        "thread-1",
        "turn-1",
        &mut state,
        &sender,
    )
    .unwrap();
    for index in 0..MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS {
        handle_turn_notification(
            &completed(
                index as i64 + 2,
                json!({
                    "id": format!("kind-drift-filler-{index}"),
                    "type": "contextCompaction"
                }),
            ),
            "thread-1",
            "turn-1",
            &mut state,
            &sender,
        )
        .unwrap();
    }
    let error = handle_turn_notification(
        &completed(
            10_000,
            json!({
                "id": "kind-drift",
                "type": "commandExecution",
                "command": "cargo test",
                "commandActions": [],
                "cwd": "/repo",
                "status": "completed"
            }),
        ),
        "thread-1",
        "turn-1",
        &mut state,
        &sender,
    )
    .expect_err("kind drift must fail before a later terminal can be confirmed");

    assert!(error.to_string().contains("changed kind"));
    assert_eq!(
        receiver
            .try_iter()
            .filter(|event| matches!(event, ConversationStreamEvent::ToolActivity { .. }))
            .count(),
        0
    );
}

#[test]
fn unapplied_file_changes_remain_lifecycle_facts_without_change_side_effects() {
    for (status, expected_outcome) in [
        ("failed", ConversationItemOutcome::Failed),
        ("declined", ConversationItemOutcome::Declined),
        ("inProgress", ConversationItemOutcome::InProgress),
        (
            "futureFileStatus",
            ConversationItemOutcome::Unknown("futureFileStatus".to_string()),
        ),
    ] {
        let (sender, receiver) = channel();
        let mut state = ActiveTurnNotificationState::new();
        let notification = notification_from_value(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 1,
                "item": {
                    "id": format!("file-{status}"),
                    "type": "fileChange",
                    "changes": [{
                        "path": RESULT_OUTPUT_FILE_PATH,
                        "kind": { "type": "update" }
                    }],
                    "status": status
                }
            }
        }));

        assert_eq!(
            handle_turn_notification(&notification, "thread-1", "turn-1", &mut state, &sender,)
                .unwrap(),
            TurnNotificationHandling::Consumed
        );
        let events = receiver.try_iter().collect::<Vec<_>>();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            ConversationStreamEvent::ItemLifecycleObserved { observation }
                if observation.outcome == expected_outcome
        ));
        assert!(state.changed_planning_file_paths().is_empty());
    }
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

fn fixture_value(path: &str) -> Value {
    serde_json::from_str(fixture_body(path)).expect("protocol fixture should be valid JSON")
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
        "fixtures/item_lifecycle_thread_read_response.json" => {
            include_str!("fixtures/item_lifecycle_thread_read_response.json")
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

fn assert_item_lifecycle_event(
    event: ConversationStreamEvent,
    expected_item_id: &str,
    expected_phase: ConversationItemLifecyclePhase,
) -> ConversationItemLifecycleObservation {
    let ConversationStreamEvent::ItemLifecycleObserved { observation } = event else {
        panic!("expected item lifecycle event, got {event:?}");
    };
    assert_eq!(observation.thread_id, "thread-item-lifecycle");
    assert_eq!(observation.turn_id, "turn-item-lifecycle");
    assert_eq!(observation.item_id, expected_item_id);
    assert_eq!(observation.phase, expected_phase);
    assert_eq!(observation.source, ConversationItemLifecycleSource::Live);
    *observation
}

fn started_item_fixture(item: &Value) -> Value {
    let mut started = item.clone();
    let object = started
        .as_object_mut()
        .expect("fixture ThreadItem should be an object");
    let item_type = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    match item_type.as_str() {
        "userMessage" => object.insert("content".to_string(), json!([])),
        "hookPrompt" => object.insert("fragments".to_string(), json!([])),
        "agentMessage" | "plan" => object.insert("text".to_string(), json!("")),
        "reasoning" => {
            object.insert("summary".to_string(), json!([]));
            object.insert("content".to_string(), json!([]))
        }
        "commandExecution" => {
            object.insert("command".to_string(), json!(""));
            object.insert("commandActions".to_string(), json!([]));
            object.insert("cwd".to_string(), json!(""));
            object.insert("status".to_string(), json!("inProgress"))
        }
        "fileChange" => {
            object.insert("changes".to_string(), json!([]));
            object.insert("status".to_string(), json!("inProgress"))
        }
        "mcpToolCall" => {
            object.insert("arguments".to_string(), json!({}));
            object.insert("status".to_string(), json!("inProgress"))
        }
        "dynamicToolCall" => {
            object.insert("arguments".to_string(), json!({}));
            object.insert("status".to_string(), json!("inProgress"));
            object.insert("success".to_string(), Value::Null)
        }
        "collabAgentToolCall" => {
            object.insert("agentsStates".to_string(), json!({}));
            object.insert("receiverThreadIds".to_string(), json!([]));
            object.insert("prompt".to_string(), Value::Null);
            object.insert("status".to_string(), json!("inProgress"))
        }
        "subAgentActivity" => object.insert("kind".to_string(), json!("started")),
        "webSearch" => object.insert("query".to_string(), json!("")),
        "imageView" => object.insert("path".to_string(), json!("")),
        "sleep" => object.insert("durationMs".to_string(), json!(0)),
        "imageGeneration" => {
            object.insert("result".to_string(), json!(""));
            object.insert("status".to_string(), json!("in_progress"))
        }
        "enteredReviewMode" | "exitedReviewMode" => object.insert("review".to_string(), json!("")),
        "contextCompaction" => None,
        unknown => panic!("fixture contains unexpected item type: {unknown}"),
    };
    started
}

fn thread_item_wire_type(item_schema: &Value) -> String {
    item_schema
        .pointer("/properties/type/enum/0")
        .and_then(Value::as_str)
        .expect("ThreadItem variant should carry one type discriminator")
        .to_string()
}

fn assert_item_fields_are_explicitly_classified(
    row: &ItemProjectionManifestRow,
    item_schema: &Value,
) {
    let schema_fields = item_schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("ThreadItem variant should expose properties")
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let classified_fields = row
        .preserved_fields
        .iter()
        .chain(row.bounded_redacted_fields.iter())
        .chain(row.ignored_fields.iter())
        .map(|field| (*field).to_string())
        .collect::<BTreeSet<_>>();
    let classified_count =
        row.preserved_fields.len() + row.bounded_redacted_fields.len() + row.ignored_fields.len();

    assert_eq!(
        classified_count,
        classified_fields.len(),
        "projection decisions for {} must be disjoint",
        row.wire_type
    );
    assert_eq!(
        classified_fields, schema_fields,
        "every source field for {} requires preserve, bounded-redact, or ignore",
        row.wire_type
    );
}

fn assert_item_required_fields_match_schema(row: &ItemProjectionManifestRow, item_schema: &Value) {
    assert_eq!(
        row.required_fields
            .iter()
            .map(|field| (*field).to_string())
            .collect::<BTreeSet<_>>(),
        json_string_set(item_schema.get("required")),
        "required/optional drift for {} needs a decoder decision",
        row.wire_type
    );
}

fn assert_item_outcome_statuses_match_schema(
    row: &ItemProjectionManifestRow,
    item_schema: &Value,
    schema_root: &Value,
) {
    let Some(status_schema) = item_schema.pointer("/properties/status") else {
        assert!(row.outcome_statuses.is_empty());
        assert!(!row.open_outcome_status);
        return;
    };
    let schema_statuses = resolved_schema_enum(status_schema, schema_root);
    if row.open_outcome_status {
        assert!(schema_statuses.is_none());
        assert!(row.outcome_statuses.is_empty());
        assert_eq!(
            resolved_schema_types(status_schema, schema_root),
            BTreeSet::from(["string".to_string()]),
            "open status for {} must remain a string accepted by the decoder",
            row.wire_type
        );
        return;
    }
    let schema_statuses = schema_statuses.expect("closed item status should resolve to an enum");
    let manifest_statuses = row
        .outcome_statuses
        .iter()
        .map(|status| (*status).to_string())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        manifest_statuses, schema_statuses,
        "new status for {} requires an explicit outcome decision",
        row.wire_type
    );
}

fn json_string_set(value: Option<&Value>) -> BTreeSet<String> {
    value
        .and_then(Value::as_array)
        .expect("schema field should be a string array")
        .iter()
        .map(|value| {
            value
                .as_str()
                .expect("schema array value should be a string")
                .to_string()
        })
        .collect()
}

fn resolved_schema_enum(node: &Value, schema_root: &Value) -> Option<BTreeSet<String>> {
    if let Some(values) = node.get("enum").and_then(Value::as_array) {
        return Some(
            values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .expect("enum values should be strings")
                        .to_string()
                })
                .collect(),
        );
    }
    if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
        return schema_root
            .pointer(reference.strip_prefix('#')?)
            .and_then(|resolved| resolved_schema_enum(resolved, schema_root));
    }
    for keyword in ["allOf", "anyOf", "oneOf"] {
        if let Some(items) = node.get(keyword).and_then(Value::as_array) {
            for item in items {
                if let Some(values) = resolved_schema_enum(item, schema_root) {
                    return Some(values);
                }
            }
        }
    }
    None
}

fn resolved_schema_types(node: &Value, schema_root: &Value) -> BTreeSet<String> {
    if let Some(kind) = node.get("type").and_then(Value::as_str) {
        return BTreeSet::from([kind.to_string()]);
    }
    if let Some(kinds) = node.get("type").and_then(Value::as_array) {
        return kinds
            .iter()
            .map(|kind| {
                kind.as_str()
                    .expect("schema type array values should be strings")
                    .to_string()
            })
            .collect();
    }
    if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
        return schema_root
            .pointer(reference.strip_prefix('#').unwrap_or_default())
            .map_or_else(BTreeSet::new, |resolved| {
                resolved_schema_types(resolved, schema_root)
            });
    }
    ["allOf", "anyOf", "oneOf"]
        .into_iter()
        .filter_map(|keyword| node.get(keyword).and_then(Value::as_array))
        .flat_map(|items| items.iter())
        .flat_map(|item| resolved_schema_types(item, schema_root))
        .collect()
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
