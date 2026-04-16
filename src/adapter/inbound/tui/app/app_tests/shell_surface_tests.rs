use std::thread;
use std::time::Duration;

use crossterm::event::Event;
use ratatui::prelude::Rect;

use super::super::shell_presentation::build_directions_maintenance_overlay_view;
use super::super::shell_runtime;
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning::BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT;
use crate::application::service::planning::{
    DirectionsMaintenanceDirectionSummary, DirectionsMaintenanceSummary,
    DirectionsSupportingFileStatus,
};
use crate::application::service::planning_contract::DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH;
use crate::domain::planning::QueueIdlePolicy;

use super::{
    build_automation_overlay_view, build_automation_preview_lines, build_automation_status_lines,
    build_conversation_shell_frame_view, build_conversation_shell_view, build_inline_tail_lines,
    build_planning_init_overlay_view, build_queue_overlay_view, build_ready_input_lines,
    build_session_overlay_view, build_startup_overlay_view, create_temp_workspace, make_test_app,
    ready_conversation, ready_turn_planning_capture, sample_planning_runtime_snapshot,
    sample_proposal_only_planning_runtime_snapshot, sample_startup_diagnostics,
    ConversationInputState, ConversationMessage, ConversationMessageKind,
    ConversationRuntimeEffect, ConversationRuntimeEvent, ConversationState, KeyCode, KeyEvent,
    KeyModifiers, PlannerWorkerStatus, PlanningExecutionSnapshot, PlanningRuntimeSnapshot,
    RecordedAutoFollowupActivity, SessionState, ShellActionAvailability, ShellFrontendMode,
    ShellOverlay, StartupState, TASK_LEDGER_FILE_PATH,
};

#[test]
fn stale_planning_capture_blocks_reconciliation_for_other_workspace() {
    let (mut app, codex_port) = make_test_app();
    let current_workspace = create_temp_workspace("planning-capture-current");
    let stale_workspace = create_temp_workspace("planning-capture-stale");
    let mut conversation = ready_conversation();
    conversation.cwd = current_workspace.clone();
    conversation.active_turn_workspace_directory = Some(current_workspace.clone());
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-mismatch".to_string());
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.conversation_state = ConversationState::ready(conversation);
    app.active_turn_planning_capture = Some(ready_turn_planning_capture(
        &stale_workspace,
        PlanningExecutionSnapshot {
            directions_toml: Some("version = 1".to_string()),
            task_ledger_json: Some("{\"version\":1,\"tasks\":[]}".to_string()),
            task_ledger_schema_json: None,
            result_output_markdown: None,
            queue_snapshot_json: None,
        },
    ));

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-mismatch".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ));

    assert!(codex_port
        .turn_calls
        .lock()
        .expect("turn call mutex poisoned")
        .is_empty());
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    assert_eq!(
        conversation
            .planning_runtime_snapshot
            .preview_status_label(),
        "blocked"
    );
    assert!(conversation
        .planning_runtime_snapshot
        .preview_detail()
        .is_some_and(|detail| detail.contains("stale planning snapshot")));
    assert!(conversation.runtime_notices.iter().any(|notice| {
        notice.contains(&stale_workspace) && notice.contains(&current_workspace)
    }));

    std::fs::remove_dir_all(current_workspace).expect("current workspace should be removed");
    std::fs::remove_dir_all(stale_workspace).expect("stale workspace should be removed");
}

#[test]
fn stream_worker_forces_failure_when_service_exits_without_terminal_event() {
    let (app, codex_port) = make_test_app();
    let mut runtime = shell_runtime::ShellRuntime::new(app);
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("conversation should start ready");
    };
    conversation.thread_id = "thread-123".to_string();
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context\nQueue Summary",
        "next task: task-1",
    ));
    codex_port
        .turn_stream_behavior
        .lock()
        .expect("turn stream behavior mutex poisoned")
        .error = Some("transport closed".to_string());

    runtime
        .app_mut()
        .submit_prompt("ship it".to_string(), super::PromptOrigin::Manual);

    for _ in 0..20 {
        thread::sleep(Duration::from_millis(5));
        runtime.poll_background_messages();
        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            continue;
        };
        if conversation.input_state == ConversationInputState::ReadyToContinue
            && conversation.status_text == "turn failed"
        {
            break;
        }
    }

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("conversation should remain ready");
    };
    assert_eq!(
        conversation.input_state,
        ConversationInputState::ReadyToContinue
    );
    assert_eq!(conversation.status_text, "turn failed");
    assert!(conversation.messages.iter().any(|message| {
        message.kind == ConversationMessageKind::Status
            && message
                .text
                .contains("turn stream failed before a terminal event: transport closed")
    }));
}

