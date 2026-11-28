use super::*;
use serde_json::{Value, json};

#[test]
fn attach_defaults_to_durable_and_accepts_explicit_transient_clients() {
    let mut params = json!({"workspace": "/repo", "client_name": "test"});
    let durable: AttachRequest = serde_json::from_value(params.clone()).unwrap();
    assert!(!durable.transient);
    params["transient"] = json!(true);
    let transient: AttachRequest = serde_json::from_value(params).unwrap();
    assert!(transient.transient);
    assert_eq!(serde_json::to_value(transient).unwrap()["transient"], true);
}

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
        KernelCommand::EnableParallel {},
        KernelCommand::DisableParallel {},
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
        KernelCommand::RequestTaskVerification {
            request_id: "host-verification-1".into(),
            task_id: "task-1".into(),
            criterion_id: "criterion-1".into(),
            scope_id: "a".repeat(64),
            expected_acceptance_revision: 3,
        },
        KernelCommand::PrepareEvidence {
            request_id: "prepare-1".into(),
            task_id: "task-1".into(),
            criterion_id: "criterion 1".into(),
            scope_id: "a".repeat(64),
            expected_acceptance_revision: 3,
        },
        KernelCommand::RecordEvidence {
            attempt_id: "b".repeat(64),
            status: TaskEvidenceStatus::Pass,
            note: None,
        },
        KernelCommand::WaiveCriterion {
            request_id: "waive-1".into(),
            task_id: "task-1".into(),
            criterion_id: "criterion 1".into(),
            scope_id: "a".repeat(64),
            expected_acceptance_revision: 3,
            reason: "Agreed exemption".into(),
        },
        KernelCommand::CreateTask {
            task: sample_task_fields(),
        },
        KernelCommand::UpdateTask {
            task_id: "task-1".to_string(),
            task: sample_task_fields(),
            expected_planning_revision: 7,
        },
        KernelCommand::DeleteTask {
            task_id: "task-1".to_string(),
            expected_planning_revision: 7,
            confirm: true,
        },
        KernelCommand::CreateDirection {
            direction: sample_direction_fields(),
        },
        KernelCommand::UpdateDirection {
            direction_id: "direction-1".to_string(),
            direction: sample_direction_fields(),
            expected_planning_revision: 7,
        },
        KernelCommand::DeleteDirection {
            direction_id: "direction-1".to_string(),
            expected_planning_revision: 7,
            confirm: true,
        },
        KernelCommand::ExportPlanningFiles {},
        KernelCommand::ApplyPlanningFiles {},
        KernelCommand::PromoteDraft {
            draft_name: "draft-1".to_string(),
            kind: crate::application::port::inbound::planning_admin_port::PlanningAdminDraftKind::DirectionDetail,
            direction_id: Some("direction-1".to_string()),
            expected_planning_revision: 7,
        },
        KernelCommand::SaveParallelAgentProfiles {
            profiles: crate::domain::parallel_agent_profile::ParallelAgentProfileConfig::default()
                .profiles,
        },
        KernelCommand::SetIssueAutomationMode {
            mode: IssueAutomationMode::Observe,
        },
        KernelCommand::SetIssueAutonomy {
            command_id: "autonomy-1".to_owned(),
            level: IssueAutonomyLevel::Propose,
        },
        KernelCommand::SetIssueBindingPaused {
            command_id: "pause-1".into(),
            binding_id: "binding-1".into(),
            paused: true,
            expected_revision: 0,
        },
        KernelCommand::RetryIssueAccountQuota {
            lane: crate::domain::issue_automation::collection::IssueQuotaLane::Start,
            expected_revision: 7,
        },
        KernelCommand::AnswerIssueDecision {
            criteria: None,
            decision_id: "decision-1".to_owned(),
            option: "continue".to_owned(),
            note: None,
        },
        serde_json::from_value(json!({
            "type": "ask_issue_decision",
            "branch": "akra-agent/slot-1/task",
            "question": "A or B?",
            "options": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}],
        }))
        .unwrap(),
        KernelCommand::HoldIssue {
            issue: (&sample_issue_reference()).into(),
            command_id: "intervention-1".into(),
        },
        KernelCommand::ReleaseIssue {
            issue: (&sample_issue_reference()).into(),
            command_id: "intervention-1".into(),
        },
        KernelCommand::SteerIssue {
            issue: (&sample_issue_reference()).into(),
            command_id: "intervention-1".into(),
            message: "Focus on tests".into(),
        },
        KernelCommand::TakeOverIssue {
            issue: (&sample_issue_reference()).into(),
            command_id: "intervention-1".into(),
        },
        KernelCommand::RetriageIssue {
            issue: (&sample_issue_reference()).into(),
            command_id: "intervention-1".into(),
        },
        KernelCommand::CancelIssueWork {
            issue: (&sample_issue_reference()).into(),
            command_id: "intervention-1".into(),
            confirm: true,
        },
        KernelCommand::TrackIssue {
            issue: (&sample_issue_reference()).into(),
            command_id: "intervention-1".into(),
        },
        KernelCommand::UntrackIssue {
            issue: (&sample_issue_reference()).into(),
            command_id: "intervention-1".into(),
        },
        KernelCommand::SyncIssues {},
        KernelCommand::SubmitDelegation {
            instruction: "Explain the test result.".into(),
            idempotency_key: "delegation-request-1".into(),
            session: Default::default(),
            task_id: None,
            parent_callback: None,
        },
        KernelCommand::CancelDelegation { id: "job-1".into() },
        KernelCommand::CheckpointDelegation {
            id: "job-1".into(),
            token: "fixture".into(),
            message: "In progress".into(),
        },
        KernelCommand::ReportDelegationResult {
            id: "job-1".into(),
            token: "fixture".into(),
            result: "Finished".into(),
        },
        KernelCommand::RetryDelegation { id: "job-1".into() },
    ]
}

fn sample_direction_fields() -> DirectionFields {
    DirectionFields {
        title: "Release".to_string(),
        summary: Some("Ship the release.".to_string()),
        success_criteria: vec!["The release is out.".to_string()],
        scope_hints: Vec::new(),
        detail_doc_path: None,
        state: Some(PlanningDirectionState::Paused),
    }
}

fn sample_task_fields() -> TaskFields {
    TaskFields {
        direction_id: Some("direction-1".to_string()),
        title: "Fix the failing test".to_string(),
        description: Some("The test fails on macOS.".to_string()),
        acceptance: None,
        status: Some(PlanningTaskStatus::Ready),
        base_priority: Some(3),
        dynamic_priority_delta: None,
        priority_reason: None,
        depends_on: vec!["task-0".to_string()],
        blocked_by: Vec::new(),
    }
}

fn no_parallel() -> ParallelSource<'static> {
    ParallelSource {
        enabled: false,
        supervisor: None,
        status: None,
    }
}

