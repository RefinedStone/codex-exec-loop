use ratatui::layout::Rect;
use ratatui::widgets::{Paragraph, Wrap};

use super::super::prompt_composer::{build_prompt_cursor_offset, wrapped_row_count};
use super::super::{ConversationScreenModel, Line, MAX_SHELL_TAIL_HEIGHT, ShellOverlay};
use super::tail_copy::{
    QUEUE_RECEIPT_UNDO_ACTION_LABEL, ShellTailLine, build_shell_tail_content_with_context,
    build_shell_tail_prompt_lines_with_context,
};

const SHELL_TAIL_NOTICE_PREFIX_WIDTH: usize = "notice: ".len();
const SHELL_TAIL_MAX_NOTICE_DETAIL_LIMIT: usize = 160;

#[derive(Clone)]
pub(crate) struct ComposerSurfaceView {
    pub(crate) body_lines: Vec<Line<'static>>,
    pub(crate) action_line: Line<'static>,
    pub(crate) focused: bool,
    pub(crate) cursor_offset: Option<(u16, u16)>,
}

// ShellTailView is the renderer-facing plan for the live status tail.
// It keeps text lines, cursor placement, and startup anchoring together so rendering uses one coherent snapshot.
#[derive(Clone)]
pub(crate) struct ShellTailView {
    // Status, notice, planning detail, and prompt lines in final draw order.
    pub(crate) lines: Vec<Line<'static>>,
    // Cursor offset relative to the tail area; None means the renderer should not move the terminal cursor.
    pub(crate) prompt_cursor_offset: Option<(u16, u16)>,
    // Startup stays top-anchored so a short fullscreen viewport always keeps the
    // compact HUD and focused composer inside the physical screen.
    pub(crate) render_from_top: bool,
    // The prompt suffix is rendered as one semantic focus surface. `lines` remains
    // the stable flattened projection used by terminal diff/cache contracts.
    pub(crate) composer_surface: Option<ComposerSurfaceView>,
    pub(crate) composer_start_line_index: usize,
    // Mouse target relative to the rendered tail body. The renderer translates it into terminal coordinates.
    pub(crate) queue_receipt_undo_hit_area: Option<Rect>,
}

impl ShellTailView {
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
pub(crate) fn build_shell_tail_view(
    screen_model: &ConversationScreenModel<'_>,
    content_width: u16,
) -> ShellTailView {
    let notice_detail_limit = usize::from(content_width)
        .saturating_sub(SHELL_TAIL_NOTICE_PREFIX_WIDTH)
        .min(SHELL_TAIL_MAX_NOTICE_DETAIL_LIMIT);
    let tail_content = build_shell_tail_content_with_context(
        screen_model,
        screen_model.github_review_recent_changes_summary.clone(),
        notice_detail_limit,
        content_width,
    );
    let lines = compact_inspection_tail_lines(screen_model, content_width, tail_content);
    let prompt_lines = build_shell_tail_prompt_lines_with_context(screen_model, content_width);
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
        ComposerSurfaceView {
            body_lines: prompt_slice[..body_end].to_vec(),
            action_line: prompt_slice[body_end].clone(),
            focused: screen_model.prompt_input_has_focus,
            cursor_offset,
        }
    });

    let queue_receipt_undo_hit_area =
        find_inline_action_hit_area(&lines, content_width, QUEUE_RECEIPT_UNDO_ACTION_LABEL);

    // Cursor placement includes the composer frame and every wrapped status row
    // before the composer.
    let prompt_cursor_offset =
        build_inline_prompt_cursor_offset_for_lines(screen_model, content_width, &lines);

    ShellTailView {
        lines,
        prompt_cursor_offset,
        render_from_top: screen_model.startup_screen_is_active(),
        composer_surface,
        composer_start_line_index,
        queue_receipt_undo_hit_area,
    }
}

pub(crate) fn composer_inner_width(content_width: u16) -> u16 {
    content_width.saturating_sub(2).max(1)
}

fn composer_surface_height(surface: &ComposerSurfaceView, content_width: u16) -> usize {
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
    lines: Vec<ShellTailLine>,
) -> Vec<Line<'static>> {
    const MAX_INSPECTION_TAIL_ROWS: usize = 6;
    const MAX_ACTIVITY_TAIL_ROWS: usize = 4;
    let is_primary_tail = screen_model.shell_overlay == ShellOverlay::Hidden;
    if content_width == 0
        || screen_model.startup_screen_is_active()
        || (is_primary_tail && screen_model.parallel_mode_enabled)
    {
        return lines.into_iter().map(|entry| entry.line).collect();
    }

    let prompt_lines = build_shell_tail_prompt_lines_with_context(screen_model, content_width);
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
        usize::from(MAX_SHELL_TAIL_HEIGHT)
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
            .any(|entry| entry.priority <= super::tail_copy::ShellTailPriority::Warning);
    let mut selected_lines = Vec::new();
    let mut used_prefix_rows = 0usize;
    for (index, entry) in priority_lines {
        // Inspection tails should not refill spare rows with diagnostic detail
        // while a pinned, terminal, or warning signal is asking for attention.
        // The unused row is intentional visual separation, not lost capacity.
        if inspection_has_attention_signal
            && entry.priority == super::tail_copy::ShellTailPriority::Detail
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

// Convert the prompt-local cursor into a tail-local cursor. The composer frame adds one cell on
// the left and one row above the prompt body.
fn build_inline_prompt_cursor_offset_for_lines(
    screen_model: &ConversationScreenModel<'_>,
    content_width: u16,
    tail_lines: &[Line<'static>],
) -> Option<(u16, u16)> {
    if !screen_model.prompt_input_has_focus {
        return None;
    }
    let composer = screen_model.composer()?;
    let prompt_lines = build_shell_tail_prompt_lines_with_context(screen_model, content_width);
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
