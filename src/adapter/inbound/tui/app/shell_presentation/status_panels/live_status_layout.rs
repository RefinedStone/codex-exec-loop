use ratatui::layout::Rect;
use ratatui::widgets::{Paragraph, Wrap};

use super::super::prompt_composer::{build_prompt_cursor_offset, wrapped_row_count};
use super::super::{ConversationScreenModel, Line, ShellConversationState, ShellOverlay};
use super::tail_copy::{
    QUEUE_RECEIPT_UNDO_ACTION_LABEL, build_inline_tail_lines_with_context,
    build_inline_tail_prompt_lines_with_context,
};

const INLINE_TAIL_NOTICE_PREFIX_WIDTH: usize = "notice: ".len();
const INLINE_TAIL_MAX_NOTICE_DETAIL_LIMIT: usize = 160;

// InlineTailView is the renderer-facing plan for the live status tail.
// It keeps text lines, cursor placement, and startup anchoring together so rendering uses one coherent snapshot.
#[derive(Clone)]
pub(crate) struct InlineTailView {
    // Status, notice, planning detail, and prompt lines in final draw order.
    pub(crate) lines: Vec<Line<'static>>,
    // Cursor offset relative to the tail area; None means the renderer should not move the terminal cursor.
    pub(crate) prompt_cursor_offset: Option<(u16, u16)>,
    // Startup mode renders this block from the top instead of pinning it to the bottom.
    pub(crate) render_from_top: bool,
    // Mouse target relative to the rendered tail body. The renderer translates it into terminal coordinates.
    pub(crate) queue_receipt_undo_hit_area: Option<Rect>,
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
    let mut lines = build_inline_tail_lines_with_context(
        screen_model,
        screen_model.github_review_recent_changes_summary.clone(),
        notice_detail_limit,
    );
    lines = compact_inspection_tail_lines(screen_model, content_width, lines);

    let queue_receipt_undo_hit_area =
        find_inline_action_hit_area(&lines, content_width, QUEUE_RECEIPT_UNDO_ACTION_LABEL);

    // Cursor placement depends on the actual line stack because status/notice rows before the prompt can wrap.
    let prompt_cursor_offset =
        build_inline_prompt_cursor_offset_for_lines(screen_model, content_width, &lines);

    InlineTailView {
        lines,
        prompt_cursor_offset,
        render_from_top: screen_model.startup_screen_is_active(),
        queue_receipt_undo_hit_area,
    }
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
    lines: Vec<Line<'static>>,
) -> Vec<Line<'static>> {
    const MAX_INSPECTION_TAIL_ROWS: usize = 6;
    const MAX_ACTIVITY_TAIL_ROWS: usize = 4;
    if content_width == 0
        || screen_model.shell_overlay == ShellOverlay::Hidden
        || screen_model.startup_screen_is_active()
    {
        return lines;
    }

    let prompt_lines = build_inline_tail_prompt_lines_with_context(screen_model);
    if prompt_lines.is_empty() || lines.len() <= prompt_lines.len() {
        return lines;
    }

    let prompt_start_index = lines.len().saturating_sub(prompt_lines.len());
    let prefix_lines = &lines[..prompt_start_index];
    let prompt_rows = rendered_rows(&prompt_lines, content_width);
    let compact_activity_tail =
        screen_model.shell_overlay == ShellOverlay::Activity && content_width <= 48;
    let max_tail_rows = if compact_activity_tail {
        MAX_ACTIVITY_TAIL_ROWS
    } else {
        MAX_INSPECTION_TAIL_ROWS
    };
    if prompt_rows >= max_tail_rows {
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
    priority_lines.sort_by_key(|(_, line)| compact_tail_priority(line));
    let mut selected_lines = Vec::new();
    let mut used_prefix_rows = 0usize;
    for (index, line) in priority_lines {
        let line_rows = wrapped_row_count(line.width(), content_width);
        if used_prefix_rows.saturating_add(line_rows) > prefix_row_budget {
            continue;
        }
        selected_lines.push((index, line));
        used_prefix_rows = used_prefix_rows.saturating_add(line_rows);
    }
    selected_lines.sort_by_key(|(index, _)| *index);
    let mut compacted = selected_lines
        .into_iter()
        .map(|(_, line)| line.clone())
        .collect::<Vec<_>>();
    compacted.extend(prompt_lines);
    compacted
}