#[test]
fn delegation_callbacks_and_retry_use_the_ledger_without_history_descriptors() {
    for command in sample_commands().into_iter().filter(|command| {
        matches!(
            command.kind(),
            KernelCommandKind::CheckpointDelegation
                | KernelCommandKind::ReportDelegationResult
                | KernelCommandKind::RetryDelegation
        )
    }) {
        if command.kind() == KernelCommandKind::RetryDelegation {
            assert!(
                command_request(serde_json::to_value(&command).unwrap(), None)
                    .descriptor()
                    .is_err()
            );
        }
        let request = CommandRequest {
            command,
            idempotency_key: Some("fixture-key".to_owned()),
        };
        assert_eq!(request.descriptor().unwrap(), None);
    }
}

/*
 * The places where the spec carries the schema keyword `description`. schemars writes Rust doc comments there.
 * A key directly under `properties` is a field name (a task has a field `description`), not the keyword.
 */
fn code_comment_keys(value: &Value, under_properties: bool) -> Vec<String> {
    match value {
        Value::Object(fields) => fields
            .iter()
            .flat_map(|(key, child)| {
                let mut found = code_comment_keys(child, key == "properties");
                if key == "description" && !under_properties {
                    found.push(child.to_string());
                }
                found
            })
            .collect(),
        Value::Array(items) => items
            .iter()
            .flat_map(|item| code_comment_keys(item, false))
            .collect(),
        _ => Vec::new(),
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
fn every_command_maps_to_its_own_execution_boundary() {
    for command in sample_commands() {
        let label = format!("{command:?}");
        let kind = command.kind();
        let input = command
            .into_input("/workspace")
            .unwrap_or_else(|error| panic!("{label} should map: {error:?}"));
        match input {
            KernelCommandInput::Core(core) => assert!(
                !matches!(core, AppCommand::Noop),
                "{label} must not map to Noop"
            ),
            // plan 18 K4-9d: a stored command does not pass the core. It keeps its kind and has a result shape.
            KernelCommandInput::Stored(stored) => {
                assert_eq!(stored.kind(), kind, "{label}");
                assert!(CommandResult::name_for(kind).is_some(), "{label}");
            }
            KernelCommandInput::SyncIssues => assert_eq!(kind, KernelCommandKind::SyncIssues),
            KernelCommandInput::Parallel {
                workspace_directory,
                enabled,
            } => {
                assert_eq!(workspace_directory, "/workspace");
                assert_eq!(
                    kind,
                    if enabled {
                        KernelCommandKind::EnableParallel
                    } else {
                        KernelCommandKind::DisableParallel
                    }
                );
                assert_eq!(
                    kind.spec().state,
                    crate::domain::kernel_spec::CommandState::Runtime
                );
                assert_eq!(
                    kind.spec().completion,
                    crate::domain::kernel_spec::CommandCompletion::Accepted
                );
                assert_eq!(
                    kind.spec().scope,
                    crate::domain::kernel_spec::CommandScope::Parallel
                );
                assert!(CommandResult::name_for(kind).is_none());
            }
            KernelCommandInput::SubmitDelegation { .. } => {
                assert_eq!(kind, KernelCommandKind::SubmitDelegation)
            }
            KernelCommandInput::CancelDelegation { .. } => {
                assert_eq!(kind, KernelCommandKind::CancelDelegation)
            }
            KernelCommandInput::CheckpointDelegation { .. } => {
                assert_eq!(kind, KernelCommandKind::CheckpointDelegation)
            }
            KernelCommandInput::ReportDelegationResult { .. } => {
                assert_eq!(kind, KernelCommandKind::ReportDelegationResult)
            }
            KernelCommandInput::RetryDelegation { .. } => {
                assert_eq!(kind, KernelCommandKind::RetryDelegation)
            }
        }
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
                op_id: None,
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
fn parallel_control_uses_only_the_attached_workspace_and_cannot_carry_authority() {
    for (kind, enabled) in [("enable_parallel", true), ("disable_parallel", false)] {
        let command: KernelCommand = serde_json::from_value(json!({"type": kind})).unwrap();
        let KernelCommandInput::Parallel {
            workspace_directory,
            enabled: requested,
        } = command.into_input("/attached/repository").unwrap()
        else {
            panic!("parallel control must use its runtime boundary");
        };
        assert_eq!(workspace_directory, "/attached/repository");
        assert_eq!(requested, enabled);
        // The attached Gateway determines the repository. These commands cannot supply another
        // workspace, a human approval, a delivery permit, or public-repository opt-in.
        for field in [
            "workspace",
            "workspace_directory",
            "confirmed",
            "human_confirmation",
            "delivery_permit",
            "allow_public_repository",
        ] {
            let mut payload = json!({"type": kind});
            payload[field] = json!("untrusted override");
            assert!(
                serde_json::from_value::<KernelCommand>(payload).is_err(),
                "{kind} must reject {field}"
            );
        }
    }
}

#[test]
fn reset_requires_confirmation_and_blank_ids_are_rejected() {
    let unconfirmed: KernelCommand =
        serde_json::from_value(json!({"type": "reset_planning", "target": "queue"}))
            .expect("confirm defaults to false");
    // plan 18 K4-5: the adapter does not refuse the command. It tells the kernel whether the client
    // confirmed, and the kernel refuses a destructive command without it.
    assert!(!unconfirmed.confirmed());
    let described = CommandRequest {
        command: unconfirmed,
        idempotency_key: None,
    }
    .descriptor()
    .expect("the command is valid")
    .expect("a destructive command is recorded");
    assert!(!described.confirmed);
    assert_eq!(described.caller, None, "the transport adds the caller");
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
        rejection: Some(ReasonCode::NoMatchingTurn),
        replayed: true,
    });
    assert_eq!(
        serde_json::to_value(replayed).expect("response should serialize"),
        json!({
            "processed": true,
            "rejection": {
                "reason": "no_matching_turn",
                "message": ReasonCode::NoMatchingTurn.summary(),
                "next": [{ "step": "query", "name": "thread_activity" }],
            },
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
        rejection: Some(ReasonCode::Busy),
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
    // plan 18 K4-3: the reason, its sentence, and its next steps all come from the reason table.
    assert_eq!(
        KernelEvent::from_core(&AppEvent::ThreadPromptAdmissionResolved {
            thread_id: "thread-2".to_string(),
            admission: ThreadPromptAdmission::RejectedUnknownThread,
        }),
        Some(KernelEvent::PromptSubmission(PromptSubmissionState {
            thread_id: "thread-2".to_string(),
            status: PromptSubmissionStatus::Rejected,
            reason: Some(ReasonCode::UnknownThread),
            next: vec![NextStep::AskUser],
            task_id: None,
            message: Some(ReasonCode::UnknownThread.summary().to_string()),
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
            reason: Some(ReasonCode::PlanningRefused),
            next: vec![NextStep::AskUser],
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
            reason: Some(ReasonCode::Unavailable),
            next: vec![NextStep::WaitFor {
                event: "prompt_submission".to_string()
            }],
            task_id: Some("task-7".to_string()),
            message: Some(ReasonCode::Unavailable.summary().to_string()),
        }))
    );
    let status_and_reason = |event: &AppEvent| {
        KernelEvent::from_core(event).and_then(|event| match event {
            KernelEvent::PromptSubmission(state) => Some((state.status, state.reason)),
            _ => None,
        })
    };
    assert_eq!(
        status_and_reason(&AppEvent::ThreadPromptSubmitted {
            thread_id: "thread-2".to_string(),
            outcome: ThreadPromptOutcome::ParallelModeStarted {
                task_id: "task-7".to_string(),
            },
        }),
        Some((
            PromptSubmissionStatus::Queued,
            Some(ReasonCode::ParallelModeStarted)
        ))
    );
    assert_eq!(
        status_and_reason(&AppEvent::ThreadPromptAdmissionResolved {
            thread_id: "thread-2".to_string(),
            admission: ThreadPromptAdmission::RejectedParallelModeUnknown,
        }),
        Some((
            PromptSubmissionStatus::Rejected,
            Some(ReasonCode::ParallelModeUnknown)
        ))
    );
    // plan 18 K4-3: a queued prompt that the kernel drops is no longer silent.
    assert_eq!(
        status_and_reason(&AppEvent::ThreadPromptSubmitted {
            thread_id: "thread-2".to_string(),
            outcome: ThreadPromptOutcome::QueuedTaskFinished {
                task_id: "task-7".to_string(),
            },
        }),
        Some((
            PromptSubmissionStatus::Dropped,
            Some(ReasonCode::TaskFinished)
        ))
    );
    assert_eq!(
        status_and_reason(&AppEvent::ThreadPromptSubmitted {
            thread_id: "thread-2".to_string(),
            outcome: ThreadPromptOutcome::QueuedRejectedAgain {
                task_id: "task-7".to_string(),
            },
        }),
        Some((
            PromptSubmissionStatus::Dropped,
            Some(ReasonCode::RejectedAgain)
        ))
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
            next: Vec::new(),
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
            "akra/kernel/confirmation/register",
            "akra/kernel/confirmation/prepare",
            "akra/kernel/command_confirmed",
            "akra/kernel/query",
            "akra/kernel/event",
            "akra/kernel/extensions/attach",
            "akra/kernel/extensions/detach",
            "akra/kernel/extensions/list",
            "akra/kernel/extensions/call",
            "akra/kernel/extensions/changed",
            "akra/kernel/detach"
        ])
    );
    let queries = describe().queries;
    assert_eq!(queries.len(), 23);
    for (name, query) in queries.iter().map(|query| query.name).zip([
        KernelQuery::Delegations {},
        KernelQuery::Delegation { id: "job-1".into() },
        KernelQuery::DelegationResult { id: "job-1".into() },
        KernelQuery::DelegationBackends {},
        KernelQuery::DelegationMyTurn {},
        KernelQuery::FeatureBindings {},
        KernelQuery::PrReviewResult { number: 42 },
        KernelQuery::Sessions {},
        KernelQuery::AutoFollow {},
        KernelQuery::ThreadActivity {},
        KernelQuery::Planning {},
        KernelQuery::ListTasks {},
        KernelQuery::TaskResult {
            task_id: "task-1".into(),
        },
        KernelQuery::TaskUsage {
            task_id: "task-1".into(),
        },
        KernelQuery::TaskEvidence {
            task_ids: None,
            after_task_id: None,
            limit: None,
        },
        KernelQuery::Parallel {},
        KernelQuery::Attention {},
        KernelQuery::History {
            after: None,
            limit: None,
        },
        KernelQuery::Operation { op_id: 7 },
        KernelQuery::IssueAutomation {},
        KernelQuery::IssueAutomationPolicy {},
        KernelQuery::Issues {
            after: None,
            limit: None,
        },
        KernelQuery::Issue {
            issue: (&sample_issue_reference()).into(),
        },
    ]) {
        assert_eq!(
            serde_json::to_value(query).expect("query should serialize")["type"],
            json!(name)
        );
    }
    let events = describe().events;
    assert_eq!(events.len(), 10);
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
            next: Vec::new(),
            task_id: None,
            message: None,
        }),
        KernelEvent::Planning(PlanningState::from_core(&RuntimeProjection::uninitialized())),
        KernelEvent::Parallel(ParallelState::from_core(no_parallel())),
        KernelEvent::Resync,
        KernelEvent::Recorded(RecordedKernelEvent {
            sequence: 1,
            recorded_at: "2026-11-20T00:00:00Z".to_string(),
            event: sample_records().into_iter().next().expect("record fixture"),
        }),
        KernelEvent::Issues(IssueAutomationState::unavailable()),
        KernelEvent::IssueDecision(IssueDecisionChangedState::from_fact(
            &sample_issue_decision_fact(),
            5,
        )),
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
            result: None,
            failure: None,
        }
    );
    assert_eq!(
        CommandResponse::from_core(Some(&[])),
        CommandResponse {
            processed: true,
            rejection: None,
            operation: None,
            result: None,
            failure: None,
        }
    );
    let refused = CommandResponse::from_core(Some(&[AppEvent::TurnSteerAdmissionResolved(
        crate::core::app::TurnSteerAdmission::RejectedUnavailable,
    )]));
    assert_eq!(
        refused.rejection.map(|rejection| rejection.reason),
        Some(ReasonCode::NoMatchingTurn)
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
                "message": ReasonCode::NoMatchingApproval.summary(),
                "next": [{ "step": "query", "name": "thread_activity" }],
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
            op_id: None,
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
                op_id: None,
            },
        }],
        next_after: 42,
        recorder: None,
    };
    let snapshot = crate::core::app::AppSnapshot::initial();
    let answer = next
        .query
        .answer(&snapshot, &[], no_parallel(), Some(page.clone()), None);
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
        first
            .query
            .answer(&snapshot, &[], no_parallel(), None, None),
        KernelQueryAnswer::History(crate::domain::kernel_event::KernelEventPage {
            records: Vec::new(),
            next_after: 0,
            recorder: None,
        }),
        "a kernel without a log answers with an empty page"
    );
}