#[test]
fn inline_shell_command_buffer_shows_automation_hint() {
    let mut conversation = ready_conversation();
    conversation.input_buffer = ":auto".to_string();
    conversation.sync_inline_shell_command_palette();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("> :auto"));
    assert!(rendered.contains("automation controls"));
}

#[test]
fn automation_inline_command_opens_overlay() {
    let (app, _) = make_test_app();
    let mut runtime = shell_runtime::ShellRuntime::new(app);
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    runtime.app_mut().push_input_character(':');
    runtime.app_mut().push_input_character('a');
    runtime.app_mut().push_input_character('u');
    runtime.app_mut().push_input_character('t');
    runtime.app_mut().push_input_character('o');
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Automation);
    assert!(conversation.input_buffer.is_empty());
    assert!(conversation
        .status_text
        .contains("operator surface: automation controls"));
}

#[test]
fn automation_overlay_view_surfaces_preview_status_and_keys() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should be ready");
    };
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context\nQueue Summary",
        "next task: task-1",
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    app.show_automation_overlay();

    let view = build_automation_overlay_view(&app);
    let header = view
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let list = view
        .list_view
        .message_lines
        .expect("automation overlay should expose static list messaging")
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let preview = view
        .preview_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let status = view
        .status_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let keys = view
        .key_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(header.contains("Automation Controls"));
    assert!(
        list.contains("automation continues only when the planning queue exposes actionable work")
    );
    assert!(preview.contains("automation mode: planning queue"));
    assert!(preview.contains("current state:"));
    assert!(preview.contains("cause:"));
    assert!(preview.contains("next action:"));
    assert!(preview.contains("Rendered Next-Turn Prompt"));
    assert!(status.contains("automation state: on"));
    assert!(keys.contains("Ctrl+a toggles automation"));
}

#[test]
fn automation_preview_uses_placeholder_without_agent_reply() {
    let (app, _) = make_test_app();

    let rendered = build_automation_preview_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("last agent reply: waiting for the first agent reply"));
    assert!(rendered.contains("thread context: draft-thread"));
}

#[test]
fn automation_preview_surfaces_queue_refresh_copy_when_queue_is_idle() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should be ready");
    };
    conversation.replace_planning_runtime_snapshot(
        PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "queue idle: no executable planning task".to_string(),
            None,
        )
        .with_queue_idle_policy(
            crate::domain::planning::QueueIdlePolicy::ReviewAndEnqueue,
            Some(DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string()),
        ),
    );

    let rendered = build_automation_preview_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("planning priority queue를 갱신하세요."));
    assert!(rendered.contains("current state: waiting"));
    assert!(rendered.contains("cause: planning is valid but has no next task yet"));
    assert!(rendered.contains("next action:"));
}

#[test]
fn automation_status_lines_include_runtime_and_warning_summary() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should be ready");
    };
    conversation.status_text =
        "turn completed / automation queued the next turn / planning queue".to_string();
    conversation.base_warnings =
        vec!["planner queue changed shape after reconciliation".to_string()];
    conversation.warnings = conversation.base_warnings.clone();
    conversation.runtime_notices = vec!["planning reconciliation completed".to_string()];

    let rendered = build_automation_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("automation state: on"));
    assert!(rendered.contains("current state:"));
    assert!(rendered.contains("cause:"));
    assert!(rendered.contains("next action:"));
    assert!(rendered.contains(
        "operator status: turn completed / automation queued the next turn / planning queue"
    ));
    assert!(rendered.contains("warning: planner queue changed shape"));
    assert!(rendered.contains("planning reconciliation completed"));
}

