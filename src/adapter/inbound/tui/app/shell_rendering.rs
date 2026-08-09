use std::rc::Rc;

use super::fullscreen_frame_model::ApprovalFullscreenScreenModel;
pub(super) use super::fullscreen_frame_model::{
    FullscreenConversationFrameProjection, FullscreenFrameRenderReceipt,
    FullscreenInspectionFrameModel, FullscreenShellFrameModel,
};
#[cfg(test)]
use super::fullscreen_frame_model::{
    apply_fullscreen_frame_render_receipt, capture_fullscreen_shell_frame_model,
};
#[cfg(test)]
use super::shell_presentation::{
    ConversationTranscriptLineInteraction, ConversationTranscriptView,
};
use super::shell_presentation::{
    ConversationTranscriptLineSurface, TurnSteerConfirmationScreenModel,
};
#[cfg(test)]
use super::*;
use super::{
    AkraTheme, SharedTranscriptCardDigests, ShellFrontendMode, ShellOverlay, TranscriptCardHitArea,
    TranscriptRenderedRow, TranscriptViewportFrame, TranscriptViewportUiState,
};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

/*
 * 이 파일은 native fullscreen shell의 ratatui frame boundary다.
 * presentation layer가 Line 기반 read model을 만들고, fullscreen_layout이 frame 분할을 정하면,
 * 이 module은 app-owned conversation viewport, 선택적 fullscreen inspection, exit confirmation modal 순서로 layer를 적용한다.
 */
#[path = "shell_rendering/fullscreen_inspection.rs"]
mod fullscreen_inspection;
#[path = "shell_rendering/fullscreen_layout.rs"]
mod fullscreen_layout;
#[path = "shell_rendering/transcript_document.rs"]
mod transcript_document;

use fullscreen_inspection::draw_fullscreen_shell_inspection;
use fullscreen_layout::{
    build_fullscreen_flow_layout, centered_fixed_rect, fullscreen_tail_render_area,
    render_body_suffix, set_cursor_if_visible,
};
pub(in crate::adapter::inbound::tui::app) use fullscreen_layout::{
    count_wrapped_rows, fullscreen_section_height,
};
pub(super) use transcript_document::FullscreenTranscriptDocument;
use transcript_document::TranscriptWrappedRowLayout;
#[cfg(test)]
use transcript_document::transcript_wrapped_row_layout;

pub(super) fn fullscreen_parallel_event_stream_area(
    projection: &FullscreenConversationFrameProjection,
    frame_area: Rect,
) -> Rect {
    if !projection.parallel_mode_enabled && projection.shell_overlay != ShellOverlay::Supersession {
        return Rect::default();
    }

    let layout = build_fullscreen_flow_layout(projection, frame_area, &projection.tail_view.lines);
    projection
        .supersession_overlay_view
        .as_deref()
        .map_or(Rect::default(), |view| {
            fullscreen_inspection::parallel_event_stream_area(view, layout[0])
        })
}

#[cfg(test)]
pub(super) fn draw(frame: &mut Frame<'_>, app: &mut NativeTuiApp, mode: ShellFrontendMode) {
    let projection = FullscreenConversationFrameProjection::from_app(app, frame.area().width);
    let model = capture_fullscreen_shell_frame_model(app, mode, frame.area(), projection);
    let receipt = draw_projected(frame, mode, model);
    assert!(apply_fullscreen_frame_render_receipt(app, receipt));
}

