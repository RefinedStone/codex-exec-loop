use super::planner_debug_preview::build_debug_preview_lines;
use super::*;
use crate::application::service::planning_runtime_facade_service::{
    PlanningRuntimePreviewRequest, PlanningRuntimeRepairAttempt,
    PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
};
use crate::application::service::turn_prompt_assembly_service::PREVIEW_THREAD_ID_PLACEHOLDER;
use crate::domain::text::compact_whitespace_detail;

const FOLLOWUP_WARNING_DETAIL_LIMIT: usize = 32;
const FOLLOWUP_RUNTIME_NOTICE_DETAIL_LIMIT: usize = 32;
const FOLLOWUP_GITHUB_REVIEW_DETAIL_LIMIT: usize = 24;
const FOLLOWUP_PLANNING_DETAIL_LIMIT: usize = 48;
const FOLLOWUP_PLANNER_PANEL_DETAIL_LIMIT: usize = 48;
const FOLLOWUP_PLANNER_DEBUG_MAX_LINES: usize = 256;

pub(super) fn build_planning_summary_line(
    app: &NativeTuiApp,
    conversation: &ConversationViewModel,
    max_detail_len: usize,
    always_show: bool,
) -> Option<String> {
    app.planning_services
        .runtime_facade
        .build_summary_line(PlanningRuntimeSummaryLineRequest {
            snapshot: &conversation.planning_runtime_snapshot,
            has_running_turn: conversation.has_running_turn(),
            is_repairing: conversation.planning_repair_state.is_some(),
            repair_failure_summary: conversation
                .planning_repair_state
                .as_ref()
                .map(|state| state.latest_request.failure_summary.as_str()),
            repair_attempt: conversation.planning_repair_state.as_ref().map(|state| {
                PlanningRuntimeRepairAttempt {
                    attempts_used: state.attempts_used,
                    max_attempts: state.max_attempts,
                }
            }),
            has_notice: conversation
                .planning_notice_summary(max_detail_len)
                .is_some(),
            max_detail_len,
            always_show,
        })
}

pub(super) fn build_planning_notice_line(
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> Option<String> {
    conversation
        .planning_notice_summary(max_detail_len)
        .map(|summary| format!("planning notice: {summary}"))
}

pub(super) fn build_planner_panel_lines(app: &NativeTuiApp, max_detail_len: usize) -> Vec<String> {
    if !app.planner_shows_debug_details() {
        return Vec::new();
    }

    let planner = &app.planner_worker_panel_state;
    if !planner.has_content() {
        return Vec::new();
    }

    let mut first_line = format!("planner status: {}", planner.status.label());
    if let Some(queue_summary) = planner.last_queue_summary.as_deref() {
        first_line.push_str(&format!(
            "  |  planner queue: {}",
            compact_whitespace_detail(queue_summary, max_detail_len)
        ));
    }

    let mut lines = vec![first_line];
    if let Some(summary) = planner.last_summary.as_deref() {
        lines.push(format!(
            "planner detail: {}",
            compact_whitespace_detail(summary, max_detail_len)
        ));
    }
    if let Some(notice_detail) = planner.last_notice_detail.as_deref() {
        lines.push(format!(
            "planner notice: {}",
            compact_whitespace_detail(notice_detail, max_detail_len)
        ));
    }
    if let Some(host_detail) = planner.last_host_detail.as_deref() {
        lines.push(format!(
            "planner host detail: {}",
            compact_whitespace_detail(host_detail, max_detail_len)
        ));
    }
    if let Some(rejected_summary) = planner.last_rejected_summary.as_deref() {
        lines.push(format!(
            "planner rejected: {}",
            compact_whitespace_detail(rejected_summary, max_detail_len)
        ));
    }
    lines
}

pub(super) fn build_followup_template_preview_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Loading => vec![Line::from("conversation is still loading")],
        ConversationState::Failed(message) => vec![Line::from(message.clone())],
        ConversationState::Ready(conversation) => {
            let template = conversation.auto_follow_state.selected_template();
            let preview = app
                .planning_services
                .runtime_facade
                .build_auto_follow_preview(PlanningRuntimePreviewRequest {
                    template,
                    auto_turn: conversation.auto_follow_state.next_auto_turn_index(),
                    max_auto_turns: conversation.auto_follow_state.max_auto_turns_value(),
                    session_id: &conversation.thread_id,
                    stop_keyword: conversation.auto_follow_state.stop_keyword_value(),
                    last_message: conversation.latest_agent_message_text(),
                    snapshot: &conversation.planning_runtime_snapshot,
                });
            let preview_thread_id = if conversation.thread_id.trim().is_empty() {
                PREVIEW_THREAD_ID_PLACEHOLDER
            } else {
                conversation.thread_id.as_str()
            };

            let mut lines = vec![
                Line::from(format!("selected: {}", template.label)),
                Line::from(format!("source: {}", template.source_label())),
                Line::from(format!("preview thread id: {preview_thread_id}")),
            ];

            if conversation.latest_agent_message_text().is_some() {
                lines.push(Line::from(
                    "preview last_message: using the latest non-empty agent reply",
                ));
            } else {
                lines.push(Line::from(
                    "preview last_message: placeholder until an agent reply exists",
                ));
            }
            lines.push(Line::from(preview.planning_status_line));
            if let Some(detail_line) = preview.planning_detail_line.as_deref() {
                lines.push(Line::from(detail_line.to_string()));
            }

            lines.push(Line::from(""));
            lines.push(Line::from("Raw Template"));
            for body_line in template.body.lines() {
                lines.push(Line::from(body_line.to_string()));
            }

            lines.push(Line::from(""));
            lines.push(Line::from("Rendered Preview"));
            for preview_line in preview.rendered_prompt.lines() {
                lines.push(Line::from(preview_line.to_string()));
            }

            append_planner_debug_preview_lines(&mut lines, app);
            lines
        }
    }
}

