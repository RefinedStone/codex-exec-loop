use ratatui::layout::Rect;
use ratatui::widgets::{Paragraph, Wrap};

use super::super::prompt_composer::{build_prompt_cursor_offset, wrapped_row_count};
use super::super::{ConversationScreenModel, Line, MAX_INLINE_TAIL_HEIGHT, ShellOverlay};
use super::tail_copy::{
    InlineTailLine, QUEUE_RECEIPT_UNDO_ACTION_LABEL, build_inline_tail_content_with_context,
    build_inline_tail_prompt_lines_with_context,
};

const INLINE_TAIL_NOTICE_PREFIX_WIDTH: usize = "notice: ".len();
const INLINE_TAIL_MAX_NOTICE_DETAIL_LIMIT: usize = 160;

#[derive(Clone)]
pub(crate) struct InlineComposerSurfaceView {
    pub(crate) body_lines: Vec<Line<'static>>,
    pub(crate) action_line: Line<'static>,
    pub(crate) focused: bool,
    pub(crate) cursor_offset: Option<(u16, u16)>,
}

// InlineTailView is the renderer-facing plan for the live status tail.
// It keeps text lines, cursor placement, and startup anchoring together so rendering uses one coherent snapshot.
#[derive(Clone)]
pub(crate) struct InlineTailView {
    // Status, notice, planning detail, and prompt lines in final draw order.
    pub(crate) lines: Vec<Line<'static>>,
    // Cursor offset relative to the tail area; None means the renderer should not move the terminal cursor.
    pub(crate) prompt_cursor_offset: Option<(u16, u16)>,
    // Startup stays top-anchored because an inline terminal can temporarily retain
    // its pre-resize scrollback origin. Keeping the compact HUD at the viewport
    // origin guarantees the focused composer remains on the physical screen.
    pub(crate) render_from_top: bool,
    // The prompt suffix is rendered as one semantic focus surface. `lines` remains
    // the stable flattened projection used by terminal diff/cache contracts.
    pub(crate) composer_surface: Option<InlineComposerSurfaceView>,
    pub(crate) composer_start_line_index: usize,
    // Mouse target relative to the rendered tail body. The renderer translates it into terminal coordinates.
    pub(crate) queue_receipt_undo_hit_area: Option<Rect>,
}

impl InlineTailView {
    pub(crate) fn prefix_lines(&self) -> &[Line<'static>] {
        &self.lines[..self.composer_start_line_index.min(self.lines.len())]
    }

    pub(crate) fn rendered_height(&self, content_width: u16, max_height: u16) -> u16 {
        let prefix_rows = rendered_rows(self.prefix_lines(), content_width);
        let composer_rows = self
            .composer_surface
            .as_ref()
            .map_or(0, |surface| composer_surface_height(surface, content_width));
        prefix_rows
            .saturating_add(composer_rows)
            .max(1)
            .min(usize::from(max_height)) as u16
    }
}

