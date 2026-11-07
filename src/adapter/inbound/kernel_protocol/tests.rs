use super::*;
use serde_json::{Value, json};

fn sample_commands() -> Vec<KernelCommand> {
    vec![
        KernelCommand::LoadSessions {},
        KernelCommand::RenameSession {
            thread_id: "thread-1".to_string(),
            name: "Renamed".to_string(),
        },
        KernelCommand::SteerTurn {
            thread_id: "thread-1".to_string(),
            expected_turn_id: "turn-1".to_string(),
            prompt: "focus on tests".to_string(),
        },
        KernelCommand::StopAll {},
        KernelCommand::DecideApproval {
            approval_id: "approval-1".to_string(),
            server_request_id: "7".to_string(),
            decision: ApprovalDecision::Accept,
        },
        KernelCommand::SetAutoFollowMaxTurns { value: 3 },
        KernelCommand::PauseAutoFollow {},
        KernelCommand::ResetPlanning {
            target: ResetTarget::All,
            confirm: true,
        },
    ]
}

#[test]
fn every_command_round_trips_and_is_described() {
    let described = describe().commands;
    let samples = sample_commands();
    assert_eq!(samples.len(), described.len());
    for (command, name) in samples.into_iter().zip(described) {
        let encoded = serde_json::to_value(&command).expect("command should serialize");
        assert_eq!(encoded["type"], Value::String(name));
        let decoded: KernelCommand =
            serde_json::from_value(encoded).expect("command should deserialize");
        assert_eq!(decoded, command);
    }
}

#[test]
fn every_command_maps_to_one_core_command() {
    for command in sample_commands() {
        let label = format!("{command:?}");
        let core = command
            .into_core("/workspace")
            .unwrap_or_else(|error| panic!("{label} should map: {error:?}"));
        assert!(
            !matches!(core, AppCommand::Noop),
            "{label} must not map to Noop"
        );
    }
    assert_eq!(
        KernelCommand::SteerTurn {
            thread_id: "thread-1".to_string(),
            expected_turn_id: "turn-1".to_string(),
            prompt: "focus".to_string(),
        }
        .into_core("/workspace"),
        Ok(AppCommand::SteerTurn(ConversationTurnSteerRequest {
            thread_id: "thread-1".to_string(),
            expected_turn_id: "turn-1".to_string(),
            prompt: "focus".to_string(),
        }))
    );
    assert_eq!(
        KernelCommand::PauseAutoFollow {}.into_core("/workspace"),
        Ok(AppCommand::PausePostTurnContinuation)
    );
}

#[test]
fn reset_requires_confirmation_and_blank_ids_are_rejected() {
    let unconfirmed: KernelCommand =
        serde_json::from_value(json!({"type": "reset_planning", "target": "queue"}))
            .expect("confirm defaults to false");
    assert_eq!(
        unconfirmed
            .into_core("/workspace")
            .map_err(|error| error.code),
        Err(ERROR_CONFIRMATION_REQUIRED)
    );
    assert_eq!(
        KernelCommand::RenameSession {
            thread_id: "  ".to_string(),
            name: "Renamed".to_string(),
        }
        .into_core("/workspace")
        .map_err(|error| error.code),
        Err(ERROR_INVALID_PARAMS)
    );
    // plan 18 K3-3a: remote clients address threads; they cannot move the shown slot.
    for shown_slot_command in ["open_conversation", "new_draft"] {
        assert!(
            serde_json::from_value::<KernelCommand>(json!({"type": shown_slot_command})).is_err(),
            "{shown_slot_command} is not part of the remote protocol"
        );
    }
}

#[test]
fn unknown_commands_and_fields_are_refused() {
    assert!(
        serde_json::from_value::<KernelCommand>(json!({"type": "evaluate_post_turn"})).is_err()
    );
    assert!(
        serde_json::from_value::<KernelCommand>(json!({"type": "stop_all", "force": true}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<CommandRequest>(
            json!({"command": {"type": "stop_all"}, "extra": 1})
        )
        .is_err()
    );
    let request: CommandRequest = serde_json::from_value(json!({"command": {"type": "stop_all"}}))
        .expect("idempotency key is optional");
    assert_eq!(request.idempotency_key, None);
}

#[test]
fn only_coarse_core_events_are_exported() {
    let activity = ThreadActivitySummary {
        thread_id: "thread-1".to_string(),
        running: true,
        active_turn_id: None,
        approval: None,
        auto_follow_active: false,
    };
    assert_eq!(
        KernelEvent::from_core(&AppEvent::ThreadActivityChanged(vec![activity])),
        Some(KernelEvent::ThreadActivity {
            threads: vec![ThreadActivity {
                thread_id: "thread-1".to_string(),
                running: true,
                active_turn_id: None,
                approval: None,
                auto_follow_active: false,
            }],
        })
    );
    let runtime = ConversationRuntimeSnapshot::initial();
    let Some(KernelEvent::ConversationRuntime(summary)) = KernelEvent::from_core(
        &AppEvent::ConversationRuntimeAuthorityChanged(Box::new(runtime.clone())),
    ) else {
        panic!("runtime authority should export a summary");
    };
    assert_eq!(summary.active_turn, None);
    assert_eq!(summary.post_turn, PostTurnStatus::Idle);
    assert_eq!(
        summary.auto_follow.max_turns,
        runtime.auto_follow.max_auto_turns
    );
    assert_eq!(
        KernelEvent::from_core(&AppEvent::SessionCatalogChanged(
            SessionCatalogSnapshot::Loading
        )),
        Some(KernelEvent::Sessions(SessionsState {
            status: SessionsStatus::Loading,
            sessions: Vec::new(),
            message: None,
        }))
    );
    assert_eq!(
        KernelEvent::from_core(&AppEvent::ParallelModeSupervisorSnapshotInvalidated),
        None,
        "fine-grained internal events stay inside the kernel"
    );
}

#[test]
fn events_and_description_have_stable_wire_shapes() {
    assert_eq!(
        serde_json::to_value(KernelEvent::Resync).expect("event should serialize"),
        json!({"type": "resync"})
    );
    let description = serde_json::to_value(describe()).expect("description should serialize");
    assert_eq!(description["protocol_version"], json!(PROTOCOL_VERSION));
    assert_eq!(
        description["methods"],
        json!([
            "akra/kernel/attach",
            "akra/kernel/describe",
            "akra/kernel/command",
            "akra/kernel/query",
            "akra/kernel/event",
            "akra/kernel/detach"
        ])
    );
    for (name, query) in describe().queries.iter().zip([
        KernelQuery::Sessions {},
        KernelQuery::ConversationRuntime {},
        KernelQuery::ThreadActivity {},
    ]) {
        assert_eq!(
            serde_json::to_value(query).expect("query should serialize")["type"],
            json!(name)
        );
    }
    for (name, event) in describe().events.iter().zip([
        KernelEvent::Sessions(SessionsState {
            status: SessionsStatus::Idle,
            sessions: Vec::new(),
            message: None,
        }),
        KernelEvent::ConversationRuntime(ConversationRuntimeSummary::from_core(
            &ConversationRuntimeSnapshot::initial(),
        )),
        KernelEvent::ThreadActivity {
            threads: Vec::new(),
        },
        KernelEvent::Resync,
    ]) {
        assert_eq!(
            serde_json::to_value(event).expect("event should serialize")["type"],
            json!(name)
        );
    }
}