pub(super) fn draw_projected(
    frame: &mut Frame<'_>,
    mode: ShellFrontendMode,
    model: FullscreenShellFrameModel,
) -> FullscreenFrameRenderReceipt {
    let (mut projection, inspection, mut receipt) = model.into_parts();
    // 현재 native shell renderer는 하나뿐이지만, mode 인자를 유지해 app runtime과 shell frontend 추상화를 한 경계에서 묶는다.
    let _ = mode;
    let frame_area = frame.area();
    // tail view는 status/prompt line과 cursor offset을 함께 담는다.
    // 같은 tail 높이가 fullscreen inspection/body 분할 기준도 된다.
    let layout = build_fullscreen_flow_layout(&projection, frame_area, &projection.tail_view.lines);
    let transcript_scroll_offset = if projection.shell_overlay == ShellOverlay::Hidden {
        let content_rows = projection.transcript_document.content_rows();
        receipt.resolve_transcript_viewport(
            projection.transcript_document_identity.clone(),
            content_rows,
            layout[0].height,
            projection.transcript_revision,
        )
    } else {
        0
    };
    let transcript_has_unseen_output = receipt.transcript_has_unseen_output();
    let turn_steer_confirmation = projection.turn_steer_confirmation.take();
    let exit_confirmation_visible = projection.exit_confirmation_visible;

    let conversation_receipt = {
        let transcript_viewport_state = receipt.transcript_viewport_state();
        draw_fullscreen_conversation_shell(
            frame,
            projection,
            &layout,
            transcript_scroll_offset,
            transcript_has_unseen_output,
            transcript_viewport_state,
        )
    };
    receipt.record_queue_receipt_undo_hit_area(conversation_receipt.queue_receipt_undo_hit_area);
    receipt.record_transcript_frame(
        conversation_receipt.transcript_viewport_card_digests,
        conversation_receipt.transcript_viewport_card_hit_areas,
        conversation_receipt.transcript_viewport_frame_snapshot,
    );
    if let Some(list_state) = draw_fullscreen_shell_inspection(frame, layout[0], inspection) {
        receipt.record_session_list_state(list_state);
    }
    if let Some(confirmation) = turn_steer_confirmation.as_ref() {
        draw_turn_steer_confirmation(frame, confirmation);
    }
    // exit confirmation은 모든 shell/overlay state 위의 modal이므로 마지막 draw operation이어야 한다.
    if exit_confirmation_visible {
        draw_exit_confirmation(frame);
    }
    receipt
}

pub(super) fn fullscreen_frame_inspection_area(
    projection: &FullscreenConversationFrameProjection,
    area: Rect,
) -> Rect {
    build_fullscreen_flow_layout(projection, area, &projection.tail_view.lines)[0]
}

fn draw_turn_steer_confirmation(
    frame: &mut Frame<'_>,
    confirmation: &TurnSteerConfirmationScreenModel,
) {
    let request = &confirmation.request;
    let title = AkraTheme::title_line(
        confirmation.language.turn_steer_confirmation_title(),
        " / exact turn",
    );
    let (prompt_preview, prompt_truncated) = steer_prompt_preview(&request.prompt, 240, 6);
    let mut lines = vec![
        Line::from(confirmation.language.turn_steer_confirmation_question()),
        Line::from(format!(
            "thread: {}  |  turn: {}",
            compact_steer_identity(&request.thread_id),
            compact_steer_identity(&request.expected_turn_id),
        )),
        Line::from(""),
        Line::from("prompt:"),
    ];
    lines.extend(prompt_preview.into_iter().map(Line::from));
    if prompt_truncated {
        lines.push(Line::from(
            confirmation.language.turn_steer_preview_truncated(),
        ));
    }
    lines.extend([
        Line::from(""),
        AkraTheme::key_line(confirmation.language.turn_steer_confirmation_keys()),
    ]);
    let desired_width = lines
        .iter()
        .map(Line::width)
        .chain(std::iter::once(title.width()))
        .max()
        .unwrap_or(1)
        .saturating_add(2)
        .min(72)
        .min(usize::from(u16::MAX)) as u16;
    let popup_width = desired_width.min(frame.area().width);
    let rendered_body_rows = Paragraph::new(lines.clone())
        .wrap(Wrap { trim: false })
        .line_count(popup_width.saturating_sub(2));
    let content_height = rendered_body_rows
        .saturating_add(2)
        .min(usize::from(u16::MAX)) as u16;
    let popup_area = centered_fixed_rect(popup_width, content_height, frame.area());
    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(AkraTheme::panel_block(title))
            .wrap(Wrap { trim: false }),
        popup_area,
    );
}

fn compact_steer_identity(value: &str) -> String {
    let mut compact = value.chars().take(12).collect::<String>();
    if value.chars().count() > 12 {
        compact.push_str("...");
    }
    compact
}

fn steer_prompt_preview(value: &str, max_chars: usize, max_lines: usize) -> (Vec<String>, bool) {
    let mut preview = Vec::new();
    let mut remaining_chars = max_chars;
    let mut source_lines = value.split('\n').peekable();
    let mut truncated = false;

    while let Some(line) = source_lines.next() {
        if preview.len() >= max_lines {
            truncated = true;
            break;
        }
        let line_chars = line.chars().count();
        if line_chars > remaining_chars {
            preview.push(line.chars().take(remaining_chars).collect());
            truncated = true;
            break;
        }
        preview.push(line.to_string());
        remaining_chars -= line_chars;
        if source_lines.peek().is_some() {
            if remaining_chars == 0 {
                truncated = true;
                break;
            }
            remaining_chars -= 1;
        }
    }

    (preview, truncated)
}