#[test]
fn planner_debug_surfaces_use_operator_facing_labels() {
    let (mut app, _) = make_test_app();
    app.toggle_planner_visibility();
    app.planner_worker_panel_state.status = PlannerWorkerStatus::RefreshSucceeded;
    app.planner_worker_panel_state.last_operation_label = Some("refresh".to_string());
    app.planner_worker_panel_state.last_queue_summary = Some("next task: task-1".to_string());
    app.planner_worker_panel_state.last_summary =
        Some("worker promoted the next queued task".to_string());
    app.planner_worker_panel_state.last_host_detail =
        Some("host accepted the refreshed queue".to_string());
    app.planner_worker_panel_state.last_prompt = Some("planner prompt".to_string());
    app.planner_worker_panel_state.last_response = Some("planner response".to_string());

    let preview = build_automation_preview_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let status = build_automation_status_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(preview.contains("Planner Debug Context"));
    assert!(preview.contains("planner session: refresh  |  state: refresh ok"));
    assert!(preview.contains("Submitted Prompt"));
    assert!(preview.contains("Planner Reply"));
    assert!(status.contains("planner state: refresh ok  |  queued work: next task: task-1"));
    assert!(status.contains("planner update: worker promoted the next queued task"));
    assert!(status.contains("operator action: host accepted the refreshed queue"));
}

#[test]
fn auto_follow_transcript_debug_detail_uses_operator_facing_labels() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.toggle_planner_visibility();
    app.planner_worker_panel_state.status = PlannerWorkerStatus::RefreshSucceeded;
    app.planner_worker_panel_state.last_operation_label = Some("refresh".to_string());
    app.planner_worker_panel_state.last_summary =
        Some("worker promoted the next queued task".to_string());
    app.planner_worker_panel_state.last_prompt = Some("planner prompt".to_string());
    app.planner_worker_panel_state.last_response = Some("planner response".to_string());

    app.execute_conversation_runtime_effect(ConversationRuntimeEffect::QueueAutoPrompt {
        prompt: "ship it".to_string(),
        queued_from_turn_id: "turn-1".to_string(),
        mode_label: "planning queue".to_string(),
        transcript_text: BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string(),
        handoff_task: None,
    });

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };
    let debug_detail = conversation.messages[0]
        .debug_detail
        .as_deref()
        .expect("auto-follow message should keep debug detail");

    assert!(debug_detail.contains("planner session: refresh  |  state: refresh ok"));
    assert!(debug_detail.contains("planner update: worker promoted the next queued task"));
    assert!(debug_detail.contains("Submitted Prompt"));
    assert!(debug_detail.contains("Planner Reply"));
}

#[test]
fn startup_checks_overlay_uses_current_state_cause_and_next_action() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));

    let view = build_startup_overlay_view(&app);
    let header = view
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let summary = view
        .summary_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let checks = view
        .check_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(header.contains("Startup Checks"));
    assert!(summary.contains("current state: ready"));
    assert!(summary.contains("cause: codex, workspace, app-server, and account access are ready"));
    assert!(
        summary.contains("next action: continue in the shell or open another inspection surface")
    );
    assert!(checks.contains("[ready] codex CLI: ok"));
    assert!(checks.contains("[ready] app-server readiness: ok"));
}