fn append_planner_debug_preview_lines(lines: &mut Vec<Line<'static>>, app: &NativeTuiApp) {
    if !app.planner_shows_debug_details() {
        return;
    }

    let planner = &app.planner_worker_panel_state;
    if planner.last_prompt.is_none() && planner.last_response.is_none() {
        return;
    }

    lines.push(Line::from(""));
    lines.push(planner_debug_header_line("Planner Session Debug"));
    lines.push(Line::from(format!(
        "last planner session: {} / {}",
        planner.last_operation_label.as_deref().unwrap_or("unknown"),
        planner.status.label()
    )));

    lines.push(planner_debug_section_header_line("Prompt"));
    append_multiline_debug_block(lines, planner.last_prompt.as_deref());

    lines.push(Line::from(""));
    lines.push(planner_debug_section_header_line("Response"));
    append_multiline_debug_block(lines, planner.last_response.as_deref());
}

fn append_multiline_debug_block(lines: &mut Vec<Line<'static>>, block: Option<&str>) {
    let Some(block) = block else {
        lines.push(Line::from("  (not available)"));
        return;
    };

    if block.trim().is_empty() {
        lines.push(Line::from("  (empty)"));
        return;
    }

    for line in build_debug_preview_lines(block, FOLLOWUP_PLANNER_DEBUG_MAX_LINES) {
        if line.is_empty() {
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(format!("  {line}")));
        }
    }
}