fn draw_exit_confirmation(frame: &mut Frame<'_>) {
    let title = AkraTheme::title_line("Confirm Exit", "");
    let lines = vec![
        Line::from("You are already at the shell home."),
        Line::from("Exit codex-exec-loop?"),
        Line::from(""),
        AkraTheme::key_line("y: exit    n: stay"),
    ];
    let desired_width = lines
        .iter()
        .map(Line::width)
        .chain(std::iter::once(title.width()))
        .max()
        .unwrap_or(1)
        .saturating_add(2)
        .min(usize::from(u16::MAX)) as u16;
    let popup_width = desired_width.min(frame.area().width);
    let rendered_body_rows = Paragraph::new(lines.clone())
        .wrap(Wrap { trim: true })
        .line_count(popup_width.saturating_sub(2));
    let content_height = rendered_body_rows
        .saturating_add(2)
        .min(usize::from(u16::MAX)) as u16;
    let popup_area = centered_fixed_rect(popup_width, content_height, frame.area());
    frame.render_widget(Clear, popup_area);
    let popup = Paragraph::new(lines)
        .block(AkraTheme::panel_block(title))
        .wrap(Wrap { trim: true });

    frame.render_widget(popup, popup_area);
}

struct FullscreenConversationShellRenderReceipt {
    queue_receipt_undo_hit_area: Option<Rect>,
    transcript_viewport_card_digests: SharedTranscriptCardDigests,
    transcript_viewport_card_hit_areas: Vec<TranscriptCardHitArea>,
    transcript_viewport_frame_snapshot: Option<TranscriptViewportFrame>,
}

fn draw_fullscreen_conversation_shell(
    frame: &mut Frame<'_>,
    projection: FullscreenConversationFrameProjection,
    layout: &Rc<[Rect]>,
    transcript_scroll_offset: usize,
    transcript_has_unseen_output: bool,
    transcript_viewport_state: &TranscriptViewportUiState,
) -> FullscreenConversationShellRenderReceipt {
    let FullscreenConversationFrameProjection {
        tail_view,
        transcript_document,
        shell_overlay,
        ..
    } = projection;
    // 더 좁은 overlay나 더 짧은 tail이 terminal buffer에 stale cell을 남기지 않도록 항상 전체 frame을 먼저 지운다.
    let frame_area = frame.area();
    frame.render_widget(Clear, frame_area);
    // hidden-overlay path는 일반 conversation shell이다.
    // inspection layout을 우회해 transcript가 tail 위의 전체 공간을 채우게 한다.
    if shell_overlay == ShellOverlay::Hidden {
        // Welcome content belongs to the transcript band. Every conversation state shares the
        // same bottom-anchored tail, so the composer never jumps after the first submission.
        // standard shell에서는 tail 높이를 먼저 재고 live transcript line을 그 위 공간에 clip한다.
        let tail_band = layout.get(1).copied().unwrap_or(frame_area);
        let tail_area = fullscreen_tail_render_area(tail_band, &tail_view);
        let transcript_viewport_card_digests = transcript_document.card_digests();
        let transcript_receipt = render_fullscreen_transcript(
            frame,
            FullscreenTranscriptRenderRequest {
                area: layout[0],
                document: transcript_document,
                scroll_offset: transcript_scroll_offset,
                has_unseen_output: transcript_has_unseen_output,
                viewport_state: transcript_viewport_state,
            },
        );
        return FullscreenConversationShellRenderReceipt {
            queue_receipt_undo_hit_area: render_bottom_anchored_tail(frame, tail_area, tail_view),
            transcript_viewport_card_digests,
            transcript_viewport_card_hit_areas: transcript_receipt.card_hit_areas,
            transcript_viewport_frame_snapshot: transcript_receipt.frame_snapshot,
        };
    }
    // overlay/modal이 active이면 layout[0]은 inspection이 쓰고 layout[1]은 그 아래에 tail을 고정한다.
    // exit modal은 두 영역을 모두 덮어야 하므로 이 함수 밖에서 계속 그린다.
    let tail_area = fullscreen_tail_render_area(layout[1], &tail_view);
    FullscreenConversationShellRenderReceipt {
        queue_receipt_undo_hit_area: render_tail_surface(
            frame,
            tail_area,
            tail_view,
            shell_overlay == ShellOverlay::Supersession,
        ),
        transcript_viewport_card_digests: SharedTranscriptCardDigests::from(Vec::new()),
        transcript_viewport_card_hit_areas: Vec::new(),
        transcript_viewport_frame_snapshot: None,
    }
}