#[test]
fn non_rendering_overlay_headers_use_canonical_operator_titles() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));

    let startup_header = build_startup_overlay_view(&app)
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(startup_header.contains("Startup Checks / operator inspection"));
    assert!(!startup_header.contains("shell inspection"));

    let session_header = build_session_overlay_view(&app)
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(session_header.contains("Recent Sessions / operator inspection"));
    assert!(!session_header.contains("shell inspection"));

    let automation_header = build_automation_overlay_view(&app)
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(automation_header.contains("Automation Controls / operator inspection"));
    assert!(!automation_header.contains("shell inspection"));

    let queue_header = build_queue_overlay_view(&app)
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(queue_header.contains("Planning Queue / operator inspection"));
    assert!(!queue_header.contains("shell inspection"));

    let planning_setup_header = build_planning_init_overlay_view(&app)
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(planning_setup_header.contains("Planning Setup / operator inspection"));
    assert!(!planning_setup_header.contains("shell guidance"));

    app.planning_init_overlay_ui_state.open_manual_editor();
    let planning_draft_header = build_planning_init_overlay_view(&app)
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(planning_draft_header.contains("Planning Draft / operator inspection"));

    app.directions_maintenance_overlay_ui_state
        .open_summary(DirectionsMaintenanceSummary {
            directions: vec![DirectionsMaintenanceDirectionSummary {
                id: "operator-copy".to_string(),
                title: "Operator Copy".to_string(),
                detail_doc_path: None,
                detail_doc_status: DirectionsSupportingFileStatus::MissingMapping,
            }],
            missing_detail_doc_count: 1,
            broken_detail_doc_count: 0,
            queue_idle_policy: QueueIdlePolicy::ReviewAndEnqueue,
            queue_idle_prompt_path: None,
            queue_idle_prompt_status: DirectionsSupportingFileStatus::MissingMapping,
            parse_error: None,
        });
    let directions_header = build_directions_maintenance_overlay_view(&app)
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(directions_header.contains("Direction Maintenance / operator inspection"));
    assert!(!directions_header.contains("shell inspection"));

    app.directions_maintenance_overlay_ui_state
        .open_manual_editor();
    let direction_draft_header = build_directions_maintenance_overlay_view(&app)
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(direction_draft_header.contains("Direction Draft / operator inspection"));
}

#[test]
fn directions_maintenance_status_lines_use_canonical_operator_labels() {
    let (mut app, _) = make_test_app();
    app.directions_maintenance_overlay_ui_state
        .open_summary(DirectionsMaintenanceSummary {
            directions: vec![DirectionsMaintenanceDirectionSummary {
                id: "operator-copy".to_string(),
                title: "Operator Copy".to_string(),
                detail_doc_path: None,
                detail_doc_status: DirectionsSupportingFileStatus::MissingMapping,
            }],
            missing_detail_doc_count: 1,
            broken_detail_doc_count: 0,
            queue_idle_policy: QueueIdlePolicy::ReviewAndEnqueue,
            queue_idle_prompt_path: None,
            queue_idle_prompt_status: DirectionsSupportingFileStatus::MissingMapping,
            parse_error: None,
        });

    let overview_status = build_directions_maintenance_overlay_view(&app)
        .status_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        overview_status.contains("direction coverage: 1 total / 1 missing docs / 0 broken docs")
    );
    assert!(overview_status.contains(
        "queue idle rule: policy review_and_enqueue / prompt state: unset / prompt path: <none>"
    ));
    assert!(overview_status.contains("direction parsing: ok"));

    app.directions_maintenance_overlay_ui_state
        .open_detail_doc_selection();
    let selection_status = build_directions_maintenance_overlay_view(&app)
        .status_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(selection_status.contains("current selection: Operator Copy"));

    app.directions_maintenance_overlay_ui_state
        .open_detail_doc_confirm();
    let confirm_view = build_directions_maintenance_overlay_view(&app);
    let confirm_summary = confirm_view
        .summary_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let confirm_status = confirm_view
        .status_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(confirm_summary.contains("current direction: Operator Copy"));
    assert!(confirm_status.contains("current state: ready to stage the detail doc repair"));
}

#[test]
fn queue_overlay_notice_drops_legacy_planning_prefix_duplication() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should be ready");
    };
    conversation.runtime_notices =
        vec!["planning repair queued retry 1/2 for task-ledger.json".to_string()];

    let rendered = build_queue_overlay_view(&app)
        .note_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("planning notice: planning repair queued retry 1/2"));
    assert!(!rendered.contains("planning notice: planning:"));
}

#[test]
fn queue_overlay_uses_current_state_cause_and_next_action_summary() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should be ready");
    };
    conversation.replace_planning_runtime_snapshot(PlanningRuntimeSnapshot::ready_with_details(
        "Planning Context".to_string(),
        "queue idle: no executable planning task".to_string(),
        Some("1 promotable follow-up proposal available: Draft queue review".to_string()),
        None,
    ));

    let rendered = build_queue_overlay_view(&app)
        .summary_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("current state: review needed"));
    assert!(rendered.contains("cause: planning has proposals but no executable next task"));
    assert!(rendered.contains("next action: review the queue and promote the next actionable task"));
}