fn compact_tail_priority(line: &Line<'_>) -> u8 {
    let text = line.to_string();
    if text.starts_with(QUEUE_RECEIPT_UNDO_ACTION_LABEL)
        || text.starts_with("COMPLETE")
        || text.contains("approval: decision")
    {
        return 0;
    }
    if text.starts_with("notice: activity: terminal:")
        || text.starts_with("notice: activity: term:")
        || matches!(
            text.strip_prefix("notice: "),
            Some("recover" | "interrupt" | "failed" | "unknown" | "runtime-fail")
        )
    {
        return 1;
    }
    if text.starts_with("runtime:")
        || text.starts_with("warn:")
        || text.starts_with("startup:")
        || text.starts_with("parallel alert:")
        || text.starts_with("planning notice:")
        || text.starts_with("planning: unavailable")
        || text.starts_with("planning: invalid")
        || text.starts_with("planning: stale")
        || text.contains("blocked:") && !text.contains("blocked: none")
    {
        return 2;
    }
    if text.starts_with('◦') {
        return 3;
    }
    if text.starts_with("notice: activity:") {
        return 4;
    }
    if text.starts_with("Akra") {
        return 5;
    }
    6
}

fn rendered_rows(lines: &[Line<'static>], content_width: u16) -> usize {
    Paragraph::new(lines.to_vec())
        .wrap(Wrap { trim: false })
        .line_count(content_width)
}

// Convert the prompt-local cursor into a tail-local cursor.
// Every wrapped row before the prompt becomes vertical offset that must be added to the prompt composer result.
fn build_inline_prompt_cursor_offset_for_lines(
    screen_model: &ConversationScreenModel<'_>,
    // Tail content width is the common basis for both wrapping and prompt cursor composition.
    content_width: u16,
    // Final display lines; we count wrapped rows before the prompt suffix inside this slice.
    tail_lines: &[Line<'static>],
) -> Option<(u16, u16)> {
    if !screen_model.prompt_input_has_focus {
        return None;
    }
    // Only a ready conversation owns a reliable input buffer cursor.
    let ShellConversationState::Ready(conversation) = screen_model.conversation_state else {
        return None;
    };

    // Rebuild only the prompt suffix to find where that suffix begins in the already assembled tail.
    let prompt_lines = build_inline_tail_prompt_lines_with_context(screen_model);
    // Saturating subtraction keeps degraded state from slicing before the beginning of tail_lines.
    let prompt_start_index = tail_lines.len().saturating_sub(prompt_lines.len());

    // Count physical terminal rows before the prompt, not logical Line entries.
    let prompt_start_row = rendered_rows(&tail_lines[..prompt_start_index], content_width)
        .try_into()
        .unwrap_or(u16::MAX);

    // Prompt composer returns cursor coordinates relative to the prompt text alone.
    let (cursor_x, cursor_y) = build_prompt_cursor_offset(conversation, content_width)?;

    // Add pre-prompt rows to reach tail-local coordinates, saturating for extremely tall notice stacks.
    Some((cursor_x, prompt_start_row.saturating_add(cursor_y)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::shell_presentation::{
        ConversationScreenModel, build_inline_live_transcript_lines,
    };
    use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;
    use crate::adapter::inbound::tui::app::{ConversationState, TuiLanguage};

    #[test]
    fn one_screen_model_produces_stable_cjk_copy_layout_and_live_lines() {
        let mut app = test_native_tui_app();
        app.tui_language = TuiLanguage::Korean;
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should keep a ready conversation");
        };
        conversation.input_buffer = "한글 prompt".to_string();
        conversation.set_input_cursor_byte_index("한글".len());

        let screen_model = ConversationScreenModel::from_app(&app);
        let first = build_inline_tail_view(&screen_model, 80);
        let second = build_inline_tail_view(&screen_model, 80);
        let first_live = build_inline_live_transcript_lines(&screen_model);
        let second_live = build_inline_live_transcript_lines(&screen_model);

        assert_eq!(first.lines, second.lines);
        assert_eq!(first.prompt_cursor_offset, second.prompt_cursor_offset);
        assert_eq!(first.render_from_top, second.render_from_top);
        assert_eq!(
            first.queue_receipt_undo_hit_area,
            second.queue_receipt_undo_hit_area
        );
        assert_eq!(first_live, second_live);
        assert_eq!(
            screen_model.core_revision,
            app.core_runtime.snapshot().revision
        );
    }
}