fn render_bottom_anchored_tail(
    frame: &mut Frame<'_>,
    tail_area: Rect,
    tail_view: super::shell_presentation::ShellTailView,
) -> Option<Rect> {
    render_tail_surface(frame, tail_area, tail_view, true)
}

fn render_tail_surface(
    frame: &mut Frame<'_>,
    tail_area: Rect,
    tail_view: super::shell_presentation::ShellTailView,
    prompt_can_focus: bool,
) -> Option<Rect> {
    let Some(surface) = tail_view.composer_surface.clone() else {
        return render_flat_tail(frame, tail_area, tail_view, prompt_can_focus);
    };
    if !prompt_can_focus || !surface.focused {
        return render_flat_tail(frame, tail_area, tail_view, false);
    }
    if tail_area.width < 4 {
        return render_flat_tail(frame, tail_area, tail_view, prompt_can_focus);
    }
    if tail_area.height < 3 {
        let mut compact_lines = surface.body_lines;
        compact_lines.push(surface.action_line);
        let focus_row = surface.cursor_offset.map(|(_, y)| y);
        let dropped_rows = render_body_suffix(frame, tail_area, compact_lines, focus_row);
        let cursor_offset = surface
            .cursor_offset
            .and_then(|(x, y)| y.checked_sub(dropped_rows).map(|y| (x, y)));
        set_cursor_if_visible(frame, tail_area, cursor_offset);
        return None;
    }

    let body_width = super::shell_presentation::composer_inner_width(tail_area.width);
    let desired_body_height = Paragraph::new(surface.body_lines.clone())
        .wrap(Wrap { trim: false })
        .line_count(body_width)
        .max(1)
        .min(usize::from(u16::MAX)) as u16;
    let composer_height = desired_body_height
        .saturating_add(2)
        .min(tail_area.height)
        .max(3);
    let prefix_height = tail_area.height.saturating_sub(composer_height);
    let prefix_area = Rect::new(tail_area.x, tail_area.y, tail_area.width, prefix_height);
    let composer_area = Rect::new(
        tail_area.x,
        tail_area.y.saturating_add(prefix_height),
        tail_area.width,
        composer_height,
    );

    let dropped_prefix_rows = if prefix_height == 0 {
        count_wrapped_rows(tail_view.prefix_lines(), tail_area.width).min(usize::from(u16::MAX))
            as u16
    } else {
        frame.render_widget(AkraTheme::status_block(), prefix_area);
        render_body_suffix(frame, prefix_area, tail_view.prefix_lines().to_vec(), None)
    };

    let mut footer_spans = vec![Span::raw(" ")];
    footer_spans.extend(surface.action_line.spans);
    footer_spans.push(Span::raw(" "));
    let composer_block = AkraTheme::composer_block(
        surface.focused && prompt_can_focus,
        Line::from(footer_spans),
    );
    let body_area = composer_block.inner(composer_area);
    frame.render_widget(composer_block, composer_area);
    let focus_row = surface.cursor_offset.map(|(_, y)| y);
    let dropped_body_rows = render_body_suffix(frame, body_area, surface.body_lines, focus_row);

    if prompt_can_focus {
        let cursor_offset = surface
            .cursor_offset
            .and_then(|(x, y)| y.checked_sub(dropped_body_rows).map(|y| (x, y)));
        set_cursor_if_visible(frame, body_area, cursor_offset);
    }

    let hit_area = tail_view
        .queue_receipt_undo_hit_area
        .and_then(|area| scroll_relative_rect(area, dropped_prefix_rows));
    resolve_queue_receipt_undo_hit_area(tail_area, hit_area)
}