#[test]
fn queue_overlay_reframes_detail_as_now_next_proposed_and_blocked_work() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should be ready");
    };
    conversation.replace_planning_runtime_snapshot(sample_planning_runtime_snapshot(
        "Planning Context",
        "next task: task-1",
    ));

    let queue_view = build_queue_overlay_view(&app);
    let now_lines = queue_view
        .now_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let next_lines = queue_view
        .next_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let proposed_lines = queue_view
        .proposed_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let blocked_lines = queue_view
        .blocked_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(now_lines.contains("Implement shell planning status"));
    assert!(next_lines.contains("Trim legacy shell code"));
    assert!(proposed_lines.contains("No proposed work is waiting for review."));
    assert!(blocked_lines.contains("Follow blocked review thread"));
    assert!(blocked_lines.contains("blocked by tasks: task-2(in_progress)"));
}

#[test]
fn queue_overlay_shows_proposal_only_state_without_now_or_next_work() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should be ready");
    };
    conversation.replace_planning_runtime_snapshot(sample_proposal_only_planning_runtime_snapshot(
        "Planning Context",
        "queue idle: no executable planning task",
        "1 promotable follow-up proposal available: Draft a queue inspection overlay",
    ));

    let queue_view = build_queue_overlay_view(&app);
    let now_lines = queue_view
        .now_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let next_lines = queue_view
        .next_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let proposed_lines = queue_view
        .proposed_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let blocked_lines = queue_view
        .blocked_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(now_lines.contains("No work is actionable now."));
    assert!(next_lines.contains("No additional queued work is next in line."));
    assert!(proposed_lines.contains("Draft a queue inspection overlay"));
    assert!(blocked_lines.contains("No blocked work is holding the queue right now."));
}

#[test]
fn queue_overlay_loading_and_failed_states_use_operator_facing_summary_lines() {
    let (mut app, _) = make_test_app();
    app.conversation_state = ConversationState::Loading;

    let loading_summary = build_queue_overlay_view(&app)
        .summary_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(loading_summary.contains("current state: waiting"));
    assert!(loading_summary.contains("cause: conversation planning state is still loading"));
    assert!(loading_summary.contains("next action: wait for the thread to finish loading"));

    app.conversation_state = ConversationState::Failed("transport closed".to_string());

    let failed_view = build_queue_overlay_view(&app);
    let failed_summary = failed_view
        .summary_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let failed_notes = failed_view
        .note_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(failed_summary.contains("current state: blocked"));
    assert!(failed_summary.contains(
        "cause: conversation planning state is unavailable because the thread failed to load"
    ));
    assert!(failed_summary.contains("next action: reload the session or open a new draft"));
    assert!(failed_notes.contains("conversation error: transport closed"));
}

