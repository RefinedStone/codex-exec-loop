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
        KernelCommand::RefreshPlanning {},
    ]
}

fn no_parallel() -> ParallelSource<'static> {
    ParallelSource {
        enabled: false,
        supervisor: None,
        status: None,
    }
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
    // plan 18 K3-6: the kernel reads the planning queue of its own workspace.
    assert_eq!(
        KernelCommand::RefreshPlanning {}.into_core("/workspace"),
        Ok(AppCommand::RefreshPlanningRuntime {
            workspace_directory: "/workspace".to_string(),
        })
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
}

fn command_request(command: serde_json::Value, key: Option<&str>) -> CommandRequest {
    let mut request = json!({ "command": command });
    if let Some(key) = key {
        request["idempotency_key"] = json!(key);
    }
    serde_json::from_value(request).expect("the request should parse")
}

#[test]
fn a_command_is_described_for_the_record_without_its_content() {
    // plan 19 Q3-6: the kernel records the command name, its thread, and a fingerprint of the content.
    let steer = json!({"type": "steer_turn", "thread_id": "thread-1", "expected_turn_id": "turn-1", "prompt": "secret words"});
    let described = command_request(steer.clone(), Some("key-1"))
        .descriptor()
        .expect("the key is valid")
        .expect("steer is recorded");
    assert_eq!(described.command, "steer_turn");
    assert_eq!(described.thread_id, "thread-1");
    assert_eq!(described.idempotency_key.as_deref(), Some("key-1"));
    assert_eq!(described.fingerprint.len(), 32);
    assert!(!format!("{described:?}").contains("secret"));

    // 같은 명령은 같은 지문이고, 내용이 다르면 지문이 다르다. 키는 지문에 들지 않는다.
    let same = command_request(steer, None).descriptor().unwrap().unwrap();
    assert_eq!(same.fingerprint, described.fingerprint);
    assert_eq!(same.idempotency_key, None);
    let other = json!({"type": "steer_turn", "thread_id": "thread-1", "expected_turn_id": "turn-1", "prompt": "other words"});
    let other = command_request(other, None).descriptor().unwrap().unwrap();
    assert_ne!(other.fingerprint, described.fingerprint);
    // 지문은 binary 가 바뀌어도 같아야 한다. 다시 조립한 커널이 기록의 지문과 견준다.
    let stop = command_request(json!({"type": "stop_all"}), None)
        .descriptor()
        .unwrap()
        .unwrap();
    // `printf '%s' '{"type":"stop_all"}' | shasum -a 256` 의 앞 32글자이다.
    assert_eq!(stop.fingerprint, "3d50b0f9d9db44466ce2c5fcde73eb7f");
    assert_eq!(stop.thread_id, "");
    // 필드가 여럿인 명령은 키를 정렬한 JSON 의 지문이다:
    // `{"expected_turn_id":"turn-1","prompt":"secret words","thread_id":"thread-1","type":"steer_turn"}`.
    assert_eq!(described.fingerprint, "7a79475eb8e56bb1a8b1318dd9076ca9");
}

#[test]
fn read_commands_are_not_recorded_and_bad_keys_are_refused() {
    for read in ["load_sessions", "refresh_planning"] {
        let request = command_request(json!({ "type": read }), Some("key-1"));
        assert_eq!(request.descriptor().expect("the key is valid"), None);
    }
    for key in ["", "   ", "line\nbreak", &"k".repeat(129)] {
        let request = command_request(json!({"type": "stop_all"}), Some(key));
        assert_eq!(
            request
                .descriptor()
                .expect_err("the key should be refused")
                .code,
            ERROR_INVALID_PARAMS,
            "{key:?}"
        );
    }
    command_request(json!({"type": "stop_all"}), Some(&"k".repeat(128)))
        .descriptor()
        .expect("128 characters are allowed");
}