// Build the tail text and cursor plan from the same presentation context.
// This avoids a frame where copy says one shell state while cursor math assumes another.
pub(crate) fn build_inline_tail_view(
    screen_model: &ConversationScreenModel<'_>,
    content_width: u16,
) -> InlineTailView {
    let notice_detail_limit = usize::from(content_width)
        .saturating_sub(INLINE_TAIL_NOTICE_PREFIX_WIDTH)
        .min(INLINE_TAIL_MAX_NOTICE_DETAIL_LIMIT);
    let tail_content = build_inline_tail_content_with_context(
        screen_model,
        screen_model.github_review_recent_changes_summary.clone(),
        notice_detail_limit,
        content_width,
    );
    let lines = compact_inspection_tail_lines(screen_model, content_width, tail_content);
    let prompt_lines = build_inline_tail_prompt_lines_with_context(screen_model, content_width);
    let composer_line_count = prompt_lines.len().min(lines.len());
    let focused_composer_line_count = if screen_model.prompt_input_has_focus {
        composer_line_count
    } else {
        0
    };
    let composer_start_line_index = lines.len().saturating_sub(focused_composer_line_count);
    let composer_surface = (focused_composer_line_count >= 2).then(|| {
        let prompt_slice = &lines[composer_start_line_index..];
        let body_end = prompt_slice.len().saturating_sub(1);
        let cursor_offset = if screen_model.prompt_input_has_focus {
            screen_model.composer().and_then(|composer| {
                build_prompt_cursor_offset(composer, composer_inner_width(content_width))
            })
        } else {
            None
        };
        InlineComposerSurfaceView {
            body_lines: prompt_slice[..body_end].to_vec(),
            action_line: prompt_slice[body_end].clone(),
            focused: screen_model.prompt_input_has_focus,
            cursor_offset,
        }
    });

    let queue_receipt_undo_hit_area =
        find_inline_action_hit_area(&lines, content_width, QUEUE_RECEIPT_UNDO_ACTION_LABEL);

    // Cursor placement includes the focus rail and every wrapped status row
    // before the composer.
    let prompt_cursor_offset =
        build_inline_prompt_cursor_offset_for_lines(screen_model, content_width, &lines);

    InlineTailView {
        lines,
        prompt_cursor_offset,
        render_from_top: screen_model.startup_screen_is_active(),
        composer_surface,
        composer_start_line_index,
        queue_receipt_undo_hit_area,
    }
}

pub(crate) fn composer_inner_width(content_width: u16) -> u16 {
    content_width.saturating_sub(1).max(1)
}

fn composer_surface_height(surface: &InlineComposerSurfaceView, content_width: u16) -> usize {
    rendered_rows(&surface.body_lines, composer_inner_width(content_width)).saturating_add(2)
}