#[test]
fn recent_sessions_overlay_waiting_and_blocked_states_use_operator_language() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Loading;
    app.session_state = SessionState::Idle;

    let waiting_view = build_session_overlay_view(&app);
    let waiting_list = waiting_view
        .list_view
        .message_lines
        .expect("waiting state should expose list copy")
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let waiting_detail = waiting_view
        .detail_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(waiting_list.contains("waiting for startup checks"));
    assert!(waiting_detail.contains("current state: waiting"));
    assert!(waiting_detail.contains("cause: startup checks have not finished yet"));
    assert!(waiting_detail
        .contains("next action: wait for startup checks to finish, then load recent sessions"));

    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", false));
    app.session_state = SessionState::Idle;

    let blocked_idle_detail = build_session_overlay_view(&app)
        .detail_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(blocked_idle_detail.contains("current state: blocked"));
    assert!(blocked_idle_detail
        .contains("cause: startup checks must succeed before recent sessions are available"));
    assert!(blocked_idle_detail.contains(
        "next action: open startup checks with Ctrl+d, fix them, then reload recent sessions"
    ));

    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.session_state = SessionState::Loading;

    let loading_view = build_session_overlay_view(&app);
    let loading_list = loading_view
        .list_view
        .message_lines
        .expect("loading state should expose list copy")
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let loading_detail = loading_view
        .detail_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(loading_list.contains("loading recent sessions"));
    assert!(loading_detail.contains("current state: waiting"));
    assert!(loading_detail.contains("cause: recent sessions are loading from codex app-server"));
    assert!(loading_detail.contains("next action: wait for the session list to load"));

    app.session_state = SessionState::Failed("request timed out".to_string());

    let failed_view = build_session_overlay_view(&app);
    let failed_list = failed_view
        .list_view
        .message_lines
        .expect("failed state should expose list copy")
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let failed_detail = failed_view
        .detail_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let keys = failed_view
        .key_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(failed_list.contains("recent sessions blocked"));
    assert!(failed_detail.contains("current state: blocked"));
    assert!(failed_detail.contains("cause: recent sessions are unavailable because loading failed"));
    assert!(failed_detail.contains("next action: press r to retry, or start a new draft with n"));
    assert!(failed_detail.contains("recent sessions error: request timed out"));
    assert!(keys.contains("r reloads recent sessions"));
    assert!(keys.contains("n opens a new draft"));
    assert!(keys.contains("Ctrl+d opens startup checks"));
}

#[test]
fn selected_session_detail_uses_operator_facing_metadata_labels() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.session_state = SessionState::Ready(crate::domain::recent_sessions::RecentSessions {
        items: vec![crate::domain::session_summary::SessionSummary {
            id: "thread-1".to_string(),
            name: Some("Session thread-1".to_string()),
            preview: "Preview line".to_string(),
            cwd: "/tmp/root".to_string(),
            source: "native".to_string(),
            model_provider: "openai".to_string(),
            updated_at_epoch: 1_700_000_000,
            status_type: "ready".to_string(),
            path: "/tmp/root/thread-1.json".to_string(),
            git_branch: Some("feature/demo".to_string()),
        }],
        warnings: Vec::new(),
        next_cursor: Some("cursor-2".to_string()),
    });
    app.selected_session_index = 0;
    app.session_overlay_ui_state
        .set_selected_session_id(Some("thread-1".to_string()));
    app.session_overlay_ui_state.sync_selected_session(Some(0));

    let detail_lines = build_session_overlay_view(&app)
        .detail_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    let detail = detail_lines.join("\n");

    assert!(detail.contains("thread id: thread-1"));
    assert!(detail.contains("last updated: 2023-11-15 07:13"));
    assert!(detail.contains("workspace: /tmp/root"));
    assert!(detail.contains("thread source: native"));
    assert!(detail.contains("model provider: openai"));
    assert!(detail.contains("current state: ready"));
    assert!(detail.contains("git branch: feature/demo"));
    assert!(detail.contains("latest preview"));
    assert!(detail.contains("session file: /tmp/root/thread-1.json"));
    assert!(detail.contains("more threads are available in the next cursor"));
    assert!(!detail_lines
        .iter()
        .any(|line| line.starts_with("updated: ")));
    assert!(!detail_lines.iter().any(|line| line.starts_with("source: ")));
    assert!(!detail_lines.iter().any(|line| line.starts_with("status: ")));
    assert!(!detail.contains("\npreview\n"));
    assert!(!detail_lines.iter().any(|line| line.starts_with("path: ")));
}

#[test]
fn recent_session_list_rows_use_operator_facing_labels() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.session_state = SessionState::Ready(crate::domain::recent_sessions::RecentSessions {
        items: vec![crate::domain::session_summary::SessionSummary {
            id: "thread-1".to_string(),
            name: Some("Fix startup checks".to_string()),
            preview: "Preview line".to_string(),
            cwd: "/tmp/root".to_string(),
            source: "native".to_string(),
            model_provider: "openai".to_string(),
            updated_at_epoch: 1_700_000_000,
            status_type: "ready".to_string(),
            path: "/tmp/root/thread-1.json".to_string(),
            git_branch: None,
        }],
        warnings: Vec::new(),
        next_cursor: None,
    });
    app.selected_session_index = 0;
    app.session_overlay_ui_state
        .set_selected_session_id(Some("thread-1".to_string()));
    app.session_overlay_ui_state.sync_selected_session(Some(0));

    let list_item = build_session_overlay_view(&app)
        .list_view
        .items
        .into_iter()
        .next()
        .expect("session row should exist")
        .lines
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(list_item.contains("thread: Fix startup checks"));
    assert!(list_item.contains("current state: ready"));
    assert!(list_item.contains("last updated: 2023-11-15 07:13"));
    assert!(list_item.contains("workspace: root"));
    assert!(!list_item.contains("[native / openai]"));
    assert!(!list_item.contains("thread-1  2023-11-15 07:13"));
}