#[test]
fn an_operation_query_answers_with_the_state_and_the_records_of_one_command() {
    // plan 18 K4-4: the kernel computes the state from the records of that operation.
    use crate::domain::kernel_event::{KernelEvent as Record, OperationStatus, TurnEndOutcome};
    let request: QueryRequest =
        serde_json::from_value(json!({ "query": { "type": "operation", "op_id": 7 } }))
            .expect("an operation query should parse");
    assert_eq!(request.query.operation_id(), Some(7));
    assert_eq!(request.query.history_range(), None);
    assert_eq!(KernelQuery::Planning {}.operation_id(), None);

    let record = |sequence, event| crate::domain::kernel_event::RecordedKernelEvent {
        sequence,
        recorded_at: "2026-11-12T00:00:00Z".to_string(),
        event,
    };
    let status = OperationStatus::from_records(
        7,
        [
            record(
                7,
                Record::CommandReceived {
                    op_id: 7,
                    command: "submit_prompt".to_string(),
                    thread_id: "thread-1".to_string(),
                    fingerprint: "0123".to_string(),
                    idempotency_key: None,
                    completion: Some(crate::domain::kernel_spec::CommandCompletion::Records),
                    caller: None,
                },
            ),
            // A record of another turn is not part of this operation.
            record(
                8,
                Record::TurnStarted {
                    thread_id: "thread-2".to_string(),
                    turn_id: "turn-other".to_string(),
                    op_id: None,
                },
            ),
            record(
                9,
                Record::TurnEnded {
                    thread_id: "thread-1".to_string(),
                    turn_id: "turn-1".to_string(),
                    outcome: Some(TurnEndOutcome::Completed),
                    op_id: Some(7),
                },
            ),
        ],
    );
    let snapshot = crate::core::app::AppSnapshot::initial();
    let answer = request
        .query
        .clone()
        .answer(&snapshot, &[], no_parallel(), None, Some(status));
    let answer = serde_json::to_value(&answer).expect("the answer should serialize");
    assert_eq!(answer["type"], json!("operation"));
    assert_eq!(answer["op_id"], json!(7));
    assert_eq!(answer["state"], json!("done"));
    assert_eq!(
        answer["records"]
            .as_array()
            .expect("records")
            .iter()
            .map(|record| record["sequence"].as_i64())
            .collect::<Vec<_>>(),
        [Some(7), Some(9)]
    );
    assert_eq!(answer["records"][1]["event"]["op_id"], json!(7));
    assert_eq!(
        serde_json::to_value(
            request
                .query
                .answer(&snapshot, &[], no_parallel(), None, None)
        )
        .expect("the answer should serialize"),
        json!({ "type": "operation", "op_id": 7, "state": "unknown", "records": [] }),
        "a kernel without a log knows no operation"
    );

    // The spec tells when each command ends and what each state means.
    let spec = spec_json();
    let completion = |name: &str| {
        spec["commands"]
            .as_array()
            .expect("commands")
            .iter()
            .find(|command| command["name"] == json!(name))
            .map(|command| command["completion"].clone())
    };
    assert_eq!(completion("submit_prompt"), Some(json!("records")));
    assert_eq!(completion("pause_auto_follow"), Some(json!("accepted")));
    // plan 18 K4-4b: every recorded command that does not end at once ends with a record.
    for command in [
        "steer_turn",
        "stop_all",
        "rename_session",
        "decide_approval",
        "reset_planning",
    ] {
        assert_eq!(completion(command), Some(json!("records")), "{command}");
    }
    assert_eq!(
        names_of(&spec["terms"]["operation_state"]),
        [
            "accepted", "running", "done", "rejected", "failed", "lost", "unknown"
        ]
    );
    assert_eq!(
        names_of(&spec["terms"]["completion"]),
        ["accepted", "records", "unreported"]
    );
}