fn find_inline_action_hit_area(
    lines: &[Line<'static>],
    content_width: u16,
    action_label: &str,
) -> Option<Rect> {
    let content_width = usize::from(content_width);
    if content_width == 0 {
        return None;
    }

    let mut rendered_row = 0usize;
    for line in lines {
        let mut logical_column = 0usize;
        for span in &line.spans {
            let span_width = span.width();
            if span.content.as_ref() == action_label {
                let action_x = logical_column % content_width;
                if action_x.saturating_add(span_width) > content_width {
                    return None;
                }
                let action_y = rendered_row.saturating_add(logical_column / content_width);
                return Some(Rect::new(
                    u16::try_from(action_x).ok()?,
                    u16::try_from(action_y).ok()?,
                    u16::try_from(span_width).ok()?,
                    1,
                ));
            }
            logical_column = logical_column.saturating_add(span_width);
        }
        rendered_row = rendered_row.saturating_add(rendered_rows(
            std::slice::from_ref(line),
            content_width as u16,
        ));
    }
    None
}

fn compact_inspection_tail_lines(
    screen_model: &ConversationScreenModel<'_>,
    content_width: u16,
    lines: Vec<InlineTailLine>,
) -> Vec<Line<'static>> {
    const MAX_INSPECTION_TAIL_ROWS: usize = 6;
    const MAX_ACTIVITY_TAIL_ROWS: usize = 4;
    let is_primary_tail = screen_model.shell_overlay == ShellOverlay::Hidden;
    if content_width == 0
        || screen_model.startup_screen_is_active()
        || (is_primary_tail
            && (screen_model.parallel_mode_enabled
                || screen_model
                    .inline_history_render_mode
                    .mirrors_recent_transcript_in_tail()))
    {
        return lines.into_iter().map(|entry| entry.line).collect();
    }

    let prompt_lines = build_inline_tail_prompt_lines_with_context(screen_model, content_width);
    if prompt_lines.is_empty() || lines.len() <= prompt_lines.len() {
        return lines.into_iter().map(|entry| entry.line).collect();
    }

    let prompt_start_index = lines.len().saturating_sub(prompt_lines.len());
    let prefix_lines = &lines[..prompt_start_index];
    let prompt_rows = if prompt_lines.len() >= 2 && screen_model.prompt_input_has_focus {
        let body_end = prompt_lines.len() - 1;
        rendered_rows(
            &prompt_lines[..body_end],
            composer_inner_width(content_width),
        )
        .saturating_add(2)
    } else {
        rendered_rows(&prompt_lines, content_width)
    };
    let compact_activity_tail =
        screen_model.shell_overlay == ShellOverlay::Activity && content_width <= 48;
    let max_tail_rows = if is_primary_tail {
        usize::from(MAX_INLINE_TAIL_HEIGHT)
    } else if compact_activity_tail {
        MAX_ACTIVITY_TAIL_ROWS
    } else {
        MAX_INSPECTION_TAIL_ROWS
    };
    if prompt_rows >= max_tail_rows {
        if is_primary_tail {
            // The hidden-tail renderer keeps the focused suffix and cursor visible. Preserve the
            // complete logical prompt so it can choose that suffix without losing input text.
            return lines.into_iter().map(|entry| entry.line).collect();
        }
        return prompt_lines
            .into_iter()
            .rev()
            .scan(0usize, |rows, line| {
                let next_rows = rows.saturating_add(wrapped_row_count(line.width(), content_width));
                if next_rows > max_tail_rows {
                    None
                } else {
                    *rows = next_rows;
                    Some(line)
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
    }

    let prefix_row_budget = max_tail_rows - prompt_rows;
    let mut priority_lines = prefix_lines.iter().enumerate().collect::<Vec<_>>();
    priority_lines.sort_by_key(|(_, entry)| entry.priority);
    let inspection_has_attention_signal = !is_primary_tail
        && prefix_lines
            .iter()
            .any(|entry| entry.priority <= super::tail_copy::InlineTailPriority::Warning);
    let mut selected_lines = Vec::new();
    let mut used_prefix_rows = 0usize;
    for (index, entry) in priority_lines {
        // Inspection tails should not refill spare rows with diagnostic detail
        // while a pinned, terminal, or warning signal is asking for attention.
        // The unused row is intentional visual separation, not lost capacity.
        if inspection_has_attention_signal
            && entry.priority == super::tail_copy::InlineTailPriority::Detail
        {
            continue;
        }
        let line_rows = rendered_rows(std::slice::from_ref(&entry.line), content_width);
        if used_prefix_rows.saturating_add(line_rows) > prefix_row_budget {
            continue;
        }
        selected_lines.push((index, entry));
        used_prefix_rows = used_prefix_rows.saturating_add(line_rows);
    }
    selected_lines.sort_by_key(|(index, _)| *index);
    let mut compacted = selected_lines
        .into_iter()
        .map(|(_, entry)| entry.line.clone())
        .collect::<Vec<_>>();
    if inspection_has_attention_signal && used_prefix_rows < prefix_row_budget {
        compacted.push(Line::default());
    }
    compacted.extend(prompt_lines);
    compacted
}

fn rendered_rows(lines: &[Line<'static>], content_width: u16) -> usize {
    Paragraph::new(lines.to_vec())
        .wrap(Wrap { trim: false })
        .line_count(content_width)
}

// Convert the prompt-local cursor into a tail-local cursor. The focus rail adds
// one cell on the left and one row above the prompt body.
fn build_inline_prompt_cursor_offset_for_lines(
    screen_model: &ConversationScreenModel<'_>,
    content_width: u16,
    tail_lines: &[Line<'static>],
) -> Option<(u16, u16)> {
    if !screen_model.prompt_input_has_focus {
        return None;
    }
    let composer = screen_model.composer()?;
    let prompt_lines = build_inline_tail_prompt_lines_with_context(screen_model, content_width);
    let prompt_start_index = tail_lines.len().saturating_sub(prompt_lines.len());
    let prompt_start_row = rendered_rows(&tail_lines[..prompt_start_index], content_width)
        .min(usize::from(u16::MAX)) as u16;
    let (cursor_x, cursor_y) =
        build_prompt_cursor_offset(composer, composer_inner_width(content_width))?;

    Some((
        cursor_x.saturating_add(1),
        prompt_start_row.saturating_add(1).saturating_add(cursor_y),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::tail_copy::InlineTailPriority;
    use super::*;
    use crate::adapter::inbound::tui::app::queue_overlay_ui::QueueMutationKind;
    use crate::adapter::inbound::tui::app::shell_presentation::{
        ConversationScreenModel, QueueMutationTailState, build_inline_live_transcript_lines,
    };
    use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;
    use crate::adapter::inbound::tui::app::{
        ConversationState, ConversationViewModel, InlineHistoryRenderMode, MAX_INLINE_TAIL_HEIGHT,
        NativeTuiApp, ShellActionAvailability, TuiLanguage,
    };
    use crate::application::service::planning::PlanningRuntimeProjection;
    use crate::core::app::{
        ActiveTurnPhase, ActiveTurnSnapshot, CorePromptOrigin, PostTurnAuthoritySnapshot,
        PostTurnEvaluationCorrelation, PostTurnRouteResolution, QueueMutationCorrelation,
        QueueMutationIntent, TurnSubmissionCorrelation,
    };
    use crate::domain::planning::{PlanningWorkerPanelState, PlanningWorkerStatus};
    use std::time::Instant;

    const LOW_PRIORITY_DETAIL: &str = "LOW_PRIORITY_DETAIL";

    fn set_running_turn(conversation: &mut ConversationViewModel, turn_id: &str) {
        let mut snapshot = conversation.runtime_snapshot().clone();
        snapshot.active_turn = Some(ActiveTurnSnapshot {
            correlation: TurnSubmissionCorrelation::new(1),
            phase: ActiveTurnPhase::Running,
            workspace_directory: conversation.cwd.clone(),
            turn_id: Some(turn_id.to_string()),
            prompt_origin: CorePromptOrigin::Manual,
            started_at: Instant::now(),
        });
        conversation.apply_runtime_snapshot(snapshot);
        conversation.record_turn_started(turn_id.to_string());
    }

    fn settle_post_turn(conversation: &mut ConversationViewModel, turn_id: &str) {
        let workspace_directory = conversation.cwd.clone();
        let mut snapshot = conversation.runtime_snapshot().clone();
        snapshot.active_turn = None;
        snapshot.post_turn = PostTurnAuthoritySnapshot::Settled {
            correlation: PostTurnEvaluationCorrelation::new(
                1,
                conversation.thread_id.clone(),
                turn_id,
                workspace_directory.clone(),
                workspace_directory,
            ),
            resolution: PostTurnRouteResolution::NoContinuation,
        };
        conversation.apply_runtime_snapshot(snapshot);
    }

    fn dense_hidden_tail_app() -> NativeTuiApp {
        let mut app = test_native_tui_app();
        app.shell.tui_language = TuiLanguage::Korean;
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test app should keep a ready conversation");
        };
        conversation.record_thread_prepared(
            "thread-hidden-priority".to_string(),
            "Hidden priority".to_string(),
            "/tmp/root".to_string(),
        );
        conversation.composer.input_buffer = "우선순위가 보존된 짧은 prompt".to_string();
        conversation
            .composer
            .set_input_cursor_byte_index(conversation.composer.input_buffer.len());
        conversation.base_warnings = vec!["긴한글경고상세".repeat(10)];
        conversation.runtime_notices = vec!["긴한글복구상세".repeat(10)];
        app
    }

    fn add_dense_low_priority_details(screen_model: &mut ConversationScreenModel<'_>) {
        screen_model.shell_action_availability = ShellActionAvailability::Ready;
        screen_model.planning_worker_shows_debug_details = true;
        screen_model.planning_worker_panel_state = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RepairFailed,
            last_operation_label: Some("repair dense tail projection".to_string()),
            last_queue_summary: Some("ready task with dense framing".to_string()),
            last_summary: Some("secondary summary detail".to_string()),
            last_notice_detail: Some("secondary worker notice".to_string()),
            last_host_detail: Some("secondary host detail".to_string()),
            last_rejected_summary: Some(LOW_PRIORITY_DETAIL.to_string()),
            ..Default::default()
        };
    }

    #[test]
    fn one_screen_model_produces_stable_cjk_copy_layout_and_live_lines() {
        let mut app = test_native_tui_app();
        app.shell.tui_language = TuiLanguage::Korean;
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test app should keep a ready conversation");
        };
        conversation.composer.input_buffer = "한글 prompt".to_string();
        conversation
            .composer
            .set_input_cursor_byte_index("한글".len());

        let screen_model = ConversationScreenModel::from_app(&app);
        let first = build_inline_tail_view(&screen_model, 80);
        let second = build_inline_tail_view(&screen_model, 80);
        let first_live = build_inline_live_transcript_lines(&screen_model);
        let second_live = build_inline_live_transcript_lines(&screen_model);

        assert_eq!(first.lines, second.lines);
        assert_eq!(first.prompt_cursor_offset, second.prompt_cursor_offset);
        let prompt_index = first
            .lines
            .iter()
            .position(|line| line.to_string().starts_with(" > "))
            .expect("focused prompt should remain in the startup tail");
        assert_eq!(prompt_index, 2);
        assert_eq!(first.prompt_cursor_offset, Some((8, 3)));
        assert_eq!(first.render_from_top, second.render_from_top);
        assert_eq!(
            first.queue_receipt_undo_hit_area,
            second.queue_receipt_undo_hit_area
        );
        assert_eq!(first_live, second_live);
        assert_eq!(
            screen_model.core_revision,
            app.runtime.client_runtime.snapshot().revision
        );
    }

    #[test]
    fn viewport_replay_handoff_projection_couples_visible_release_to_delivery_ack() {
        let mut app = test_native_tui_app();
        app.shell.inline_history_render_mode = InlineHistoryRenderMode::ViewportReplay;
        app.shell.chrome.shell_overlay = ShellOverlay::Hidden;
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test app should keep a ready conversation");
        };
        conversation.record_thread_prepared(
            "thread-live".to_string(),
            "Live transcript".to_string(),
            "/tmp/root".to_string(),
        );
        set_running_turn(conversation, "turn-live");
        assert!(conversation.complete_live_agent_message(
            "agent-live".to_string(),
            Some("final_answer".to_string()),
            "HANDOFF_MARKER".to_string(),
        ));
        conversation.finish_turn("turn-live", &[]);
        conversation.begin_post_turn_settlement("turn-live");
        settle_post_turn(conversation, "turn-live");
        assert!(conversation.complete_post_turn_settlement("turn-live"));
        conversation.push_live_agent_delta(
            "agent-next".to_string(),
            Some("analysis".to_string()),
            "LIVE_MARKER".to_string(),
        );

        {
            let screen_model = ConversationScreenModel::from_app(&app);
            let live_transcript = screen_model
                .live_transcript()
                .expect("ready conversation should retain a live transcript projection");
            let rendered = build_inline_live_transcript_lines(&screen_model)
                .into_iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n");

            assert!(live_transcript.handoff_pending);
            assert!(live_transcript.acknowledge_handoff_after_successful_draw);
            assert!(screen_model.renders_viewport_transcript_handoff());
            assert!(!screen_model.renders_parallel_viewport_handoff());
            assert_eq!(rendered.matches("HANDOFF_MARKER").count(), 1);
            assert_eq!(rendered.matches("LIVE_MARKER").count(), 1);
            let handoff_index = rendered
                .find("HANDOFF_MARKER")
                .expect("rendered transcript should retain the handoff marker");
            let live_index = rendered
                .find("LIVE_MARKER")
                .expect("rendered transcript should retain the live marker");
            assert!(handoff_index < live_index);
        }

        app.shell.chrome.shell_overlay = ShellOverlay::Help;
        {
            let overlay_screen_model = ConversationScreenModel::from_app(&app);
            assert!(!overlay_screen_model.renders_viewport_transcript_handoff());
            assert_eq!(
                build_inline_live_transcript_lines(&overlay_screen_model)
                    .into_iter()
                    .map(|line| line.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
                    .matches("HANDOFF_MARKER")
                    .count(),
                1
            );
        }

        app.conversation.lifecycle.conversation_state = ConversationState::Loading;
        let loading_screen_model = ConversationScreenModel::from_app(&app);
        assert!(loading_screen_model.live_transcript().is_none());
        assert!(build_inline_live_transcript_lines(&loading_screen_model).is_empty());
    }

    #[test]
    fn screen_model_hides_a_planning_projection_from_another_workspace() {
        let mut app = test_native_tui_app();
        let projection = PlanningRuntimeProjection::ready(
            "root prompt".to_string(),
            "root queue".to_string(),
            None,
        );
        app.sync_ready_conversation_planning_runtime_projection(projection.clone());
        assert_eq!(
            ConversationScreenModel::from_app(&app).planning_runtime_projection,
            projection
        );
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test app should keep a ready conversation");
        };
        conversation.cwd = "/tmp/other-workspace".to_string();
        conversation.draft_workspace_directory = "/tmp/other-workspace".to_string();

        let screen_model = ConversationScreenModel::from_app(&app);

        assert_eq!(
            screen_model.planning_runtime_projection,
            PlanningRuntimeProjection::uninitialized()
        );
    }

    #[test]
    fn korean_pending_queue_keeps_semantic_priority_in_modal_tail() {
        const WIDTH: u16 = 80;
        const LOW_DETAIL: &str = "낮은상세표시";
        let mut app = test_native_tui_app();
        app.shell.tui_language = TuiLanguage::Korean;
        app.shell.chrome.shell_overlay = ShellOverlay::Queue;
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test app should keep a ready conversation");
        };
        conversation.record_thread_prepared(
            "thread-tail".to_string(),
            "Semantic tail".to_string(),
            "/tmp/root".to_string(),
        );
        conversation.status_text = LOW_DETAIL.to_string();
        conversation.base_warnings = vec!["긴한글경고상세".repeat(20)];
        conversation.runtime_notices = vec!["긴한글복구상세".repeat(20)];

        let context = app.current_queue_mutation_context();
        app.planning
            .queue_mutation_ui_state
            .record_started(QueueMutationCorrelation::new(
                1,
                QueueMutationIntent {
                    workspace_directory: context.workspace_directory,
                    active_thread_id: context.active_thread_id,
                    kind: QueueMutationKind::RemoveSelected,
                    expected_planning_revision: 1,
                    targets: Vec::new(),
                    receipt_at_start: None,
                },
            ));

        let mut screen_model = ConversationScreenModel::from_app(&app);
        screen_model.shell_action_availability = ShellActionAvailability::Ready;
        let raw = build_inline_tail_content_with_context(&screen_model, None, 72, 80);
        let attention = raw
            .iter()
            .map(|entry| &entry.line)
            .find(|line| line.to_string().starts_with('!'))
            .expect("aggregated attention should remain in the raw semantic tail");
        assert_eq!(wrapped_row_count(attention.width(), WIDTH), 1);
        assert!(!attention.to_string().contains("Ctrl+D"));

        let tail_view = build_inline_tail_view(&screen_model, WIDTH);
        let rendered = tail_view
            .lines
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            rendered.contains("큐: op-1  |  권한 확인 대기 중"),
            "{rendered}"
        );
        assert!(rendered.contains("작업 입력  |  : 명령"), "{rendered}");
        assert!(!rendered.contains("Ctrl+D"), "{rendered}");
        assert!(!rendered.contains("runtime:"), "{rendered}");
        assert!(!rendered.contains(LOW_DETAIL), "{rendered}");
        assert_eq!(tail_view.rendered_height(WIDTH, 6), 6);
        assert!(tail_view.prompt_cursor_offset.is_none());
        assert!(tail_view.queue_receipt_undo_hit_area.is_none());
    }

    #[test]
    fn running_stale_planning_keeps_warning_priority_in_modal_tail() {
        const WIDTH: u16 = 80;
        let mut app = test_native_tui_app();
        app.shell.chrome.shell_overlay = ShellOverlay::Queue;
        app.sync_ready_conversation_planning_runtime_projection(
            PlanningRuntimeProjection::ready_with_details(
                "Planning Context".to_string(),
                "now: none  |  next: none  |  proposed: none  |  blocked: none".to_string(),
                None,
                None,
            )
            .with_workspace_present(true),
        );
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test app should keep a ready conversation");
        };
        conversation.record_thread_prepared(
            "thread-stale-tail".to_string(),
            "Stale planning tail".to_string(),
            "/tmp/root".to_string(),
        );
        set_running_turn(conversation, "turn-stale-tail");
        conversation.base_warnings = vec!["긴한글경고상세".repeat(20)];
        conversation.runtime_notices = vec!["긴한글복구상세".repeat(20)];

        let mut screen_model = ConversationScreenModel::from_app(&app);
        screen_model.shell_action_availability = ShellActionAvailability::Ready;
        let raw = build_inline_tail_content_with_context(&screen_model, None, 72, 80);
        let stale = raw
            .iter()
            .find(|entry| entry.line.to_string() == "planning: stale")
            .expect("running turn should project stale planning");
        assert_eq!(stale.priority, InlineTailPriority::Warning);

        let tail_view = build_inline_tail_view(&screen_model, WIDTH);
        let rendered = tail_view
            .lines
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!rendered.contains("Ctrl+D"), "{rendered}");
        assert!(!rendered.contains("runtime:"), "{rendered}");
        assert!(rendered.contains("planning: stale"), "{rendered}");
        assert!(rendered.contains("Akra"), "{rendered}");
        assert_eq!(tail_view.rendered_height(WIDTH, 6), 6);
    }

    #[test]
    fn hidden_host_scrollback_keeps_pinned_queue_warning_prompt_and_undo_target() {
        for width in [80, 120] {
            for (queue_state, expected_queue_copy, expects_undo_target) in [
                (
                    QueueMutationTailState::Pending(7),
                    "큐: op-7  |  권한 확인 대기 중",
                    false,
                ),
                (
                    QueueMutationTailState::UndoAvailable(1),
                    QUEUE_RECEIPT_UNDO_ACTION_LABEL,
                    true,
                ),
            ] {
                let app = dense_hidden_tail_app();
                let mut screen_model = ConversationScreenModel::from_app(&app);
                add_dense_low_priority_details(&mut screen_model);
                screen_model.queue_mutation_tail_state = queue_state;
                assert_eq!(screen_model.shell_overlay, ShellOverlay::Hidden);
                assert!(!screen_model.parallel_mode_enabled);
                assert!(
                    !screen_model
                        .inline_history_render_mode
                        .mirrors_recent_transcript_in_tail()
                );

                let raw_lines =
                    build_inline_tail_content_with_context(&screen_model, None, 72, width)
                        .into_iter()
                        .map(|entry| entry.line)
                        .collect::<Vec<_>>();
                assert!(
                    rendered_rows(&raw_lines, width) > usize::from(MAX_INLINE_TAIL_HEIGHT),
                    "fixture must exceed the renderer tail budget at width {width}"
                );

                let tail_view = build_inline_tail_view(&screen_model, width);
                let rendered = tail_view
                    .lines
                    .iter()
                    .map(Line::to_string)
                    .collect::<Vec<_>>()
                    .join("\n");

                assert!(
                    rendered_rows(&tail_view.lines, width) <= usize::from(MAX_INLINE_TAIL_HEIGHT),
                    "{rendered}"
                );
                assert!(rendered.contains(expected_queue_copy), "{rendered}");
                assert!(rendered.contains("Ctrl+D"), "{rendered}");
                assert!(!rendered.contains("runtime:"), "{rendered}");
                assert!(
                    rendered.contains("우선순위가 보존된 짧은 prompt"),
                    "{rendered}"
                );
                assert!(!rendered.contains(LOW_PRIORITY_DETAIL), "{rendered}");
                assert_eq!(
                    tail_view.queue_receipt_undo_hit_area.is_some(),
                    expects_undo_target,
                    "{rendered}"
                );
                if let Some(hit_area) = tail_view.queue_receipt_undo_hit_area {
                    assert_eq!(hit_area.width, QUEUE_RECEIPT_UNDO_ACTION_LABEL.len() as u16);
                    assert_eq!(hit_area.height, 1);
                }
                let (_, cursor_y) = tail_view
                    .prompt_cursor_offset
                    .expect("focused prompt must keep its cursor");
                assert!(cursor_y < MAX_INLINE_TAIL_HEIGHT, "{rendered}");
            }
        }
    }

    #[test]
    fn hidden_host_scrollback_budgets_prefix_with_ratatui_word_wrap() {
        const WIDTH: u16 = 48;
        const PINNED_QUEUE: &str = "PINNED_QUEUE";
        let app = dense_hidden_tail_app();
        let screen_model = ConversationScreenModel::from_app(&app);
        let adversarial_line = |marker: char| {
            let word = marker.to_string().repeat(25);
            Line::from(format!("{word} {word} {word}"))
        };
        let dropped_detail = adversarial_line('C');
        assert_eq!(wrapped_row_count(dropped_detail.width(), WIDTH), 2);
        assert_eq!(
            rendered_rows(std::slice::from_ref(&dropped_detail), WIDTH),
            3,
            "fixture must exercise Ratatui word wrapping rather than width division"
        );

        let mut content = vec![InlineTailLine {
            line: Line::from(PINNED_QUEUE),
            priority: InlineTailPriority::Pinned,
        }];
        content.extend(['A', 'B', 'C'].map(|marker| InlineTailLine {
            line: adversarial_line(marker),
            priority: InlineTailPriority::Detail,
        }));
        content.extend(
            build_inline_tail_prompt_lines_with_context(&screen_model, WIDTH)
                .into_iter()
                .map(|line| InlineTailLine {
                    line,
                    priority: InlineTailPriority::Pinned,
                }),
        );

        let compacted = compact_inspection_tail_lines(&screen_model, WIDTH, content);
        let rendered = compacted
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            rendered_rows(&compacted, WIDTH) <= usize::from(MAX_INLINE_TAIL_HEIGHT),
            "{rendered}"
        );
        assert!(rendered.contains(PINNED_QUEUE), "{rendered}");
        assert!(
            !rendered.contains(&dropped_detail.to_string()),
            "{rendered}"
        );
    }

    #[test]
    fn hidden_host_scrollback_leaves_an_over_budget_prompt_to_the_suffix_renderer() {
        const WIDTH: u16 = 48;
        let mut app = dense_hidden_tail_app();
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test app should keep a ready conversation");
        };
        conversation.composer.input_buffer = format!("{}PROMPT_END", "긴 prompt ".repeat(120));
        conversation
            .composer
            .set_input_cursor_byte_index(conversation.composer.input_buffer.len());
        let mut screen_model = ConversationScreenModel::from_app(&app);
        add_dense_low_priority_details(&mut screen_model);
        let raw_lines = build_inline_tail_content_with_context(&screen_model, None, 40, 48)
            .into_iter()
            .map(|entry| entry.line)
            .collect::<Vec<_>>();
        let prompt_lines = build_inline_tail_prompt_lines_with_context(&screen_model, WIDTH);
        assert!(rendered_rows(&prompt_lines, WIDTH) >= usize::from(MAX_INLINE_TAIL_HEIGHT));

        let tail_view = build_inline_tail_view(&screen_model, WIDTH);

        assert_eq!(tail_view.lines, raw_lines);
        assert!(tail_view.prompt_cursor_offset.is_some());
    }
}