#[test]
fn loading_and_blocked_shell_views_use_canonical_operator_copy() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.conversation_state = ConversationState::Loading;

    let loading_view = build_conversation_shell_view(&app, ShellFrontendMode::InlineMainBuffer);
    let loading_header = loading_view
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let loading_transcript = loading_view
        .conversation_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let loading_input = loading_view
        .input_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(loading_header.contains("Conversation Shell / waiting"));
    assert!(loading_header.contains("current state: waiting"));
    assert!(loading_header.contains("cause: thread history is still loading from codex app-server"));
    assert!(loading_header.contains("next action: wait for the thread history to load"));
    assert_eq!(loading_view.input_title.to_string(), "Prompt / waiting");
    assert!(loading_transcript.contains("current state: waiting"));
    assert!(
        loading_transcript.contains("cause: thread history is still loading from codex app-server")
    );
    assert!(loading_transcript.contains("next action: wait for the thread history to load"));
    assert!(loading_input.contains("current state: waiting"));
    assert!(loading_input.contains("cause: thread history is still loading from codex app-server"));
    assert!(loading_input.contains("next action: wait for the thread history to load"));

    app.conversation_state = ConversationState::Failed("transport closed".to_string());

    let blocked_view = build_conversation_shell_view(&app, ShellFrontendMode::InlineMainBuffer);
    let blocked_header = blocked_view
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let blocked_transcript = blocked_view
        .conversation_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let blocked_input = blocked_view
        .input_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(blocked_header.contains("Conversation Shell / blocked"));
    assert!(blocked_header.contains("current state: blocked"));
    assert!(blocked_header.contains("cause: thread history is unavailable because loading failed"));
    assert!(blocked_header.contains("next action: reload the session or open a new draft"));
    assert!(blocked_header.contains("conversation error: transport closed"));
    assert_eq!(blocked_view.input_title.to_string(), "Prompt / blocked");
    assert!(blocked_transcript.contains("current state: blocked"));
    assert!(
        blocked_transcript.contains("cause: thread history is unavailable because loading failed")
    );
    assert!(blocked_transcript.contains("next action: reload the session or open a new draft"));
    assert!(blocked_transcript.contains("conversation error: transport closed"));
    assert!(blocked_input.contains("current state: blocked"));
    assert!(blocked_input.contains("cause: thread history is unavailable because loading failed"));
    assert!(blocked_input.contains("next action: reload the session or open a new draft"));
    assert!(blocked_input.contains("conversation error: transport closed"));
}

#[test]
fn ready_shell_header_uses_operator_facing_thread_labels() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));

    let mut draft = ready_conversation();
    draft.thread_id.clear();
    draft.input_state = ConversationInputState::DraftReady;
    app.conversation_state = ConversationState::ready(draft);

    let draft_header = build_conversation_shell_view(&app, ShellFrontendMode::InlineMainBuffer)
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(draft_header.contains("thread: new draft  |  input: draft ready"));
    assert!(!draft_header.contains("not started yet"));

    app.conversation_state = ConversationState::ready(ready_conversation());

    let ready_header = build_conversation_shell_view(&app, ShellFrontendMode::InlineMainBuffer)
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(ready_header.contains("thread: Existing session  |  input: ready"));
    assert!(!ready_header.contains("thread-1"));
}