fn planner_debug_header_line(label: &str) -> Line<'static> {
    Line::from(Span::styled(
        label.to_string(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn planner_debug_section_header_line(label: &str) -> Line<'static> {
    Line::from(Span::styled(
        label.to_string(),
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    ))
}

pub(super) fn build_followup_template_status_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Loading => vec![Line::from("conversation is still loading")],
        ConversationState::Failed(message) => vec![Line::from(message.clone())],
        ConversationState::Ready(conversation) => {
            let turn_running = conversation.has_running_turn();
            let activity_scope = conversation
                .turn_activity
                .activity_scope_label(turn_running);
            let approval_summary = conversation.approval_summary();
            let github_review_summary =
                app.github_review_recent_changes_summary(FOLLOWUP_GITHUB_REVIEW_DETAIL_LIMIT);
            let planning_projection = app
                .planning_services
                .runtime_facade
                .build_followup_status_projection(PlanningRuntimeStatusProjectionRequest {
                    snapshot: &conversation.planning_runtime_snapshot,
                    has_running_turn: turn_running,
                    is_repairing: conversation.planning_repair_state.is_some(),
                    repair_failure_summary: conversation
                        .planning_repair_state
                        .as_ref()
                        .map(|state| state.latest_request.failure_summary.as_str()),
                    repair_attempt: conversation.planning_repair_state.as_ref().map(|state| {
                        PlanningRuntimeRepairAttempt {
                            attempts_used: state.attempts_used,
                            max_attempts: state.max_attempts,
                        }
                    }),
                    max_detail_len: FOLLOWUP_PLANNING_DETAIL_LIMIT,
                });
            let planning_status_line = planning_projection.planning_status_line;
            let repair_attempt_line = planning_projection.repair_attempt_line;
            let queue_head_line = planning_projection.queue_head_line;
            let proposal_line = planning_projection.proposal_line;
            let failure_line = planning_projection.failure_line;
            let mut lines = vec![
                Line::from(format!(
                    "auto follow-up: {} / {}",
                    conversation.auto_follow_state.status_label(),
                    conversation.auto_follow_state.activity_label()
                )),
                Line::from(format!(
                    "progress: {}",
                    conversation.auto_follow_state.completed_progress_label()
                )),
                Line::from(format!(
                    "max auto turns: {}",
                    conversation.auto_follow_state.max_auto_turns_value()
                )),
                Line::from(format!(
                    "stop keyword: {}",
                    conversation.auto_follow_state.stop_keyword_label()
                )),
                Line::from(format!(
                    "stop on no-file-change: {}",
                    conversation.auto_follow_state.no_file_change_stop_label()
                )),
                Line::from(format!(
                    "planner detail: {}",
                    app.planner_visibility_label()
                )),
                Line::from(planning_status_line),
                Line::from(format!(
                    "{activity_scope} commands: {}  |  {activity_scope} file changes: {}",
                    conversation
                        .turn_activity
                        .activity_command_count(turn_running),
                    conversation
                        .turn_activity
                        .activity_file_change_count(turn_running)
                )),
                Line::from({
                    let mut activity_line = format!(
                        "{activity_scope} tool activity: {}",
                        conversation.turn_activity.activity_summary(turn_running)
                    );
                    if let Some(approval_summary) = approval_summary.as_deref() {
                        activity_line.push_str(&format!("  |  approval: {approval_summary}"));
                    }
                    if let Some(github_review_summary) = github_review_summary.as_deref() {
                        activity_line.push_str(&format!("  |  github: {github_review_summary}"));
                    }
                    activity_line
                }),
            ];
            if let Some(started_at) = conversation.auto_follow_state.active_started_at() {
                let elapsed = std::time::Instant::now().saturating_duration_since(started_at);
                let elapsed_label = super::shell_presentation::format_elapsed(elapsed);
                lines.push(Line::from(format!(
                    "working: {}  |  elapsed: {elapsed_label}",
                    conversation.auto_follow_state.activity_label()
                )));
            }
            if let Some(repair_attempt_line) = repair_attempt_line {
                lines.push(Line::from(repair_attempt_line));
            }
            if let Some(queue_head_line) = queue_head_line {
                lines.push(Line::from(queue_head_line));
            }
            if let Some(proposal_line) = proposal_line {
                lines.push(Line::from(proposal_line));
            }
            if let Some(failure_line) = failure_line {
                lines.push(Line::from(failure_line));
            }
            if let Some(planning_notice_summary) =
                conversation.planning_notice_summary(FOLLOWUP_PLANNING_DETAIL_LIMIT)
            {
                lines.push(Line::from(planning_notice_summary));
            }
            lines.extend(
                build_planner_panel_lines(app, FOLLOWUP_PLANNER_PANEL_DETAIL_LIMIT)
                    .into_iter()
                    .map(Line::from),
            );

            if app.is_max_auto_turns_editing() {
                lines.push(Line::from(format!(
                    "editing max auto turns: {}  |  Enter save  |  Esc/Ctrl+C cancel",
                    app.followup_overlay_ui_state.max_auto_turns_editor.buffer
                )));
            } else if app.is_stop_keyword_editing() {
                lines.push(Line::from(format!(
                    "editing stop keyword: {}  |  Enter save  |  Esc/Ctrl+C cancel",
                    app.followup_overlay_ui_state.stop_keyword_editor.buffer
                )));
            } else {
                lines.push(Line::from(
                    "edit controls: Ctrl+l max turns  |  Ctrl+g stop keyword  |  Ctrl+b planner detail",
                ));
            }
            lines.push(Line::from(Span::styled(
                match conversation
                    .runtime_notice_summary(FOLLOWUP_RUNTIME_NOTICE_DETAIL_LIMIT)
                    .as_deref()
                {
                    Some(runtime_notice_summary) => format!(
                        "status: {}  |  {}  |  {}",
                        conversation.status_text,
                        conversation.warning_summary(FOLLOWUP_WARNING_DETAIL_LIMIT),
                        runtime_notice_summary,
                    ),
                    None => format!(
                        "status: {}  |  {}",
                        conversation.status_text,
                        conversation.warning_summary(FOLLOWUP_WARNING_DETAIL_LIMIT),
                    ),
                },
                Style::default().fg(Color::Yellow),
            )));

            lines
        }
    }
}
