use super::*;
use serde_json::{Value, json};

fn sample_commands() -> Vec<KernelCommand> {
    vec![
        KernelCommand::LoadSessions {},
        KernelCommand::RenameSession {
            thread_id: "thread-1".to_string(),
            name: "Renamed".to_string(),
        },
        KernelCommand::SubmitPrompt {
            thread_id: "thread-1".to_string(),
            prompt: "fix the failing test".to_string(),
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
    // plan 18 K3-3b: the prompt goes to the addressed thread; the kernel's workspace prepares it.
    assert_eq!(
        KernelCommand::SubmitPrompt {
            thread_id: "thread-2".to_string(),
            prompt: "fix the failing test".to_string(),
        }
        .into_core("/workspace"),
        Ok(AppCommand::SubmitThreadPrompt(Box::new(
            ThreadPromptSubmission {
                workspace_directory: "/workspace".to_string(),
                thread_id: "thread-2".to_string(),
                prompt: "fix the failing test".to_string(),
            }
        )))
    );
    assert_eq!(
        KernelCommand::SubmitPrompt {
            thread_id: "thread-2".to_string(),
            prompt: " ".to_string(),
        }
        .into_core("/workspace")
        .map_err(|error| error.code),
        Err(ERROR_INVALID_PARAMS)
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
    // The confirmation travels with the core command, so the planning service checks it too.
    assert_eq!(
        KernelCommand::ResetPlanning {
            target: ResetTarget::Directions,
            confirm: true,
        }
        .into_core("/workspace"),
        Ok(AppCommand::ResetPlanningWorkspace(
            PlanningWorkspaceResetIntent::new(
                "/workspace",
                PlanningWorkspaceResetTarget::Directions,
            )
            .with_confirmation(true)
        ))
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
    serde_json::from_value::<CommandRequest>(json!({"command": {"type": "stop_all"}}))
        .expect("a command request needs only the command");
    assert!(
        serde_json::from_value::<CommandRequest>(
            json!({"command": {"type": "stop_all"}, "idempotency_key": "k-1"})
        )
        .is_err(),
        "v0 has no idempotency key; a client must not think a retry is deduplicated"
    );
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
    // plan 18 K3 review: only the shared auto-follow settings leave the kernel, not the shown turn.
    let runtime = ConversationRuntimeSnapshot::initial();
    assert_eq!(
        KernelEvent::from_core(&AppEvent::ConversationRuntimeAuthorityChanged(Box::new(
            runtime.clone()
        ))),
        Some(KernelEvent::AutoFollow(AutoFollowSettings {
            max_turns: runtime.auto_follow.max_auto_turns,
            paused: runtime.auto_follow.continuation_paused,
        }))
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
        KernelEvent::from_core(&AppEvent::ThreadPromptAdmissionResolved {
            thread_id: "thread-2".to_string(),
            admission: ThreadPromptAdmission::RejectedUnknownThread,
        }),
        Some(KernelEvent::PromptSubmission(PromptSubmissionState {
            thread_id: "thread-2".to_string(),
            status: PromptSubmissionStatus::Rejected,
            reason: Some(PromptSubmissionReason::UnknownThread),
            task_id: None,
            message: Some("unknown thread; load_sessions lists the known threads".to_string()),
        }))
    );
    assert_eq!(
        KernelEvent::from_core(&AppEvent::ThreadPromptAdmissionResolved {
            thread_id: "thread-2".to_string(),
            admission: ThreadPromptAdmission::RejectedParallelMode,
        }),
        Some(KernelEvent::PromptSubmission(PromptSubmissionState {
            thread_id: "thread-2".to_string(),
            status: PromptSubmissionStatus::Rejected,
            reason: Some(PromptSubmissionReason::ParallelMode),
            task_id: None,
            message: Some(
                "parallel mode is on; the kernel starts no thread prompt until it is off"
                    .to_string()
            ),
        }))
    );
    assert_eq!(
        KernelEvent::from_core(&AppEvent::ThreadPromptSubmitted {
            thread_id: "thread-2".to_string(),
            outcome: ThreadPromptOutcome::PreparationFailed(
                ThreadPromptPreparationFailure::PlanningRefused {
                    reason: "planning is invalid".to_string(),
                }
            ),
        }),
        Some(KernelEvent::PromptSubmission(PromptSubmissionState {
            thread_id: "thread-2".to_string(),
            status: PromptSubmissionStatus::Failed,
            reason: Some(PromptSubmissionReason::PlanningRefused),
            task_id: None,
            message: Some("planning is invalid".to_string()),
        }))
    );
    // plan 18 K3 review: a queued task names its id so the client does not resend the prompt.
    assert_eq!(
        KernelEvent::from_core(&AppEvent::ThreadPromptSubmitted {
            thread_id: "thread-2".to_string(),
            outcome: ThreadPromptOutcome::Submitted {
                admission: TurnSubmissionAdmission::RejectedUnavailable,
                task_id: "task-7".to_string(),
            },
        }),
        Some(KernelEvent::PromptSubmission(PromptSubmissionState {
            thread_id: "thread-2".to_string(),
            status: PromptSubmissionStatus::Queued,
            reason: Some(PromptSubmissionReason::Unavailable),
            task_id: Some("task-7".to_string()),
            message: Some(
                "the thread cannot take a prompt now; planning task task-7 stays queued, \
                 do not resend the prompt"
                    .to_string()
            ),
        }))
    );
    assert_eq!(
        KernelEvent::from_core(&AppEvent::ThreadPromptSubmitted {
            thread_id: "thread-2".to_string(),
            outcome: ThreadPromptOutcome::ParallelModeStarted {
                task_id: "task-7".to_string(),
            },
        })
        .and_then(|event| match event {
            KernelEvent::PromptSubmission(state) => Some((state.status, state.reason)),
            _ => None,
        }),
        Some((
            PromptSubmissionStatus::Queued,
            Some(PromptSubmissionReason::ParallelModeStarted)
        ))
    );
    assert_eq!(
        KernelEvent::from_core(&AppEvent::ThreadPromptAdmissionResolved {
            thread_id: "thread-2".to_string(),
            admission: ThreadPromptAdmission::RejectedParallelModeUnknown,
        })
        .and_then(|event| match event {
            KernelEvent::PromptSubmission(state) => state.reason,
            _ => None,
        }),
        Some(PromptSubmissionReason::ParallelModeUnknown)
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
    assert_eq!(
        serde_json::to_value(KernelEvent::PromptSubmission(PromptSubmissionState {
            thread_id: "thread-1".to_string(),
            status: PromptSubmissionStatus::Preparing,
            reason: None,
            task_id: None,
            message: None,
        }))
        .expect("event should serialize"),
        json!({"type": "prompt_submission", "thread_id": "thread-1", "status": "preparing"})
    );
    // plan 19 P3: a refusal carries a reason code that tells the client what to do next.
    assert_eq!(
        serde_json::to_value(KernelEvent::from_core(&AppEvent::ThreadPromptSubmitted {
            thread_id: "thread-1".to_string(),
            outcome: ThreadPromptOutcome::PreparationFailed(
                ThreadPromptPreparationFailure::BootstrapReviewRequired {
                    draft_name: "draft-1".to_string(),
                }
            ),
        }))
        .expect("event should serialize")["reason"],
        json!("bootstrap_review_required")
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
        KernelQuery::AutoFollow {},
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
        KernelEvent::AutoFollow(AutoFollowSettings::from_core(
            &ConversationRuntimeSnapshot::initial(),
        )),
        KernelEvent::ThreadActivity {
            threads: Vec::new(),
        },
        KernelEvent::PromptSubmission(PromptSubmissionState {
            thread_id: "thread-1".to_string(),
            status: PromptSubmissionStatus::Submitted,
            reason: None,
            task_id: None,
            message: None,
        }),
        KernelEvent::Resync,
    ]) {
        assert_eq!(
            serde_json::to_value(event).expect("event should serialize")["type"],
            json!(name)
        );
    }
}

#[test]
fn command_response_names_why_the_kernel_refused_a_command() {
    // plan 18 K3 review: steer and approval refusals reach the client that sent them.
    assert_eq!(
        CommandResponse::from_core(None),
        CommandResponse {
            processed: false,
            rejection: None,
        }
    );
    assert_eq!(
        CommandResponse::from_core(Some(&[])),
        CommandResponse {
            processed: true,
            rejection: None,
        }
    );
    let refused = CommandResponse::from_core(Some(&[AppEvent::TurnSteerAdmissionResolved(
        crate::core::app::TurnSteerAdmission::RejectedUnavailable,
    )]));
    assert_eq!(
        refused.rejection.map(|rejection| rejection.reason),
        Some(CommandRejectionReason::NoMatchingTurn)
    );
    let refused = CommandResponse::from_core(Some(&[AppEvent::ApprovalDecisionAdmissionResolved(
        crate::core::app::ApprovalDecisionAdmission::RejectedUnavailable,
    )]));
    assert_eq!(
        serde_json::to_value(refused).expect("response should serialize"),
        json!({
            "processed": true,
            "rejection": {
                "reason": "no_matching_approval",
                "message": "no pending approval has that id"
            }
        })
    );
}

#[test]
fn a_query_answers_with_the_event_of_the_same_name() {
    let snapshot = crate::core::app::AppSnapshot::initial();
    let request: QueryRequest =
        serde_json::from_value(json!({ "query": { "type": "auto_follow" } }))
            .expect("query request should parse");
    assert_eq!(
        request.query.answer(&snapshot, &[]),
        KernelEvent::AutoFollow(AutoFollowSettings::from_core(
            &snapshot.conversation_runtime
        ))
    );
    assert_eq!(
        KernelQuery::ThreadActivity {}.answer(&snapshot, &[]),
        KernelEvent::ThreadActivity {
            threads: Vec::new()
        }
    );
    assert!(
        serde_json::from_value::<QueryRequest>(
            json!({ "query": { "type": "sessions" }, "extra": 1 })
        )
        .is_err()
    );
    assert_eq!(
        serde_json::to_value(EventNotification {
            event: KernelEvent::Resync
        })
        .expect("notification should serialize"),
        json!({ "event": { "type": "resync" } })
    );
}
