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
    for (command, entry) in samples.into_iter().zip(described) {
        let encoded = serde_json::to_value(&command).expect("command should serialize");
        assert_eq!(encoded["type"], json!(entry.name));
        assert_eq!(command.kind().name(), entry.name);
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
    assert_eq!(described.command, KernelCommandKind::SteerTurn);
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
            "rejection": { "reason": "no_matching_turn", "message": CommandRejectionReason::NoMatchingTurn.summary() },
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
    let description = describe();
    assert_eq!(description.protocol_version, PROTOCOL_VERSION);
    assert_eq!(
        json!(
            description
                .methods
                .iter()
                .map(|method| method.name)
                .collect::<Vec<_>>()
        ),
        json!([
            "akra/kernel/attach",
            "akra/kernel/describe",
            "akra/kernel/command",
            "akra/kernel/query",
            "akra/kernel/event",
            "akra/kernel/detach"
        ])
    );
    let queries = describe().queries;
    assert_eq!(queries.len(), 6);
    for (name, query) in queries.iter().map(|query| query.name).zip([
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
    let events = describe().events;
    assert_eq!(events.len(), 8);
    for (name, event) in events.iter().map(|event| event.name).zip([
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
                "message": CommandRejectionReason::NoMatchingApproval.summary()
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
    assert!(
        describe()
            .events
            .iter()
            .any(|event| event.name == "recorded")
    );
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
        assert!(records.iter().any(|record| record.name == kind), "{kind}");
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
    assert_eq!(answer, KernelQueryAnswer::History(page));
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
        KernelQueryAnswer::History(crate::domain::kernel_event::KernelEventPage {
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
        KernelQueryAnswer::AutoFollow(AutoFollowSettings::from_core(
            &snapshot.conversation_runtime
        ))
    );
    assert_eq!(
        KernelQuery::ThreadActivity {}.answer(&snapshot, &[], no_parallel(), None),
        KernelQueryAnswer::ThreadActivity {
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

// spec (plan 18 K4-1) ------------------------------------------------------------------------------

const KERNEL_SPEC_FILE: &str = "schema/akra-kernel-spec.json";

fn spec_json() -> Value {
    serde_json::to_value(describe()).expect("the spec should serialize")
}

/// 스펙의 한 목록에서 `name` 들을 꺼낸다.
fn names_of(entries: &Value) -> Vec<String> {
    entries
        .as_array()
        .expect("a spec list")
        .iter()
        .map(|entry| entry["name"].as_str().expect("an entry name").to_string())
        .collect()
}

/// tag 가 있는 전송 enum 의 스키마에서 variant 의 이름들을 꺼낸다. 이름은 타입에서 나온 값이다.
fn wire_variant_names(tagged_enum: &Value, tag: &str) -> Vec<String> {
    tagged_enum["oneOf"]
        .as_array()
        .expect("a tagged enum in $defs")
        .iter()
        .map(|variant| {
            variant["properties"][tag]["const"]
                .as_str()
                .unwrap_or_else(|| panic!("a variant without a `{tag}` constant"))
                .to_string()
        })
        .collect()
}

#[test]
fn the_spec_tables_and_the_wire_types_name_the_same_things() {
    // The tables give the meaning, the serde types give the shape. A name in one and not in the other is a
    // command, query, event, or record that the spec would describe wrongly.
    let spec = spec_json();
    for (list, type_name, tag) in [
        ("commands", "KernelCommand", "type"),
        ("queries", "KernelQuery", "type"),
        ("queries", "KernelQueryAnswer", "type"),
        ("events", "KernelEvent", "type"),
        ("records", "KernelRecord", "kind"),
    ] {
        assert_eq!(
            names_of(&spec[list]),
            wire_variant_names(&spec["$defs"][type_name], tag),
            "{list} and {type_name}"
        );
    }
    for (list, fields) in [
        ("commands", &["input", "output"][..]),
        ("queries", &["input", "output"][..]),
        ("events", &["payload"][..]),
        ("records", &["payload"][..]),
    ] {
        for entry in spec[list].as_array().expect("a spec list") {
            for field in fields {
                assert!(
                    entry[field].is_object(),
                    "{list} `{}` has no {field} schema",
                    entry["name"]
                );
            }
            assert!(
                entry["summary"]
                    .as_str()
                    .is_some_and(|summary| !summary.is_empty()),
                "{list} `{}` has no summary",
                entry["name"]
            );
        }
    }
}

#[test]
fn reason_and_error_names_are_the_wire_values() {
    let spec = spec_json();
    for reason in CommandRejectionReason::ALL {
        assert_eq!(
            serde_json::to_value(reason).expect("a reason should serialize"),
            json!(reason.name()),
            "the serde name and the table name of a reason differ"
        );
    }
    assert_eq!(
        names_of(&spec["reasons"]),
        spec["$defs"]["CommandRejectionReason"]["enum"]
            .as_array()
            .expect("the reason enum")
            .iter()
            .map(|name| name.as_str().expect("a reason name").to_string())
            .collect::<Vec<_>>()
    );
    // Every rejection of a command is a reason of the spec.
    let reasons = names_of(&spec["reasons"]);
    for command in spec["commands"].as_array().expect("commands") {
        for rejection in command["rejections"].as_array().expect("rejections") {
            assert!(reasons.contains(&rejection.as_str().expect("a reason").to_string()));
        }
    }
    let codes = spec["errors"]
        .as_array()
        .expect("errors")
        .iter()
        .map(|error| error["code"].as_i64().expect("an error code"))
        .collect::<Vec<_>>();
    assert_eq!(
        codes,
        [
            -32602, -32601, -32010, -32011, -32012, -32013, -32014, -32015
        ]
    );
}

#[test]
fn the_spec_entry_of_a_command_carries_its_meaning_and_its_input() {
    let spec = spec_json();
    let reset = spec["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["name"] == "reset_planning")
        .expect("reset_planning");
    assert_eq!(reset["scope"], "planning");
    assert_eq!(reset["effect"], "destructive");
    assert_eq!(reset["idempotency"], "natural");
    assert_eq!(
        reset["input"]["properties"]["type"]["const"],
        "reset_planning"
    );
    assert_eq!(
        reset["input"]["properties"]["target"]["$ref"],
        "#/$defs/ResetTarget"
    );
    assert_eq!(reset["input"]["additionalProperties"], false);
    assert_eq!(reset["output"]["$ref"], "#/$defs/CommandResponse");
    assert_eq!(
        spec["$defs"]["ResetTarget"]["enum"],
        json!(["queue", "directions", "all"])
    );

    let submit = spec["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["name"] == "submit_prompt")
        .expect("submit_prompt");
    assert_eq!(submit["idempotency"], "keyed");
    assert_eq!(
        submit["rejections"],
        json!([
            "busy",
            "preparation_active",
            "unknown_thread",
            "parallel_mode",
            "parallel_mode_unknown"
        ])
    );
    // The value the kernel keeps only to resubmit a queued prompt is not part of the spec.
    assert!(spec["$defs"]["QueuedThreadPrompt"]["properties"]["handoff"].is_null());
    // Code comments are for developers. The spec carries only the summaries of its tables, but it keeps
    // a field that is named `title`.
    let text = serde_json::to_string(&spec).expect("the spec should serialize");
    assert!(
        !text.contains("\"description\""),
        "a code comment reached the spec"
    );
    assert!(spec["$defs"]["PlanningTask"]["properties"]["title"].is_object());
    assert_eq!(spec["protocol_version"], 1);
    assert_eq!(
        spec["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    // The spec explains its own words, and tells a request from a notification.
    assert_eq!(
        names_of(&spec["terms"]["effect"]),
        ["read", "write", "destructive"]
    );
    assert_eq!(
        names_of(&spec["terms"]["idempotency"]),
        ["natural", "keyed"]
    );
    assert_eq!(
        names_of(&spec["terms"]["scope"]),
        ["session", "thread", "approval", "auto_follow", "planning"]
    );
    let notifications = spec["methods"]
        .as_array()
        .expect("methods")
        .iter()
        .filter(|method| method["kind"] == "notification")
        .map(|method| method["name"].as_str().expect("a method name"))
        .collect::<Vec<_>>();
    assert_eq!(notifications, ["akra/kernel/event"]);
}

#[test]
fn the_kernel_spec_file_matches_the_spec() {
    /*
     * `schema/akra-kernel-spec.json` is the exported data: Desktop types, the agent CLI, and the skill document
     * are made from it. The Rust code makes it, so it cannot differ from the protocol unless it is stale. Refresh:
     * `bash scripts/refresh_akra_kernel_spec.sh`.
     */
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(KERNEL_SPEC_FILE);
    let current = format!(
        "{}\n",
        serde_json::to_string_pretty(&describe()).expect("the spec should serialize")
    );
    if std::env::var_os("AKRA_REFRESH_KERNEL_SPEC").is_some() {
        std::fs::write(&path, &current).expect("the spec file should be written");
    }
    let stored = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{KERNEL_SPEC_FILE} cannot be read: {error}"))
        .replace("\r\n", "\n");
    assert!(
        stored == current,
        "{KERNEL_SPEC_FILE} is stale. Run `bash scripts/refresh_akra_kernel_spec.sh` and commit the file."
    );
}

/*
 * 표본 값이 스키마에 맞는지 본다. 이 프로토콜이 쓰는 JSON Schema 의 부분만 안다: `$ref`, `oneOf`, `anyOf`, `type`,
 * `const`, `enum`, `properties`, `required`, `additionalProperties`, `items`. 스키마가 실제 전송 모양과 다르면
 * (schemars 와 serde 가 어긋나면) 여기서 걸린다.
 */
fn schema_errors(schema: &Value, value: &Value, definitions: &Value, path: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(schema) = schema.as_object() else {
        return errors;
    };
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let name = reference.trim_start_matches("#/$defs/");
        match definitions.get(name) {
            Some(target) => errors.extend(schema_errors(target, value, definitions, path)),
            None => errors.push(format!("{path}: unknown definition {reference}")),
        }
    }
    for (keyword, exactly_one) in [("oneOf", true), ("anyOf", false)] {
        if let Some(branches) = schema.get(keyword).and_then(Value::as_array) {
            let matching = branches
                .iter()
                .filter(|branch| schema_errors(branch, value, definitions, path).is_empty())
                .count();
            if matching == 0 || (exactly_one && matching != 1) {
                errors.push(format!("{path}: {matching} branch(es) of {keyword} match"));
            }
        }
    }
    if let Some(expected) = schema.get("type") {
        let matches_type = |name: &str| match name {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            _ => false,
        };
        let allowed = match expected {
            Value::String(name) => matches_type(name),
            Value::Array(names) => names.iter().filter_map(Value::as_str).any(matches_type),
            _ => true,
        };
        if !allowed {
            errors.push(format!("{path}: {value} is not of type {expected}"));
        }
    }
    if let Some(constant) = schema.get("const")
        && constant != value
    {
        errors.push(format!("{path}: expected {constant}, found {value}"));
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
        && !allowed.contains(value)
    {
        errors.push(format!("{path}: {value} is not one of {allowed:?}"));
    }
    if let Some(fields) = value.as_object() {
        let properties = schema.get("properties").and_then(Value::as_object);
        for name in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !fields.contains_key(name) {
                errors.push(format!("{path}: missing `{name}`"));
            }
        }
        for (name, field) in fields {
            match properties.and_then(|properties| properties.get(name)) {
                Some(property) => errors.extend(schema_errors(
                    property,
                    field,
                    definitions,
                    &format!("{path}.{name}"),
                )),
                None if schema.get("additionalProperties") == Some(&Value::Bool(false)) => {
                    errors.push(format!("{path}: unexpected `{name}`"));
                }
                None => {}
            }
        }
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        if schema
            .get("minLength")
            .and_then(Value::as_u64)
            .is_some_and(|minimum| length < minimum)
            || schema
                .get("maxLength")
                .and_then(Value::as_u64)
                .is_some_and(|maximum| length > maximum)
        {
            errors.push(format!("{path}: the length {length} is out of range"));
        }
        // 이 프로토콜의 정규 규칙은 "공백이 아닌 글자가 있다" 하나이다. 다른 규칙이 생기면 아래 시험이 알린다.
        if schema.get("pattern").is_some() && text.trim().is_empty() {
            errors.push(format!("{path}: the text is blank"));
        }
    }
    if let (Some(items), Some(elements)) = (schema.get("items"), value.as_array()) {
        for (index, element) in elements.iter().enumerate() {
            errors.extend(schema_errors(
                items,
                element,
                definitions,
                &format!("{path}[{index}]"),
            ));
        }
    }
    errors
}

/// 스펙의 목록에서 그 이름의 항목이 가진 스키마로 값을 검사한다.
fn assert_matches_spec(spec: &Value, list: &str, field: &str, value: &Value) {
    let tag = if list == "records" { "kind" } else { "type" };
    let name = value[tag].as_str().expect("a tagged sample");
    let entry = spec[list]
        .as_array()
        .expect("a spec list")
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap_or_else(|| panic!("{list} has no `{name}`"));
    let errors = schema_errors(&entry[field], value, &spec["$defs"], name);
    assert!(
        errors.is_empty(),
        "{list} `{name}` {field}: {errors:?}\n{value}"
    );
}

/// 스키마가 쓰는 키워드를 모은다. `properties` 바로 아래의 키는 필드 이름이라서 뺀다.
fn schema_keywords(schema: &Value, keywords: &mut std::collections::BTreeSet<String>) {
    match schema {
        Value::Object(fields) => {
            for (keyword, value) in fields {
                keywords.insert(keyword.clone());
                match (keyword.as_str(), value) {
                    ("properties" | "$defs", Value::Object(named)) => {
                        named
                            .values()
                            .for_each(|schema| schema_keywords(schema, keywords));
                    }
                    ("default" | "const" | "enum", _) => {}
                    _ => schema_keywords(value, keywords),
                }
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|schema| schema_keywords(schema, keywords)),
        _ => {}
    }
}

#[test]
fn the_test_checker_knows_every_schema_keyword_of_the_spec() {
    // `schema_errors` passes over a keyword it does not know. So a new keyword in the spec must fail here first.
    let spec = spec_json();
    let mut keywords = std::collections::BTreeSet::new();
    for list in ["methods", "commands", "queries", "events", "records"] {
        for entry in spec[list].as_array().expect("a spec list") {
            for field in ["params", "result", "input", "output", "payload"] {
                schema_keywords(&entry[field], &mut keywords);
            }
        }
    }
    schema_keywords(&json!({ "$defs": spec["$defs"] }), &mut keywords);
    let known = [
        "$defs",
        "$ref",
        "additionalProperties",
        "anyOf",
        "const",
        "default",
        "enum",
        "format",
        "items",
        "maxLength",
        "minLength",
        "minimum",
        "oneOf",
        "pattern",
        "properties",
        "required",
        "type",
    ];
    let unknown = keywords
        .iter()
        .filter(|keyword| !known.contains(&keyword.as_str()))
        .collect::<Vec<_>>();
    assert!(unknown.is_empty(), "the checker does not know {unknown:?}");
    // `format` and `minimum` say only "an unsigned or sized integer". `additionalProperties` is always `false`,
    // and the only pattern is the non-blank rule.
    let text = serde_json::to_string(&spec).expect("the spec should serialize");
    assert!(!text.contains("\"additionalProperties\":{"));
    assert_eq!(
        text.matches("\"pattern\":").count(),
        text.matches("\"pattern\":\"\\\\S\"").count()
    );
}

#[test]
fn the_input_schema_carries_the_rules_the_code_checks() {
    // A value the schema accepts must not fail with "does not match the input schema".
    let spec = spec_json();
    let steer = spec["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|entry| entry["name"] == "steer_turn")
        .expect("steer_turn");
    let blank = json!({ "type": "steer_turn", "thread_id": "thread-1", "expected_turn_id": "turn-1", "prompt": "  " });
    assert_eq!(
        schema_errors(&steer["input"], &blank, &spec["$defs"], "steer_turn"),
        ["steer_turn.prompt: the text is blank"]
    );
    let command: KernelCommand = serde_json::from_value(blank).expect("serde accepts the shape");
    assert_eq!(
        command
            .into_core("/workspace")
            .expect_err("the code refuses a blank prompt")
            .code,
        ERROR_INVALID_PARAMS
    );
    let key = &spec["$defs"]["CommandRequest"]["properties"]["idempotency_key"];
    assert_eq!(
        (key["minLength"].as_u64(), key["maxLength"].as_u64()),
        (Some(1), Some(128))
    );
}

#[test]
fn a_destructive_command_needs_confirm_in_the_schema_and_in_the_code() {
    // The spec says `destructive` commands need `confirm: true`. Each such command must have the field and
    // must be refused without it. A new destructive command that forgets the check fails here.
    let spec = spec_json();
    let mut destructive = 0;
    for command in sample_commands() {
        if command.kind().spec().effect != crate::domain::kernel_spec::CommandEffect::Destructive {
            continue;
        }
        destructive += 1;
        let name = command.kind().name();
        let entry = spec["commands"]
            .as_array()
            .expect("commands")
            .iter()
            .find(|entry| entry["name"] == name)
            .expect("a command entry");
        assert_eq!(
            entry["input"]["properties"]["confirm"]["type"], "boolean",
            "{name}"
        );
        let mut unconfirmed = serde_json::to_value(&command).expect("command should serialize");
        unconfirmed["confirm"] = json!(false);
        let unconfirmed: KernelCommand =
            serde_json::from_value(unconfirmed).expect("the unconfirmed command should parse");
        assert_eq!(
            unconfirmed
                .into_core("/workspace")
                .expect_err("{name} must need confirm")
                .code,
            ERROR_CONFIRMATION_REQUIRED,
            "{name}"
        );
    }
    assert_eq!(
        destructive, 1,
        "reset_planning is the destructive command today"
    );
}

/// 값이 채워진 기록 표본이다. 종류마다 하나이다.
fn sample_records() -> Vec<crate::domain::kernel_event::KernelEvent> {
    use crate::domain::kernel_event::{
        ApprovalRequestKind, ApprovalResolution, KernelEvent as Record, PendingApproval,
        PostTurnRoute, QueuedThreadPrompt, ThreadPromptRejection, ThreadPromptSettlement,
        TurnEndOutcome, TurnOrigin,
    };
    let queued = QueuedThreadPrompt {
        thread_id: "thread-1".to_string(),
        workspace_directory: "/workspace".to_string(),
        handoff: crate::domain::planning::ManualPromptMainSessionHandoff {
            prompt: "planned prompt".to_string(),
            transcript_text: "fix the test".to_string(),
            task: None,
        },
        task_id: "task-1".to_string(),
    };
    let approval = PendingApproval::new(
        "thread-1",
        Some("turn-1"),
        "approval-1",
        ApprovalRequestKind::FileChange,
        "File change requested.",
    );
    vec![
        queued.settled(ThreadPromptSettlement::TaskFinished),
        Record::ThreadPromptQueued(queued),
        Record::ThreadPromptRejected {
            thread_id: "thread-1".to_string(),
            reason: ThreadPromptRejection::ParallelMode,
        },
        Record::TurnSubmitted {
            thread_id: String::new(),
            submission: 3,
            origin: TurnOrigin::AutoFollow,
        },
        Record::TurnStarted {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
        },
        Record::TurnEnded {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            outcome: Some(TurnEndOutcome::Interrupted),
        },
        Record::TurnEnded {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-2".to_string(),
            outcome: None,
        },
        Record::PostTurnRouted {
            thread_id: "thread-1".to_string(),
            completed_turn_id: "turn-1".to_string(),
            route: PostTurnRoute::ParallelConsumed,
        },
        approval.resolved(ApprovalResolution::TimedOut),
        Record::ApprovalRequested(approval),
        Record::CommandReceived {
            op_id: 7,
            command: "steer_turn".to_string(),
            thread_id: "thread-1".to_string(),
            fingerprint: "0123".to_string(),
            idempotency_key: Some("key-1".to_string()),
        },
        Record::CommandRejected {
            op_id: 7,
            thread_id: "thread-1".to_string(),
            reason: CommandRejectionReason::Busy,
        },
    ]
}

#[test]
fn real_wire_values_match_the_schemas_of_the_spec() {
    let spec = spec_json();
    let to_json = |value: &dyn erased::Json| value.json();
    let matches_type = |type_name: &str, value: Value| {
        let errors = schema_errors(
            &json!({ "$ref": format!("#/$defs/{type_name}") }),
            &value,
            &spec["$defs"],
            type_name,
        );
        assert!(errors.is_empty(), "{type_name}: {errors:?}\n{value}");
    };
    for command in sample_commands() {
        assert_matches_spec(&spec, "commands", "input", &to_json(&command));
        matches_type(
            "CommandRequest",
            to_json(&CommandRequest {
                command,
                idempotency_key: Some("key-1".to_string()),
            }),
        );
    }
    let snapshot = crate::core::app::AppSnapshot::initial();
    let record = |sequence, event| crate::domain::kernel_event::RecordedKernelEvent {
        sequence,
        recorded_at: "2026-11-11T00:00:00Z".to_string(),
        event,
    };
    let records = sample_records();
    let mut kinds = std::collections::BTreeSet::new();
    for event in &records {
        kinds.insert(event.kind());
        assert_matches_spec(&spec, "records", "payload", &to_json(event));
    }
    assert_eq!(
        kinds.len(),
        spec["records"].as_array().expect("records").len()
    );
    for query in [
        KernelQuery::Sessions {},
        KernelQuery::AutoFollow {},
        KernelQuery::ThreadActivity {},
        KernelQuery::Planning {},
        KernelQuery::Parallel {},
        KernelQuery::History {
            after: Some(3),
            limit: None,
        },
    ] {
        assert_matches_spec(&spec, "queries", "input", &to_json(&query));
        matches_type("QueryRequest", to_json(&QueryRequest { query }));
        let page = crate::domain::kernel_event::KernelEventPage {
            records: records
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, event)| record(index as i64 + 1, event))
                .collect(),
            next_after: 12,
        };
        let answer = query.answer(&snapshot, &[], no_parallel(), Some(page));
        assert_matches_spec(&spec, "queries", "output", &to_json(&answer));
    }
    // 값이 채워진 상태이다. 빈 목록만 보내면 중첩된 타입의 스키마를 검사하지 못한다.
    let task = PlanningTask {
        rank: 1,
        task_id: "task-1".to_string(),
        direction_title: "Direction".to_string(),
        title: "Task".to_string(),
        status: PlanningTaskStatus::Ready,
        priority: 3,
    };
    let populated = [
        KernelEvent::Resync,
        KernelEvent::Recorded(record(9, records[0].clone())),
        KernelEvent::PromptSubmission(PromptSubmissionState {
            thread_id: "thread-1".to_string(),
            status: PromptSubmissionStatus::Queued,
            reason: Some(PromptSubmissionReason::TurnStartedDuringPreparation),
            task_id: Some("task-1".to_string()),
            message: Some("another turn started".to_string()),
        }),
        KernelEvent::Sessions(SessionsState {
            status: SessionsStatus::Ready,
            sessions: vec![SessionEntry {
                thread_id: "thread-1".to_string(),
                title: "Session".to_string(),
                workspace: "/workspace".to_string(),
                updated_at_epoch: 1_700_000_000,
            }],
            message: Some("loaded".to_string()),
        }),
        KernelEvent::AutoFollow(AutoFollowSettings {
            max_turns: 3,
            paused: true,
        }),
        KernelEvent::ThreadActivity {
            threads: vec![ThreadActivity {
                thread_id: "thread-1".to_string(),
                running: true,
                active_turn_id: Some("turn-1".to_string()),
                approval: Some(ApprovalRequest {
                    approval_id: "approval-1".to_string(),
                    server_request_id: "7".to_string(),
                    summary: "Command execution requested.".to_string(),
                    details: vec!["cargo test".to_string()],
                }),
                auto_follow_active: false,
            }],
        },
        KernelEvent::Planning(PlanningState {
            status: PlanningStatus::ReadyWithTask,
            revision: Some(4),
            queue_summary: Some("1 ready".to_string()),
            failure_reason: None,
            auto_follow_pause_reason: Some("paused".to_string()),
            next_task: Some(task.clone()),
            active_tasks: vec![task.clone()],
            proposed_tasks: vec![task],
            skipped_tasks: vec![SkippedPlanningTask {
                task_id: "task-2".to_string(),
                title: "Skipped".to_string(),
                status: PlanningTaskStatus::Blocked,
                reason: "blocked".to_string(),
            }],
        }),
        KernelEvent::Parallel(ParallelState {
            enabled: true,
            status: Some("running".to_string()),
            supervisor: Some(ParallelSupervisor {
                state: ParallelSupervisorState::Supervise,
                pool: ParallelPool {
                    configured: 2,
                    idle: 1,
                    leased: 0,
                    running: 1,
                    awaiting_cleanup: 0,
                    blocked: 0,
                    missing: 0,
                    unavailable: 0,
                    exhausted: false,
                },
                agents: vec![ParallelAgent {
                    agent_id: "agent-1".to_string(),
                    thread_id: Some("thread-2".to_string()),
                    task_title: "Task".to_string(),
                    slot_id: "slot-1".to_string(),
                    branch: "akra-agent/slot-1".to_string(),
                    state: "running".to_string(),
                    duration: "1m".to_string(),
                    summary: "working".to_string(),
                }],
                merge_head: "abc123".to_string(),
                merge_queue: vec![ParallelMergeItem {
                    task_title: "Task".to_string(),
                    state: "queued".to_string(),
                    branch: "akra-agent/slot-1".to_string(),
                    commit: "abc123".to_string(),
                }],
                pr_validation: Some(ParallelPrValidation {
                    pull_request_number: 12,
                    url: "https://example.invalid/pull/12".to_string(),
                    state: "waiting".to_string(),
                    phase: "checks".to_string(),
                    reason: "checks pending".to_string(),
                    blocker: Some("ci".to_string()),
                }),
                notice: Some("notice".to_string()),
            }),
        }),
    ];
    let mut event_names = std::collections::BTreeSet::new();
    for event in populated {
        let value = to_json(&event);
        event_names.insert(value["type"].as_str().expect("an event type").to_string());
        assert_matches_spec(&spec, "events", "payload", &value);
        matches_type("EventNotification", to_json(&EventNotification { event }));
    }
    assert_eq!(
        event_names.len(),
        spec["events"].as_array().expect("events").len()
    );
    matches_type(
        "AttachRequest",
        json!({ "workspace": "/workspace", "client_name": "akra-desktop" }),
    );
    matches_type(
        "AttachResponse",
        json!({ "client_id": "1", "protocol_version": 1, "kernel": "repository" }),
    );
    // 명령의 답: 받아들인 영수증과, 영수증이 있는 거절.
    for rejection in [None, Some(CommandRejectionReason::Busy)] {
        let answer = CommandResponse::from_core(Some(&[])).with_operation(Some(
            crate::domain::kernel_event::OperationReceipt {
                op_id: 5,
                rejection,
                replayed: rejection.is_some(),
            },
        ));
        matches_type("CommandResponse", to_json(&answer));
    }
    // 검사기가 틀린 값을 잡는지 본다. 잡지 못하면 위의 확인은 뜻이 없다.
    let wrong =
        json!({ "type": "steer_turn", "thread_id": "thread-1", "prompt": 3, "extra": true });
    let entry = spec["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|entry| entry["name"] == "steer_turn")
        .expect("steer_turn");
    let errors = schema_errors(&entry["input"], &wrong, &spec["$defs"], "steer_turn");
    assert_eq!(errors.len(), 3, "{errors:?}");
}

mod erased {
    /// 직렬화할 수 있는 값을 JSON 으로 바꾼다. 여러 타입의 표본을 한 반복문에서 쓰기 위한 것이다.
    pub(super) trait Json {
        fn json(&self) -> serde_json::Value;
    }

    impl<T: serde::Serialize> Json for T {
        fn json(&self) -> serde_json::Value {
            serde_json::to_value(self).expect("a sample should serialize")
        }
    }
}