#[test]
fn a_query_answers_with_the_event_of_the_same_name() {
    let snapshot = crate::core::app::AppSnapshot::initial();
    let request: QueryRequest =
        serde_json::from_value(json!({ "query": { "type": "auto_follow" } }))
            .expect("query request should parse");
    assert_eq!(
        request
            .query
            .answer(&snapshot, &[], no_parallel(), None, None),
        KernelQueryAnswer::AutoFollow(AutoFollowSettings::from_core(
            &snapshot.conversation_runtime
        ))
    );
    assert_eq!(
        KernelQuery::ThreadActivity {}.answer(&snapshot, &[], no_parallel(), None, None),
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

#[test]
fn the_attention_query_reads_the_snapshot_sources() {
    let mut snapshot = crate::core::app::AppSnapshot::initial();
    assert_eq!(
        KernelQuery::Attention {}.answer(&snapshot, &[], no_parallel(), None, None),
        KernelQueryAnswer::Attention { items: Vec::new() }
    );
    let proposed = queue_task(1, "task-3", TaskStatus::Proposed);
    *snapshot.planning_parallel.planning_runtime = RuntimeProjection::ready_with_queue_projection(
        "fragment".to_string(),
        "1 task".to_string(),
        None,
        None,
        crate::domain::planning::PriorityQueueProjection {
            next_task: None,
            active_tasks: Vec::new(),
            proposed_tasks: vec![proposed],
            skipped_tasks: Vec::new(),
        },
    );
    let threads = [ThreadActivitySummary {
        thread_id: "thread-1".to_string(),
        running: true,
        active_turn_id: None,
        approval: Some(crate::domain::conversation::ConversationApprovalRequest {
            approval_id: "approval-1".to_string(),
            server_request_id: "request-1".to_string(),
            method: "item/fileChange/requestApproval".to_string(),
            kind: crate::domain::conversation::ConversationApprovalRequestKind::FileChange,
            summary: "Apply the patch".to_string(),
            details: Vec::new(),
        }),
        auto_follow_active: false,
    }];

    let KernelQueryAnswer::Attention { items } =
        KernelQuery::Attention {}.answer(&snapshot, &threads, no_parallel(), None, None)
    else {
        panic!("attention answers with attention");
    };

    let ids: Vec<_> = items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "thread:thread-1:approval-1",
            "task:task-3:2026-11-10T00:00:00Z"
        ]
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
    for reason in ReasonCode::ALL {
        assert_eq!(
            serde_json::to_value(reason).expect("a reason should serialize"),
            json!(reason.name()),
            "the serde name and the table name of a reason differ"
        );
    }
    assert_eq!(
        names_of(&spec["reasons"]),
        spec["$defs"]["ReasonCode"]["enum"]
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
fn every_reason_and_error_tells_the_next_steps_by_name() {
    // plan 18 K4-3: `next` is data. Each step names an event, query, or command of the spec.
    let spec = spec_json();
    let names = |section: &str| names_of(&spec[section]);
    let (events, queries, commands) = (names("events"), names("queries"), names("commands"));
    let methods = names("methods");
    for entry in spec["reasons"]
        .as_array()
        .expect("reasons")
        .iter()
        .chain(spec["errors"].as_array().expect("errors"))
    {
        let steps = entry["next"].as_array().expect("every entry has `next`");
        assert!(!steps.is_empty(), "{} has no next step", entry["name"]);
        for step in steps {
            let named = |field: &str| step[field].as_str().expect("a named step").to_string();
            match step["step"].as_str().expect("a step kind") {
                "wait_for" => assert!(events.contains(&named("event")), "{step}"),
                "query" => assert!(queries.contains(&named("name")), "{step}"),
                "command" => assert!(commands.contains(&named("name")), "{step}"),
                "method" => assert!(methods.contains(&named("name")), "{step}"),
                "wait" => assert!(step["milliseconds"].as_u64().is_some(), "{step}"),
                "resend" | "ask_user" | "stop" => {}
                other => panic!("unknown step {other}"),
            }
        }
    }
    assert_eq!(
        serde_json::to_value(reason_next(ReasonCode::Busy)).expect("steps should serialize"),
        json!([{ "step": "wait_for", "event": "thread_activity" }, { "step": "resend" }])
    );
    // A rejected prompt also sends `prompt_submission`. A step that waits for that event would end at once.
    for reason in KernelCommandKind::SubmitPrompt.spec().rejections {
        assert!(
            !reason_next(*reason).contains(&NextStep::WaitFor {
                event: "prompt_submission".to_string()
            }),
            "{}",
            reason.name()
        );
    }
    assert_eq!(
        serde_json::to_value(error_next(KernelErrorKind::NotAttached))
            .expect("steps should serialize"),
        json!([{ "step": "method", "name": "akra/kernel/attach" }, { "step": "resend" }])
    );
    assert_eq!(
        serde_json::to_value(reason_next(ReasonCode::SteerInFlight))
            .expect("steps should serialize"),
        json!([{ "step": "wait", "milliseconds": 1000 }, { "step": "resend" }])
    );
}

#[test]
fn every_reason_reaches_a_client() {
    // 명령 거절, prompt_submission 또는 저장된 이슈 판정으로 모든 이유가 클라이언트에 도달한다.
    let mut reached = KernelCommandKind::ALL
        .iter()
        .flat_map(|kind| kind.spec().rejections.iter().copied())
        .collect::<Vec<_>>();
    let task_id = "task-1".to_string();
    let outcomes = [
        TurnSubmissionAdmission::RejectedActive {
            active_correlation: crate::core::app::TurnSubmissionCorrelation::new(1),
        },
        TurnSubmissionAdmission::RejectedStopPending {
            stop_correlation: crate::core::app::StopRequestCorrelation::new(1, None),
        },
        TurnSubmissionAdmission::RejectedUnavailable,
    ]
    .into_iter()
    .map(|admission| ThreadPromptOutcome::Submitted {
        admission,
        task_id: task_id.clone(),
    })
    .chain([
        ThreadPromptOutcome::ParallelModeStarted {
            task_id: task_id.clone(),
        },
        ThreadPromptOutcome::QueuedTaskFinished {
            task_id: task_id.clone(),
        },
        ThreadPromptOutcome::QueuedRejectedAgain {
            task_id: task_id.clone(),
        },
    ])
    .chain(
        [
            ThreadPromptPreparationFailure::Cancelled,
            ThreadPromptPreparationFailure::TextMismatch,
            ThreadPromptPreparationFailure::BootstrapReviewRequired {
                draft_name: "draft".to_string(),
            },
            ThreadPromptPreparationFailure::PlanningRefused {
                reason: "invalid".to_string(),
            },
        ]
        .map(ThreadPromptOutcome::PreparationFailed),
    );
    for outcome in outcomes {
        let event = KernelEvent::from_core(&AppEvent::ThreadPromptSubmitted {
            thread_id: "thread-1".to_string(),
            outcome,
        });
        if let Some(KernelEvent::PromptSubmission(state)) = event {
            assert_eq!(
                state.next,
                state.reason.map(reason_next).unwrap_or_default()
            );
            reached.extend(state.reason);
        }
    }
    reached.extend(
        crate::domain::issue_automation::triage::IssueTriageReasonCode::ALL
            .iter()
            .copied()
            .map(super::issue_automation::IssueTriageReasonState::from)
            .map(|reason| reason.code),
    );
    let mut superseded = sample_issue_decision_fact();
    superseded.state = crate::domain::issue_automation::decision::IssueDecisionState::Superseded;
    reached.extend(IssueDecisionChangedState::from_fact(&superseded, 42).reason);
    for reason in ReasonCode::ALL {
        assert!(
            reached.contains(reason),
            "{} reaches no client",
            reason.name()
        );
    }
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
            "kernel_unreachable",
            "busy",
            "preparation_active",
            "unknown_thread",
            "session_list_unavailable",
            "foreign_workspace",
            "parallel_mode",
            "parallel_mode_unknown"
        ])
    );
    // The value the kernel keeps only to resubmit a queued prompt is not part of the spec.
    assert!(spec["$defs"]["QueuedThreadPrompt"]["properties"]["handoff"].is_null());
    // Code comments are for developers. The spec carries only the summaries of its tables, but it keeps
    // a field that is named `title`.
    assert_eq!(
        code_comment_keys(&spec, false),
        Vec::<String>::new(),
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
        [
            "session",
            "thread",
            "approval",
            "auto_follow",
            "parallel",
            "planning",
            "issue",
            "delegation"
        ]
    );
    let notifications = spec["methods"]
        .as_array()
        .expect("methods")
        .iter()
        .filter(|method| method["kind"] == "notification")
        .map(|method| method["name"].as_str().expect("a method name"))
        .collect::<Vec<_>>();
    assert_eq!(
        notifications,
        ["akra/kernel/event", "akra/kernel/extensions/changed"]
    );
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
pub(super) fn schema_errors(
    schema: &Value,
    value: &Value,
    definitions: &Value,
    path: &str,
) -> Vec<String> {
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
    if let Some(number) = value.as_f64()
        && (schema
            .get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|minimum| number < minimum)
            || schema
                .get("maximum")
                .and_then(Value::as_f64)
                .is_some_and(|maximum| number > maximum))
    {
        errors.push(format!("{path}: the number {number} is out of range"));
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
        if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
            match pattern {
                r"\S" if text.trim().is_empty() => {
                    errors.push(format!("{path}: the text is blank"));
                }
                r"\S" => {}
                "^[a-f0-9]{64}$" => {
                    if text.len() != 64
                        || !text
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    {
                        errors.push(format!(
                            "{path}: expected 64 lowercase hexadecimal characters"
                        ));
                    }
                }
                _ => errors.push(format!("{path}: unsupported schema pattern {pattern}")),
            }
        }
    }
    if let Some(elements) = value.as_array() {
        let length = elements.len() as u64;
        if schema
            .get("minItems")
            .and_then(Value::as_u64)
            .is_some_and(|minimum| length < minimum)
            || schema
                .get("maxItems")
                .and_then(Value::as_u64)
                .is_some_and(|maximum| length > maximum)
        {
            errors.push(format!("{path}: the array length {length} is out of range"));
        }
        if let Some(items) = schema.get("items") {
            for (index, element) in elements.iter().enumerate() {
                errors.extend(schema_errors(
                    items,
                    element,
                    definitions,
                    &format!("{path}[{index}]"),
                ));
            }
        }
    }
    errors
}