fn render_flat_tail(
    frame: &mut Frame<'_>,
    tail_area: Rect,
    tail_view: super::shell_presentation::ShellTailView,
    prompt_can_focus: bool,
) -> Option<Rect> {
    let focus_row = tail_view.prompt_cursor_offset.map(|(_, y)| y);
    let dropped_rows = render_body_suffix(frame, tail_area, tail_view.lines, focus_row);
    let hit_area = tail_view
        .queue_receipt_undo_hit_area
        .and_then(|area| scroll_relative_rect(area, dropped_rows));
    let prompt_cursor_offset = tail_view
        .prompt_cursor_offset
        .and_then(|(x, y)| y.checked_sub(dropped_rows).map(|y| (x, y)));

    let hit_area = resolve_queue_receipt_undo_hit_area(tail_area, hit_area);
    if prompt_can_focus {
        set_cursor_if_visible(frame, tail_area, prompt_cursor_offset);
    }
    hit_area
}

fn scroll_relative_rect(area: Rect, dropped_rows: u16) -> Option<Rect> {
    Some(Rect::new(
        area.x,
        area.y.checked_sub(dropped_rows)?,
        area.width,
        area.height,
    ))
}

fn resolve_queue_receipt_undo_hit_area(
    tail_area: Rect,
    relative_hit_area: Option<Rect>,
) -> Option<Rect> {
    let hit_area = relative_hit_area.filter(|relative| {
        relative.right() <= tail_area.width && relative.bottom() <= tail_area.height
    });
    hit_area.map(|relative| {
        Rect::new(
            tail_area.x.saturating_add(relative.x),
            tail_area.y.saturating_add(relative.y),
            relative.width,
            relative.height,
        )
    })
}

struct RenderedTranscriptReceipt {
    card_hit_areas: Vec<TranscriptCardHitArea>,
    frame_snapshot: Option<TranscriptViewportFrame>,
}

struct FullscreenTranscriptRenderRequest<'a> {
    area: Rect,
    document: Rc<FullscreenTranscriptDocument>,
    scroll_offset: usize,
    has_unseen_output: bool,
    viewport_state: &'a TranscriptViewportUiState,
}

fn render_fullscreen_transcript(
    frame: &mut Frame<'_>,
    request: FullscreenTranscriptRenderRequest<'_>,
) -> RenderedTranscriptReceipt {
    let FullscreenTranscriptRenderRequest {
        area: transcript_area,
        document,
        scroll_offset,
        has_unseen_output,
        viewport_state: transcript_viewport_state,
    } = request;
    if document.is_empty() || transcript_area.width == 0 || transcript_area.height == 0 {
        return RenderedTranscriptReceipt {
            card_hit_areas: Vec::new(),
            frame_snapshot: None,
        };
    }
    let (paragraph_lines, window_scroll_offset) =
        document.paragraph_window(scroll_offset, transcript_area.height);
    let paragraph = Paragraph::new(paragraph_lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph.scroll((window_scroll_offset, 0)), transcript_area);
    render_transcript_row_surfaces(
        frame,
        transcript_area,
        scroll_offset,
        document.wrapped_rows(),
    );

    let unseen_badge_area = if has_unseen_output {
        let label = " ↓ new output · Ctrl+End ";
        let label_width = label.chars().count().min(usize::from(u16::MAX)) as u16;
        let width = label_width.min(transcript_area.width);
        let area = Rect::new(
            transcript_area.right().saturating_sub(width),
            transcript_area.bottom().saturating_sub(1),
            width,
            1,
        );
        frame.render_widget(
            Paragraph::new(Line::styled(label, AkraTheme::accent())),
            area,
        );
        Some(area)
    } else {
        None
    };

    // The snapshot is the selection authority for this committed frame. Capture
    // it only after every transcript-area overlay is drawn so pointer selection,
    // highlight, and clipboard text all describe the same visible cells.
    let frame_snapshot = capture_transcript_frame_snapshot(
        frame,
        transcript_area,
        scroll_offset,
        document.wrapped_rows(),
        transcript_viewport_state,
        unseen_badge_area.as_slice(),
    );

    let visible_end = scroll_offset.saturating_add(usize::from(transcript_area.height));
    let card_rows = document.card_rows();
    let first_visible_card = card_rows.partition_point(|row| row.end <= scroll_offset);
    let card_hit_areas = card_rows[first_visible_card..]
        .iter()
        .take_while(|row| row.start < visible_end)
        .filter_map(|row| {
            let clipped_start = row.start.max(scroll_offset);
            let clipped_end = row.end.min(visible_end);
            (clipped_end > clipped_start).then(|| TranscriptCardHitArea {
                digest: row.digest,
                area: Rect::new(
                    transcript_area.x,
                    transcript_area.y.saturating_add(
                        u16::try_from(clipped_start.saturating_sub(scroll_offset))
                            .unwrap_or(u16::MAX),
                    ),
                    transcript_area.width,
                    u16::try_from(clipped_end.saturating_sub(clipped_start)).unwrap_or(u16::MAX),
                ),
            })
        })
        .collect();
    RenderedTranscriptReceipt {
        card_hit_areas,
        frame_snapshot: Some(frame_snapshot),
    }
}