#[test]
fn a_command_response_carries_the_operation_receipt() {
    use crate::domain::kernel_event::OperationReceipt;
    let accepted = CommandResponse::from_core(Some(&[])).with_operation(Some(OperationReceipt {
        op_id: 41,
        rejection: None,
        replayed: false,
    }));
    assert_eq!(
        serde_json::to_value(accepted).expect("response should serialize"),
        json!({
            "processed": true,
            "operation": { "op_id": 41, "state": "accepted", "next": { "query": "history", "after": 41 } },
        })
    );
    let replayed = CommandResponse::replayed(OperationReceipt {
        op_id: 41,
        rejection: Some(CommandRejectionReason::NoMatchingTurn),
        replayed: true,
    });
    assert_eq!(
        serde_json::to_value(replayed).expect("response should serialize"),
        json!({
            "processed": true,
            "rejection": { "reason": "no_matching_turn", "message": "the thread has no running turn with that id" },
            "operation": {
                "op_id": 41,
                "state": "rejected",
                "next": { "query": "history", "after": 41 },
                "replayed": true,
            },
        })
    );
    // 영수증이 없는 답은 이전과 같은 모양이다.
    assert_eq!(
        serde_json::to_value(CommandResponse::from_core(Some(&[])).with_operation(None))
            .expect("response should serialize"),
        json!({ "processed": true })
    );
    assert_eq!(
        KernelProtocolError::idempotency_key_reused().code,
        ERROR_IDEMPOTENCY_KEY_REUSED
    );
    assert_eq!(
        KernelProtocolError::idempotency_unavailable().code,
        ERROR_IDEMPOTENCY_UNAVAILABLE
    );
}