#[test]
fn framed_shell_titles_and_empty_transcript_use_operator_facing_copy() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));

    let frame_view = build_conversation_shell_frame_view(
        &mut app,
        ShellFrontendMode::InlineMainBuffer,
        Rect::new(0, 0, 96, 28),
    );
    let transcript = frame_view
        .transcript_view
        .lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let header = frame_view
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        frame_view.shell_title.to_string(),
        "Shell / current conversation and operator controls"
    );
    assert_eq!(
        frame_view.transcript_view.title.to_string(),
        "Conversation / ready"
    );
    assert_eq!(
        frame_view.status_title.to_string(),
        "Status / current state, cause, and next action"
    );
    assert!(header.contains("operator surface: inline main buffer"));
    assert!(header.contains("transcript source: host terminal scrollback"));
    assert!(transcript.contains("current state: ready"));
    assert!(transcript.contains("cause: no messages have been recorded in this conversation yet"));
    assert!(transcript.contains("next action: send the first prompt to start the conversation"));
}

#[test]
fn framed_transcript_title_tracks_loading_and_blocked_states() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.conversation_state = ConversationState::Loading;

    let loading_frame = build_conversation_shell_frame_view(
        &mut app,
        ShellFrontendMode::InlineMainBuffer,
        Rect::new(0, 0, 96, 28),
    );
    assert_eq!(
        loading_frame.transcript_view.title.to_string(),
        "Conversation / waiting"
    );

    app.conversation_state = ConversationState::Failed("transport closed".to_string());

    let blocked_frame = build_conversation_shell_frame_view(
        &mut app,
        ShellFrontendMode::InlineMainBuffer,
        Rect::new(0, 0, 96, 28),
    );
    assert_eq!(
        blocked_frame.transcript_view.title.to_string(),
        "Conversation / blocked"
    );
}

#[test]
fn inline_notice_uses_operator_action_label_for_recent_auto_follow_activity() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should be ready");
    };
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.last_auto_followup_activity = Some(RecordedAutoFollowupActivity {
        summary: "queued the next turn".to_string(),
        detail: "host accepted the refreshed queue".to_string(),
    });

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains(
        "operator notice: automation update: queued the next turn  |  operator action: host accepted the refreshed queue"
    ));
}

#[test]
fn inline_tail_uses_canonical_planning_summary_and_plan_badge() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.conversation_state = ConversationState::ready(ready_conversation());
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should be ready");
    };
    conversation.replace_planning_runtime_snapshot(PlanningRuntimeSnapshot::ready_with_details(
        "Planning Context".to_string(),
        "queue idle: no executable planning task".to_string(),
        Some("1 promotable follow-up proposal available: Draft queue review".to_string()),
        None,
    ));

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Plan on / review needed"));
    assert!(rendered.contains("current state: review needed"));
    assert!(rendered.contains("cause: planning has proposals"));
    assert!(rendered.contains("next action: review the queue"));
}

#[test]
fn planning_controls_existing_workspace_uses_canonical_state_label() {
    let (mut app, _) = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("conversation should be ready");
    };
    conversation.replace_planning_runtime_snapshot(PlanningRuntimeSnapshot::ready_with_details(
        "Planning Context".to_string(),
        "queue idle: no executable planning task".to_string(),
        Some("1 promotable follow-up proposal available: Draft queue review".to_string()),
        None,
    ));
    app.planning_init_overlay_ui_state.open_existing_workspace();

    let rendered = build_planning_init_overlay_view(&app)
        .option_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("planning state: Plan on / review needed"));
}

#[test]
fn ctrl_l_starts_max_auto_turns_editing_inside_automation_overlay() {
    let (app, _) = make_test_app();
    let mut runtime = shell_runtime::ShellRuntime::new(app);
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('l'),
        KeyModifiers::CONTROL,
    )));

    assert!(runtime.app().is_max_auto_turns_editing());
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Automation);
}

#[test]
fn automation_overlay_scroll_keys_update_preview_offset() {
    let (mut app, _) = make_test_app();
    app.show_automation_overlay();
    assert_eq!(app.followup_overlay_ui_state.preview_scroll, 0);

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE,)));
    assert!(app.followup_overlay_ui_state.preview_scroll > 0);

    assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));
    assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
}