fn render_transcript_row_surfaces(
    frame: &mut Frame<'_>,
    area: Rect,
    scroll_offset: usize,
    wrapped_rows: &[TranscriptWrappedRowLayout],
) {
    for visible_row in 0..area.height {
        let absolute_row = scroll_offset.saturating_add(usize::from(visible_row));
        let Some(row) = wrapped_rows.get(absolute_row) else {
            continue;
        };
        if row.surface != ConversationTranscriptLineSurface::UserPrompt {
            continue;
        }
        for column in 0..area.width {
            if let Some(cell) = frame.buffer_mut().cell_mut(Position::new(
                area.x.saturating_add(column),
                area.y.saturating_add(visible_row),
            )) {
                cell.set_style(AkraTheme::user_prompt_surface());
            }
        }
    }
}

fn capture_transcript_frame_snapshot(
    frame: &mut Frame<'_>,
    area: Rect,
    scroll_offset: usize,
    wrapped_rows: &[TranscriptWrappedRowLayout],
    transcript_viewport_state: &TranscriptViewportUiState,
    selection_exclusions: &[Rect],
) -> TranscriptViewportFrame {
    let mut rows = Vec::new();
    for visible_row in 0..area.height {
        let absolute_row = scroll_offset.saturating_add(usize::from(visible_row));
        let row_layout = wrapped_rows.get(absolute_row);
        let screen_row = area.y.saturating_add(visible_row);
        let row_selection_exclusions =
            selection_excluded_columns_for_row(area, screen_row, selection_exclusions);
        if let Some((start_column, end_column)) =
            transcript_viewport_state.selection_columns_for_row(absolute_row, area.width)
        {
            for column in start_column..=end_column {
                if selection_column_is_excluded(column, &row_selection_exclusions) {
                    continue;
                }
                if let Some(cell) = frame.buffer_mut().cell_mut(Position::new(
                    area.x.saturating_add(column),
                    area.y.saturating_add(visible_row),
                )) {
                    cell.set_style(AkraTheme::transcript_selection());
                }
            }
        }
        let cells = (0..area.width)
            .map(|column| {
                frame
                    .buffer_mut()
                    .cell(Position::new(
                        area.x.saturating_add(column),
                        area.y.saturating_add(visible_row),
                    ))
                    .map_or_else(String::new, |cell| cell.symbol().to_string())
            })
            .collect();
        rows.push(TranscriptRenderedRow {
            absolute_row,
            logical_line_index: row_layout.map_or_else(
                || usize::MAX.saturating_sub(usize::from(visible_row)),
                |layout| layout.logical_line_index,
            ),
            soft_wrap_separator: row_layout
                .map(|layout| layout.soft_wrap_separator.clone())
                .unwrap_or_default(),
            selection_range_id: row_layout.and_then(|layout| layout.selection_range_id),
            selectable_from_column: row_layout
                .map(|layout| layout.selectable_from_column)
                .unwrap_or_default(),
            selection_excluded_columns: row_selection_exclusions,
            cells,
        });
    }
    TranscriptViewportFrame { area, rows }
}

fn selection_excluded_columns_for_row(
    area: Rect,
    screen_row: u16,
    exclusions: &[Rect],
) -> Vec<(u16, u16)> {
    exclusions
        .iter()
        .filter(|exclusion| {
            screen_row >= exclusion.top()
                && screen_row < exclusion.bottom()
                && exclusion.right() > area.left()
                && exclusion.left() < area.right()
        })
        .filter_map(|exclusion| {
            let start = exclusion.left().max(area.left()).saturating_sub(area.x);
            let end_exclusive = exclusion.right().min(area.right()).saturating_sub(area.x);
            (end_exclusive > start).then_some((start, end_exclusive.saturating_sub(1)))
        })
        .collect()
}

fn selection_column_is_excluded(column: u16, exclusions: &[(u16, u16)]) -> bool {
    exclusions
        .iter()
        .any(|(start, end)| column >= *start && column <= *end)
}

#[cfg(test)]
#[path = "fullscreen_rendering_tests.rs"]
mod tests;