#[test]
fn a_prompt_refused_before_preparation_is_a_rejected_command() {
    // plan 19 Q3-6 review: the receipt of `submit_prompt` must not say accepted when core refused it at once.
    let refused = CommandResponse::from_core(Some(&[AppEvent::ThreadPromptAdmissionResolved {
        thread_id: "thread-1".to_string(),
        admission: crate::core::app::ThreadPromptAdmission::RejectedBusy,
    }]))
    .with_operation(Some(crate::domain::kernel_event::OperationReceipt {
        op_id: 5,
        rejection: Some(CommandRejectionReason::Busy),
        replayed: false,
    }));
    let json = serde_json::to_value(refused).expect("response should serialize");
    assert_eq!(json["rejection"]["reason"], json!("busy"));
    assert_eq!(json["operation"]["state"], json!("rejected"));
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
        KernelQuery::Planning {},
        KernelQuery::Parallel {},
        KernelQuery::History {
            after: None,
            limit: None,
        },
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
        KernelEvent::Planning(PlanningState::from_core(&RuntimeProjection::uninitialized())),
        KernelEvent::Parallel(ParallelState::from_core(no_parallel())),
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
            operation: None,
        }
    );
    assert_eq!(
        CommandResponse::from_core(Some(&[])),
        CommandResponse {
            processed: true,
            rejection: None,
            operation: None,
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
fn a_recorded_event_carries_the_log_sequence_for_the_history_cursor() {
    // plan 19 Q3-3b: the live event and the history page share one record shape.
    let event = KernelEvent::Recorded(crate::domain::kernel_event::RecordedKernelEvent {
        sequence: 42,
        recorded_at: "2026-11-11T00:00:00Z".to_string(),
        event: crate::domain::kernel_event::KernelEvent::TurnEnded {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            outcome: Some(crate::domain::kernel_event::TurnEndOutcome::Interrupted),
        },
    });
    assert_eq!(
        serde_json::to_value(&event).expect("the event should serialize"),
        json!({
            "type": "recorded",
            "sequence": 42,
            "recorded_at": "2026-11-11T00:00:00Z",
            "event": {
                "kind": "turn_ended",
                "thread_id": "thread-1",
                "turn_id": "turn-1",
                "outcome": "interrupted",
            },
        })
    );
    assert!(describe().events.contains(&"recorded".to_string()));
}

#[test]
fn approval_records_reach_the_client_without_the_request_content() {
    // plan 19 Q3-5: the request and its end are records. `describe` lists every record kind.
    use crate::domain::kernel_event::{
        ApprovalRequestKind, ApprovalResolution, KernelEvent as Record, PendingApproval,
    };
    let approval = PendingApproval::new(
        "thread-1",
        Some("turn-1"),
        "approval-1",
        ApprovalRequestKind::CommandExecution,
        "Command execution requested.",
    );
    let recorded = |sequence, event| {
        let event = KernelEvent::Recorded(RecordedKernelEvent {
            sequence,
            recorded_at: "2026-11-11T00:00:00Z".to_string(),
            event,
        });
        serde_json::to_value(&event).expect("the event should serialize")["event"].clone()
    };
    assert_eq!(
        recorded(7, Record::ApprovalRequested(approval.clone())),
        json!({
            "kind": "approval_requested",
            "thread_id": "thread-1",
            "turn_id": "turn-1",
            "approval_id": "approval-1",
            "request_kind": "command_execution",
            "summary": "Command execution requested.",
        })
    );
    assert_eq!(
        recorded(8, approval.resolved(ApprovalResolution::Lost)),
        json!({
            "kind": "approval_resolved",
            "thread_id": "thread-1",
            "approval_id": "approval-1",
            "resolution": "lost",
        })
    );
    let records = describe().records;
    for kind in ["approval_requested", "approval_resolved", "turn_ended"] {
        assert!(records.contains(&kind.to_string()), "{kind}");
    }
}

#[test]
fn a_history_query_reads_after_its_cursor_and_answers_with_the_page() {
    // plan 19 Q3-3: the gateway reads the page from the kernel log and the answer carries it.
    let first: QueryRequest = serde_json::from_value(json!({ "query": { "type": "history" } }))
        .expect("a bare history query should parse");
    assert_eq!(first.query.history_range(), Some((0, 100)));
    let next: QueryRequest =
        serde_json::from_value(json!({ "query": { "type": "history", "after": 41, "limit": 5 } }))
            .expect("a history query with a cursor should parse");
    assert_eq!(next.query.history_range(), Some((41, 5)));
    assert_eq!(KernelQuery::Planning {}.history_range(), None);

    let page = crate::domain::kernel_event::KernelEventPage {
        records: vec![crate::domain::kernel_event::RecordedKernelEvent {
            sequence: 42,
            recorded_at: "2026-11-11T00:00:00Z".to_string(),
            event: crate::domain::kernel_event::KernelEvent::TurnStarted {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
            },
        }],
        next_after: 42,
    };
    let snapshot = crate::core::app::AppSnapshot::initial();
    let answer = next
        .query
        .answer(&snapshot, &[], no_parallel(), Some(page.clone()));
    assert_eq!(answer, KernelEvent::History(page));
    assert_eq!(
        serde_json::to_value(&answer).expect("the answer should serialize"),
        json!({
            "type": "history",
            "records": [{
                "sequence": 42,
                "recorded_at": "2026-11-11T00:00:00Z",
                "event": { "kind": "turn_started", "thread_id": "thread-1", "turn_id": "turn-1" },
            }],
            "next_after": 42,
        })
    );
    assert_eq!(
        first.query.answer(&snapshot, &[], no_parallel(), None),
        KernelEvent::History(crate::domain::kernel_event::KernelEventPage {
            records: Vec::new(),
            next_after: 0,
        }),
        "a kernel without a log answers with an empty page"
    );
}

#[test]
fn a_query_answers_with_the_event_of_the_same_name() {
    let snapshot = crate::core::app::AppSnapshot::initial();
    let request: QueryRequest =
        serde_json::from_value(json!({ "query": { "type": "auto_follow" } }))
            .expect("query request should parse");
    assert_eq!(
        request.query.answer(&snapshot, &[], no_parallel(), None),
        KernelEvent::AutoFollow(AutoFollowSettings::from_core(
            &snapshot.conversation_runtime
        ))
    );
    assert_eq!(
        KernelQuery::ThreadActivity {}.answer(&snapshot, &[], no_parallel(), None),
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

fn queue_task(rank: usize, task_id: &str, status: TaskStatus) -> PriorityQueueTask {
    PriorityQueueTask {
        rank,
        task_id: task_id.to_string(),
        direction_id: "direction-1".to_string(),
        direction_title: "Ship K3".to_string(),
        task_title: format!("task {task_id}"),
        status,
        combined_priority: 10,
        updated_at: "2026-11-10T00:00:00Z".to_string(),
        rank_reasons: vec!["priority".to_string()],
    }
}

#[test]
fn planning_state_carries_the_queue_of_the_kernel_workspace() {
    // plan 18 K3-6: a remote client sees the same queue as the TUI planning panel.
    let head = queue_task(1, "task-1", TaskStatus::InProgress);
    let projection = RuntimeProjection::ready_with_queue_projection(
        "fragment".to_string(),
        "2 tasks".to_string(),
        None,
        Some(head.clone()),
        crate::domain::planning::PriorityQueueProjection {
            next_task: Some(head.clone()),
            active_tasks: vec![head, queue_task(2, "task-2", TaskStatus::Ready)],
            proposed_tasks: vec![queue_task(3, "task-3", TaskStatus::Proposed)],
            skipped_tasks: vec![crate::domain::planning::PriorityQueueSkippedTask {
                task_id: "task-4".to_string(),
                task_title: "task task-4".to_string(),
                direction_id: "direction-1".to_string(),
                status: TaskStatus::Blocked,
                reason: "blocked by task-1".to_string(),
            }],
        },
    )
    .with_planning_revision(Some(4));

    let state = serde_json::to_value(KernelEvent::Planning(PlanningState::from_core(&projection)))
        .expect("planning should serialize");

    assert_eq!(state["type"], json!("planning"));
    assert_eq!(state["status"], json!("ready_with_task"));
    assert_eq!(state["revision"], json!(4));
    assert_eq!(
        state["next_task"],
        json!({
            "rank": 1,
            "task_id": "task-1",
            "direction_title": "Ship K3",
            "title": "task task-1",
            "status": "in_progress",
            "priority": 10
        })
    );
    assert_eq!(state["active_tasks"].as_array().map(Vec::len), Some(2));
    assert_eq!(state["proposed_tasks"][0]["status"], json!("proposed"));
    assert_eq!(
        state["skipped_tasks"],
        json!([{
            "task_id": "task-4",
            "title": "task task-4",
            "status": "blocked",
            "reason": "blocked by task-1"
        }])
    );
    assert_eq!(
        serde_json::to_value(PlanningState::from_core(&RuntimeProjection::uninitialized()))
            .expect("planning should serialize"),
        json!({
            "status": "uninitialized",
            "active_tasks": [],
            "proposed_tasks": [],
            "skipped_tasks": []
        })
    );
}

#[test]
fn parallel_state_carries_the_supervisor_board() {
    // plan 18 K3-6: pool, agents and merge queue come from the same supervisor snapshot as the TUI.
    use crate::domain::parallel_mode::{
        ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot,
        ParallelModeDistributorQueueItem, ParallelModeDistributorSnapshot,
        ParallelModePoolBoardSnapshot, ParallelModeQueueItemState,
        ParallelModeSupervisorDetailSnapshot,
    };
    let mut agent = ParallelModeAgentRosterEntry::new(
        "agent-1",
        "task task-2",
        "slot-1",
        "akra/task-2",
        "running",
        "3m",
        "editing tests",
    );
    agent.thread_id = Some("thread-9".to_string());
    let snapshot = ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/repo",
        ParallelModePoolBoardSnapshot::new(2, "pool", "reconciled", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(vec![agent], "no agents"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no session"),
        ParallelModeDistributorSnapshot::new(
            vec![ParallelModeDistributorQueueItem::new(
                "agent-1",
                "task task-2",
                ParallelModeQueueItemState::PrPending,
                "akra/task-2",
                "abc1234",
                "waiting for CI",
            )],
            Vec::new(),
            "head: task-2",
            "",
        ),
        Some("pool is full".to_string()),
    );

    let state = serde_json::to_value(KernelEvent::Parallel(ParallelState::from_core(
        ParallelSource {
            enabled: true,
            supervisor: Some(&snapshot),
            status: Some("parallel mode on"),
        },
    )))
    .expect("parallel should serialize");

    assert_eq!(state["type"], json!("parallel"));
    assert_eq!(state["enabled"], json!(true));
    assert_eq!(state["status"], json!("parallel mode on"));
    let supervisor = &state["supervisor"];
    assert_eq!(supervisor["state"], json!("supervise"));
    assert_eq!(supervisor["pool"]["configured"], json!(2));
    assert_eq!(
        supervisor["agents"],
        json!([{
            "agent_id": "agent-1",
            "thread_id": "thread-9",
            "task_title": "task task-2",
            "slot_id": "slot-1",
            "branch": "akra/task-2",
            "state": "running",
            "duration": "3m",
            "summary": "editing tests"
        }])
    );
    assert_eq!(supervisor["merge_head"], json!("head: task-2"));
    assert_eq!(
        supervisor["merge_queue"],
        json!([{
            "task_title": "task task-2",
            "state": "pr_pending",
            "branch": "akra/task-2",
            "commit": "abc1234"
        }])
    );
    assert_eq!(supervisor["notice"], json!("pool is full"));
    assert!(supervisor.get("pr_validation").is_none());
    assert_eq!(
        serde_json::to_value(ParallelState::from_core(no_parallel()))
            .expect("parallel should serialize"),
        json!({ "enabled": false }),
        "before the first supervisor refresh the kernel only knows the mode"
    );
}