/// 스펙의 목록에서 그 이름의 항목이 가진 스키마로 값을 검사한다.
pub(super) fn assert_matches_spec(spec: &Value, list: &str, field: &str, value: &Value) {
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

#[test]
fn parallel_state_values_are_typed_and_keep_their_wire_names() {
    /*
     * plan 18 K4-2: the state values of the parallel board are closed sets in the spec. The wire names are the
     * ones clients already receive: the display label of the domain state with `_` for a space.
     */
    use crate::domain::parallel_mode::{
        ParallelModeAgentRosterState, ParallelModeQueueItemState, PrValidationOperatorState,
        PrValidationPhase,
    };
    let wire = |value: &dyn erased::Json| value.json();
    for state in ParallelModeAgentRosterState::ALL {
        assert_eq!(
            wire(&ParallelAgentState::from_core(Some(*state))),
            json!(state.name())
        );
    }
    assert_eq!(wire(&ParallelAgentState::from_core(None)), json!("unknown"));
    // The spec explains each agent state. `unknown` is not a roster state and has no entry.
    assert_eq!(
        names_of(&spec_json()["terms"]["agent_state"]),
        ParallelModeAgentRosterState::ALL
            .iter()
            .map(|state| state.name())
            .collect::<Vec<_>>()
    );
    let merge_states = [
        ParallelModeQueueItemState::Idle,
        ParallelModeQueueItemState::Queued,
        ParallelModeQueueItemState::Pushing,
        ParallelModeQueueItemState::PrPending,
        ParallelModeQueueItemState::MergePending,
        ParallelModeQueueItemState::Integrating,
        ParallelModeQueueItemState::Cleaning,
        ParallelModeQueueItemState::Done,
        ParallelModeQueueItemState::Blocked,
        ParallelModeQueueItemState::Failed,
    ];
    for state in merge_states {
        assert_eq!(
            wire(&ParallelMergeState::from_core(state)),
            json!(state.label().replace(' ', "_"))
        );
    }
    let validation_states = [
        PrValidationOperatorState::Pending,
        PrValidationOperatorState::Blocking,
        PrValidationOperatorState::Remediation,
        PrValidationOperatorState::Terminal,
        PrValidationOperatorState::Blocked,
        PrValidationOperatorState::Failed,
    ];
    for state in validation_states {
        assert_eq!(
            wire(&ParallelPrValidationState::from_core(state)),
            json!(state.label().replace(' ', "_"))
        );
    }
    let phases = [
        (PrValidationPhase::Registered, "registered"),
        (
            PrValidationPhase::PreMergeObservation,
            "pre_merge_observation",
        ),
        (PrValidationPhase::RemediationQueued, "remediation_queued"),
        (PrValidationPhase::RemediationRunning, "remediation_running"),
        (
            PrValidationPhase::PostMergeObservation,
            "post_merge_observation",
        ),
        (PrValidationPhase::Settled, "settled"),
        (PrValidationPhase::Blocked, "blocked"),
        (PrValidationPhase::Failed, "failed"),
    ];
    for (phase, name) in phases {
        assert_eq!(
            wire(&ParallelPrValidationPhase::from_core(phase)),
            json!(name)
        );
    }
    // The spec lists the values. A client no longer reads them as free text.
    let spec = spec_json();
    for (type_name, count) in [
        (
            "ParallelAgentState",
            ParallelModeAgentRosterState::ALL.len() + 1,
        ),
        ("ParallelMergeState", merge_states.len()),
        ("ParallelPrValidationState", validation_states.len()),
        ("ParallelPrValidationPhase", phases.len()),
    ] {
        assert_eq!(
            spec["$defs"][type_name]["enum"].as_array().map(Vec::len),
            Some(count),
            "{type_name}"
        );
    }
    for (owner, field, type_name) in [
        ("ParallelAgent", "state", "ParallelAgentState"),
        ("ParallelMergeItem", "state", "ParallelMergeState"),
        ("ParallelPrValidation", "state", "ParallelPrValidationState"),
        ("ParallelPrValidation", "phase", "ParallelPrValidationPhase"),
    ] {
        assert_eq!(
            spec["$defs"][owner]["properties"][field]["$ref"],
            json!(format!("#/$defs/{type_name}")),
            "{owner}.{field}"
        );
    }
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
        "maxItems",
        "maxLength",
        "minItems",
        "minLength",
        "minimum",
        "maximum",
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
    // `format` names numeric representations; minimum/maximum bound page sizes and minItems/maxItems
    // bound list lengths. Every pattern is checked above, including exact evidence identity tokens.
    let text = serde_json::to_string(&spec).expect("the spec should serialize");
    assert!(!text.contains("\"additionalProperties\":{"));
    assert_eq!(
        text.matches("\"pattern\":").count(),
        text.matches("\"pattern\":\"\\\\S\"").count()
            + text.matches("\"pattern\":\"^[a-f0-9]{64}$\"").count()
    );
}

#[test]
fn the_test_checker_enforces_evidence_tokens_and_array_bounds() {
    let spec = spec_json();
    let prepare = spec["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "prepare_evidence")
        .unwrap();
    let token = &prepare["input"]["properties"]["scope_id"];
    for (candidate, accepted) in [
        ("a".repeat(64), true),
        ("A".repeat(64), false),
        ("g".repeat(64), false),
        ("a".repeat(63), false),
        ("a".repeat(65), false),
        (format!("{}\n", "a".repeat(64)), false),
    ] {
        assert_eq!(
            schema_errors(token, &json!(candidate), &spec["$defs"], "scope_id").is_empty(),
            accepted
        );
    }
    let query = spec["queries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "task_evidence")
        .unwrap();
    let ids = &query["input"]["properties"]["task_ids"];
    for (length, accepted) in [(0, false), (1, true), (256, true), (257, false)] {
        let value = json!(
            (0..length)
                .map(|index| format!("task-{index}"))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            schema_errors(ids, &value, &spec["$defs"], "task_ids").is_empty(),
            accepted,
            "task_ids length {length}"
        );
    }
    assert!(!schema_errors(ids, &json!([1]), &spec["$defs"], "task_ids").is_empty());
    assert!(
        !schema_errors(
            &json!({ "pattern": "unknown" }),
            &json!("anything"),
            &spec["$defs"],
            "text"
        )
        .is_empty()
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
        // The kernel reads this value and refuses the command (plan 18 K4-5). A destructive command whose
        // `confirm` field does not reach `confirmed()` fails here.
        assert!(!unconfirmed.confirmed(), "{name}");
        let mut confirmed = serde_json::to_value(&command).expect("command should serialize");
        confirmed["confirm"] = json!(true);
        let confirmed: KernelCommand =
            serde_json::from_value(confirmed).expect("the confirmed command should parse");
        assert!(confirmed.confirmed(), "{name}");
    }
    assert_eq!(
        destructive, 4,
        "reset_planning, delete_task, delete_direction, and cancel_issue_work are the destructive commands today"
    );
}

/// 값이 채워진 기록 표본이다. 종류마다 하나이다.
fn sample_issue_reference() -> crate::domain::issue_work::IssueReference {
    crate::domain::issue_work::IssueReference {
        provider: crate::domain::issue_work::IssueProvider::GitHub,
        server_id: "github.com".to_owned(),
        repository_id: "R_stable".to_owned(),
        issue_id: "I_stable".to_owned(),
        project_scope: crate::domain::issue_work::IssueProjectScope {
            workspace_dir: "/workspace".to_owned(),
        },
    }
}

fn sample_records() -> Vec<crate::domain::kernel_event::KernelEvent> {
    use crate::domain::kernel_event::{
        ApprovalRequestKind, ApprovalResolution, KernelEvent as Record, PendingApproval,
        PostTurnRoute, QueuedThreadPrompt, ReasonCode, ThreadPromptSettlement, TurnEndOutcome,
        TurnOrigin,
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
        op_id: None,
    };
    let approval = PendingApproval::new(
        "thread-1",
        Some("turn-1"),
        "approval-1",
        ApprovalRequestKind::FileChange,
        "File change requested.",
    );
    vec![
        Record::AutoFollowConfigured {
            max_auto_turns: 3,
            continuation_paused: true,
        },
        queued.settled(ThreadPromptSettlement::TaskFinished),
        Record::ThreadPromptQueued(queued),
        Record::ThreadPromptRejected {
            thread_id: "thread-1".to_string(),
            reason: ReasonCode::ParallelMode,
        },
        Record::TaskExecutionLinked {
            task_id: "task-1".into(), thread_id: "thread-1".into(), op_id: 7,
        },
        Record::TurnUsageObserved {
            thread_id: "thread-1".into(), turn_id: "turn-1".into(), op_id: Some(7),
            last: Default::default(), total: Default::default(),
        },
        Record::TurnSubmitted {
            thread_id: String::new(),
            submission: 3,
            origin: TurnOrigin::AutoFollow,
            op_id: None,
        },
        Record::TurnStarted {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            op_id: None,
        },
        Record::TurnEnded {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            outcome: Some(TurnEndOutcome::Interrupted),
            op_id: None,
        },
        Record::TurnEnded {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-2".to_string(),
            outcome: None,
            op_id: None,
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
            completion: Some(crate::domain::kernel_spec::CommandCompletion::Unreported),
            caller: None,
        },
        Record::CommandRejected {
            op_id: 7,
            thread_id: "thread-1".to_string(),
            reason: ReasonCode::Busy,
        },
        Record::CommandCompleted {
            task_mutation: None,
            op_id: 7,
            thread_id: "thread-1".to_string(),
            result: crate::domain::kernel_event::OperationEnd::Failed,
            reason: Some(ReasonCode::PlanningRefused),
            detail: Some("the request gave this cause".to_string()),
        },
        Record::RepositoryChanged {
            head: "b".repeat(40),
            branch: Some("main".to_string()),
            previous_head: Some("a".repeat(40)),
            previous_branch: None,
        },
        Record::issue_tracked(
            &crate::domain::issue_work::IssueReference {
                provider: crate::domain::issue_work::IssueProvider::GitHub,
                server_id: "github.com".to_string(),
                repository_id: "R_stable".to_string(),
                issue_id: "I_stable".to_string(),
                project_scope: crate::domain::issue_work::IssueProjectScope {
                    workspace_dir: "/workspace".to_string(),
                },
            },
            "binding-1",
            3,
        ),
        Record::issue_triaged(
            &sample_issue_reference(),
            "binding-1",
            4,
            1,
            &"a".repeat(64),
        ),
        Record::issue_decision_requested(&sample_issue_decision_fact(), 5),
        Record::issue_decision_resolved(&sample_issue_decision_fact(), 6),
        Record::issue_decision_arbiter_finished(
            &crate::domain::issue_automation::arbiter::IssueArbiterFinishedEvent {
                event_id: format!("issue-arbiter-finished:{}", "b".repeat(64)),
                fact: sample_issue_decision_fact(),
                outcome: crate::domain::issue_automation::arbiter::IssueArbiterFinishedOutcome::Waiting {
                    reason: crate::domain::issue_automation::arbiter::IssueArbiterWaitReason::LowConfidence,
                },
                at: "2026-11-12T00:00:00Z".into(),
                catalog_revision: 7,
            },
        ),
        Record::issue_decision_reminded(
            &crate::domain::issue_automation::decision::IssueDecisionReminder {
                event_id: format!("issue-decision-reminder:{}", "a".repeat(64)),
                fact: sample_issue_decision_fact(),
                due_at: "2026-11-12T00:00:00Z".into(),
                reminded_at: "2026-11-11T22:00:00Z".into(),
                revision: 7,
            },
        ),
        Record::IssueWorkSubmitted {
            event_id: "work-submitted-1".into(),
            issue: (&sample_issue_reference()).into(),
            work_order_id: "issue-work-1".into(),
            task_ids: vec!["task-1".into()],
            revision: 4,
        },
        Record::IssueHeld {
            issue: (&sample_issue_reference()).into(),
            work_order_id: Some("issue-work-1".into()),
            authorization_epoch: 2,
        },
        Record::IssueReleased {
            issue: (&sample_issue_reference()).into(),
            work_order_id: Some("issue-work-1".into()),
            authorization_epoch: 2,
        },
        Record::IssueWorkPermitRevoked {
            issue: (&sample_issue_reference()).into(),
            work_order_id: Some("issue-work-1".into()),
            authorization_epoch: 2,
        },
        Record::IssueWorkPermitGranted {
            event_id: "work-permit-1".into(),
            issue: (&sample_issue_reference()).into(),
            work_order_id: "issue-work-1".into(),
            source_sha: "a".repeat(40),
            gate_revision: 1,
            delivery: false,
            revision: 5,
        },
        Record::IssueExecutionProgressed {
            event_id: "execution-progress-1".into(),
            issue: (&sample_issue_reference()).into(),
            work_order_id: "issue-work-1".into(),
            progress: crate::domain::issue_automation::execution::IssueExecutionProgress::default(),
            revision: 6,
        },
        Record::IssueIntegrated {
            event_id: "execution-integrated-1".into(),
            issue: (&sample_issue_reference()).into(),
            work_order_id: "issue-work-1".into(),
            integration: crate::domain::issue_automation::execution::IssueExecutionIntegration {
                record_id: "integration-1".into(),
                source_sha: "a".repeat(40),
                integration_sha: "b".repeat(40),
                pull_request_url: Some("https://github.com/example/project/pull/1".into()),
            },
            revision: 7,
        },
        Record::IssueClosed {
            event_id: "feedback-closed-1".into(),
            issue: (&sample_issue_reference()).into(),
            origin: crate::domain::kernel_event::IssueClosureRecordOrigin::ConfirmedFeedback,
            intent_id: Some("closed-intent-1".into()),
            actor_id: Some("actor-stable".into()),
            closure_event_id: Some("closure-stable".into()),
            observation_revision: None,
            revision: 8,
        },
        Record::IssueFeedbackPosted {
            event_id: "feedback-posted-1".into(),
            issue: (&sample_issue_reference()).into(),
            intent_id: "feedback-intent-1".into(),
            action: crate::domain::kernel_event::IssueFeedbackRecordAction::CommentCreated,
            actor_id: "actor-stable".into(),
            comment_id: Some("comment-stable".into()),
            label_id: None,
            revision: 8,
        },
        Record::issue_automation_mode_changed(
            Some(crate::domain::issue_automation::collection::IssueCollectionMode::Observe),
            crate::domain::issue_automation::collection::IssueCollectionMode::Off,
            2,
            3,
            Some("akra-desktop"),
        ),
        Record::task_evidence(&crate::domain::kernel_event::sample_task_evidence_events()[0]),
        Record::task_evidence(&crate::domain::kernel_event::sample_task_evidence_events()[1]),
        Record::task_evidence(&crate::domain::kernel_event::sample_task_verification_event()),
        crate::domain::kernel_event::sample_issue_binding_pause_event(),
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
        KernelQuery::Delegations {},
        KernelQuery::Delegation { id: "job-1".into() },
        KernelQuery::DelegationResult { id: "job-1".into() },
        KernelQuery::DelegationBackends {},
        KernelQuery::DelegationMyTurn {},
        KernelQuery::FeatureBindings {},
        KernelQuery::PrReviewResult { number: 42 },
        KernelQuery::Sessions {},
        KernelQuery::AutoFollow {},
        KernelQuery::ThreadActivity {},
        KernelQuery::Planning {},
        KernelQuery::ListTasks {},
        KernelQuery::TaskResult {
            task_id: "task-1".into(),
        },
        KernelQuery::TaskUsage {
            task_id: "task-1".into(),
        },
        KernelQuery::TaskEvidence {
            task_ids: None,
            after_task_id: None,
            limit: None,
        },
        KernelQuery::Parallel {},
        KernelQuery::Attention {},
        KernelQuery::History {
            after: Some(3),
            limit: None,
        },
        KernelQuery::Operation { op_id: 7 },
        KernelQuery::IssueAutomation {},
        KernelQuery::IssueAutomationPolicy {},
        KernelQuery::Issues {
            after: None,
            limit: None,
        },
        KernelQuery::Issue {
            issue: (&sample_issue_reference()).into(),
        },
    ] {
        assert_matches_spec(&spec, "queries", "input", &to_json(&query));
        matches_type(
            "QueryRequest",
            to_json(&QueryRequest {
                query: query.clone(),
            }),
        );
        let page = crate::domain::kernel_event::KernelEventPage {
            records: records
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, event)| record(index as i64 + 1, event))
                .collect(),
            next_after: 12,
            recorder: Some(crate::domain::kernel_event::KernelRecorderHealth {
                degraded: true,
                saturated: true,
                pending_bytes: 1,
                failed_events: 1,
                dropped_events: 1,
            }),
        };
        let answer = query.answer(&snapshot, &[], no_parallel(), Some(page), None);
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
        KernelEvent::IssueDecision(IssueDecisionChangedState::from_fact(
            &sample_issue_decision_fact(),
            5,
        )),
        KernelEvent::Issues(IssueAutomationState::unavailable()),
        KernelEvent::Resync,
        KernelEvent::Recorded(record(9, records[0].clone())),
        KernelEvent::PromptSubmission(PromptSubmissionState {
            thread_id: "thread-1".to_string(),
            status: PromptSubmissionStatus::Queued,
            reason: Some(ReasonCode::TurnStartedDuringPreparation),
            next: reason_next(ReasonCode::TurnStartedDuringPreparation),
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
            task_evidence: None,
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
                    state: ParallelAgentState::ReportedComplete,
                    duration: "1m".to_string(),
                    summary: "working".to_string(),
                }],
                merge_head: "abc123".to_string(),
                merge_queue: vec![ParallelMergeItem {
                    task_title: "Task".to_string(),
                    state: ParallelMergeState::MergePending,
                    branch: "akra-agent/slot-1".to_string(),
                    commit: "abc123".to_string(),
                }],
                pr_validation: Some(ParallelPrValidation {
                    pull_request_number: 12,
                    url: "https://example.invalid/pull/12".to_string(),
                    state: ParallelPrValidationState::Remediation,
                    phase: ParallelPrValidationPhase::PostMergeObservation,
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
    let attention = KernelQueryAnswer::Attention {
        items: vec![AttentionItem {
            id: "task:task-1:2026-11-26T01:02:03Z".to_string(),
            cause: AttentionCause::TaskProposed,
            subject: AttentionSubject {
                kind: AttentionSubjectKind::Task,
                id: "task-1".to_string(),
            },
            revision: "2026-11-26T01:02:03Z".to_string(),
            summary: "Task".to_string(),
            updated_at: Some("2026-11-26T01:02:03Z".to_string()),
            next: vec![NextStep::AskUser],
        }],
    };
    assert_matches_spec(&spec, "queries", "output", &to_json(&attention));
    matches_type(
        "AttachRequest",
        json!({ "workspace": "/workspace", "client_name": "akra-desktop" }),
    );
    matches_type(
        "AttachResponse",
        json!({ "client_id": "1", "protocol_version": 1, "kernel": "repository" }),
    );
    // 명령의 답: 받아들인 영수증과, 영수증이 있는 거절.
    for rejection in [None, Some(ReasonCode::Busy)] {
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

fn sample_issue_decision_fact() -> crate::domain::issue_automation::decision::IssueDecisionFact {
    use crate::domain::issue_automation::decision::*;
    IssueDecisionFact {
        delivery_review: None,
        decision_id: IssueDecisionId::new("decision-1").unwrap(),
        subject: IssueDecisionSubject::Issue(sample_issue_reference()),
        gate: crate::domain::issue_automation::policy::IssueAutomationGate::Start,
        kind: IssueDecisionKind::StartApproval,
        basis: IssueDecisionBasis {
            delivery: None,
            authorization_epoch: 0,
            policy_version: "a".repeat(64),
            field_hash: None,
            work_order_id: None,
            gate_revision: 1,
        },
        state: IssueDecisionState::Open,
        answer_by: None,
        option: None,
    }
}

#[test]
fn task_usage_query_validates_identity_and_keeps_missing_usage_null() {
    let query: KernelQuery =
        serde_json::from_value(json!({"type":"task_usage","task_id":"task-1"})).unwrap();
    assert_eq!(query.task_usage_id().unwrap(), Some("task-1"));
    assert!(
        KernelQuery::TaskUsage {
            task_id: " ".into()
        }
        .task_usage_id()
        .is_err()
    );
    let answer = KernelQueryAnswer::TaskUsage(
        crate::domain::task_usage::TaskUsageState::unavailable("task-1".into()),
    );
    let json = serde_json::to_value(answer).unwrap();
    assert!(json["observed_tokens"].is_null());
    assert_eq!(json["status"], "unavailable");
}
