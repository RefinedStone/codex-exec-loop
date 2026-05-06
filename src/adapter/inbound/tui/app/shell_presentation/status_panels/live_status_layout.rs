use super::super::prompt_composer::{build_prompt_cursor_offset, wrapped_row_count};
use super::super::{
    INLINE_TAIL_NOTICE_DETAIL_LIMIT, Line, NativeTuiApp, ShellConversationState,
    ShellCorePresentationContext, ShellOverlay,
};
use super::tail_copy::{
    build_inline_tail_lines_with_context, build_inline_tail_prompt_lines_with_context,
};

// InlineTailView is the renderer-facing plan for the live status tail.
// It keeps text lines, cursor placement, and startup anchoring together so rendering uses one coherent snapshot.
#[derive(Clone)]
pub(crate) struct InlineTailView {
    // Status, notice, planner, and prompt lines in final draw order.
    pub(crate) lines: Vec<Line<'static>>,
    // Cursor offset relative to the tail area; None means the renderer should not move the terminal cursor.
    pub(crate) prompt_cursor_offset: Option<(u16, u16)>,
    // Startup mode renders this block from the top instead of pinning it to the bottom.
    pub(crate) render_from_top: bool,
}

// Build the tail text and cursor plan from the same presentation context.
// This avoids a frame where copy says one shell state while cursor math assumes another.
pub(crate) fn build_inline_tail_view(app: &NativeTuiApp, content_width: u16) -> InlineTailView {
    // The context narrows NativeTuiApp to the shell state needed by both tail copy and cursor layout.
    let context = ShellCorePresentationContext::from_app(app);
    let mut lines = build_inline_tail_lines_with_context(
        app,
        &context,
        app.github_review_recent_changes_summary(INLINE_TAIL_NOTICE_DETAIL_LIMIT),
    );
    lines = compact_inspection_tail_lines(app, &context, content_width, lines);

    // Cursor placement depends on the actual line stack because status/notice rows before the prompt can wrap.
    let prompt_cursor_offset =
        build_inline_prompt_cursor_offset_for_lines(app, &context, content_width, &lines);

    InlineTailView {
        lines,
        prompt_cursor_offset,
        render_from_top: context.startup_screen_is_active(),
    }
}

fn compact_inspection_tail_lines(
    app: &NativeTuiApp,
    context: &ShellCorePresentationContext<'_>,
    content_width: u16,
    lines: Vec<Line<'static>>,
) -> Vec<Line<'static>> {
    const MAX_INSPECTION_TAIL_ROWS: usize = 6;
    if content_width == 0
        || (app.shell_overlay == ShellOverlay::Hidden && !app.is_exit_confirmation_visible())
        || context.startup_screen_is_active()
    {
        return lines;
    }

    let prompt_lines =
        build_inline_tail_prompt_lines_with_context(app, context, app.shell_action_availability());
    if prompt_lines.is_empty() || lines.len() <= prompt_lines.len() {
        return lines;
    }

    let prompt_start_index = lines.len().saturating_sub(prompt_lines.len());
    let prefix_lines = &lines[..prompt_start_index];
    let prompt_rows = rendered_rows(&prompt_lines, content_width);
    if prompt_rows >= MAX_INSPECTION_TAIL_ROWS {
        return prompt_lines
            .into_iter()
            .rev()
            .scan(0usize, |rows, line| {
                let next_rows = rows.saturating_add(wrapped_row_count(line.width(), content_width));
                if next_rows > MAX_INSPECTION_TAIL_ROWS {
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

    let prefix_row_budget = MAX_INSPECTION_TAIL_ROWS - prompt_rows;
    let mut compacted = Vec::new();
    let mut used_prefix_rows = 0usize;
    for line in prefix_lines {
        let line_rows = wrapped_row_count(line.width(), content_width);
        if used_prefix_rows.saturating_add(line_rows) > prefix_row_budget {
            break;
        }
        compacted.push(line.clone());
        used_prefix_rows = used_prefix_rows.saturating_add(line_rows);
    }
    compacted.extend(prompt_lines);
    compacted
}

fn rendered_rows(lines: &[Line<'static>], content_width: u16) -> usize {
    lines
        .iter()
        .map(|line| wrapped_row_count(line.width(), content_width))
        .sum()
}

// Convert the prompt-local cursor into a tail-local cursor.
// Every wrapped row before the prompt becomes vertical offset that must be added to the prompt composer result.
fn build_inline_prompt_cursor_offset_for_lines(
    app: &NativeTuiApp,
    // Shared context keeps prompt suffix reconstruction aligned with the tail lines already built.
    context: &ShellCorePresentationContext<'_>,
    // Tail content width is the common basis for both wrapping and prompt cursor composition.
    content_width: u16,
    // Final display lines; we count wrapped rows before the prompt suffix inside this slice.
    tail_lines: &[Line<'static>],
) -> Option<(u16, u16)> {
    // Only a ready conversation owns a reliable input buffer cursor.
    let ShellConversationState::Ready(conversation) = context.conversation_state else {
        return None;
    };

    // Rebuild only the prompt suffix to find where that suffix begins in the already assembled tail.
    let prompt_lines =
        build_inline_tail_prompt_lines_with_context(app, context, app.shell_action_availability());
    // Saturating subtraction keeps degraded state from slicing before the beginning of tail_lines.
    let prompt_start_index = tail_lines.len().saturating_sub(prompt_lines.len());

    // Count physical terminal rows before the prompt, not logical Line entries.
    let prompt_start_row = tail_lines[..prompt_start_index]
        .iter()
        .map(|line| wrapped_row_count(line.width(), content_width))
        .sum::<usize>()
        .try_into()
        .unwrap_or(u16::MAX);

    // Prompt composer returns cursor coordinates relative to the prompt text alone.
    let (cursor_x, cursor_y) = build_prompt_cursor_offset(conversation, content_width)?;

    // Add pre-prompt rows to reach tail-local coordinates, saturating for extremely tall notice stacks.
    Some((cursor_x, prompt_start_row.saturating_add(cursor_y)))
}
