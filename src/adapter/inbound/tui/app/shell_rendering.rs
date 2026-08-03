use std::collections::VecDeque;
use std::mem;
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
use super::shell_presentation::{
    ConversationTranscriptCardRow, ConversationTranscriptLineInteraction,
    ConversationTranscriptLineSurface, TurnSteerConfirmationScreenModel, startup_ascii_art_lines,
};
#[cfg(test)]
use super::*;
use super::{
    AkraTheme, ShellFrontendMode, ShellOverlay, TranscriptCardHitArea, TranscriptRenderedRow,
    TranscriptViewportFrame, TranscriptViewportUiState,
};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/*
 * 이 파일은 native fullscreen shell의 ratatui frame boundary다.
 * presentation layer가 Line 기반 read model을 만들고, fullscreen_layout이 frame 분할을 정하면,
 * 이 module은 app-owned conversation viewport, 선택적 fullscreen inspection, exit confirmation modal 순서로 layer를 적용한다.
 */
#[path = "shell_rendering/fullscreen_inspection.rs"]
mod fullscreen_inspection;
#[path = "shell_rendering/fullscreen_layout.rs"]
mod fullscreen_layout;

use fullscreen_inspection::draw_fullscreen_shell_inspection;
use fullscreen_layout::{
    build_fullscreen_flow_layout, centered_fixed_rect, fullscreen_tail_render_area,
    render_body_suffix, set_cursor_if_visible,
};
pub(in crate::adapter::inbound::tui::app) use fullscreen_layout::{
    count_wrapped_rows, fullscreen_section_height,
};

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
        let content_rows = count_wrapped_rows(&projection.transcript_lines, layout[0].width);
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
    transcript_viewport_card_digests: Vec<[u8; 32]>,
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
        transcript_lines,
        transcript_line_interactions,
        transcript_card_rows,
        shell_overlay,
        ..
    } = projection;
    // 더 좁은 overlay나 더 짧은 tail이 terminal buffer에 stale cell을 남기지 않도록 항상 전체 frame을 먼저 지운다.
    let frame_area = frame.area();
    frame.render_widget(Clear, frame_area);
    // hidden-overlay path는 일반 conversation shell이다.
    // inspection layout을 우회해 transcript가 tail 위의 전체 공간을 채우게 한다.
    if shell_overlay == ShellOverlay::Hidden {
        // startup banner 같은 presentation state는 의도적으로 상단부터 전체 frame을 소유하므로 bottom anchored가 아니어야 한다.
        if tail_view.render_from_top {
            if !transcript_lines.is_empty() {
                // The startup logo is modeled as transcript content so the first frame stays
                // immutable and renderer-owned. Keep it directly above the top-anchored tail;
                // returning after only the tail would silently discard the logo.
                let tail_height = tail_view.rendered_height(frame_area.width, frame_area.height);
                let available_logo_height = frame_area.height.saturating_sub(tail_height);
                let logo_lines = startup_ascii_art_lines(Some(available_logo_height));
                let logo_height = count_wrapped_rows(&logo_lines, frame_area.width)
                    .min(usize::from(available_logo_height))
                    as u16;
                let logo_area =
                    Rect::new(frame_area.x, frame_area.y, frame_area.width, logo_height);
                let tail_area = Rect::new(
                    frame_area.x,
                    logo_area.bottom(),
                    frame_area.width,
                    tail_height,
                );
                let transcript_viewport_card_digests =
                    transcript_card_rows.iter().map(|row| row.digest).collect();
                let transcript_receipt = render_fullscreen_transcript(
                    frame,
                    FullscreenTranscriptRenderRequest {
                        area: logo_area,
                        lines: logo_lines,
                        line_interactions: Vec::new(),
                        card_rows: Vec::new(),
                        scroll_offset: 0,
                        has_unseen_output: false,
                        viewport_state: transcript_viewport_state,
                    },
                );
                return FullscreenConversationShellRenderReceipt {
                    queue_receipt_undo_hit_area: render_bottom_anchored_tail(
                        frame, tail_area, tail_view,
                    ),
                    transcript_viewport_card_digests,
                    transcript_viewport_card_hit_areas: transcript_receipt.card_hit_areas,
                    transcript_viewport_frame_snapshot: transcript_receipt.frame_snapshot,
                };
            }
            let top_area = Rect::new(
                frame_area.x,
                frame_area.y,
                frame_area.width,
                tail_view.rendered_height(frame_area.width, frame_area.height),
            );
            return FullscreenConversationShellRenderReceipt {
                queue_receipt_undo_hit_area: render_bottom_anchored_tail(
                    frame, top_area, tail_view,
                ),
                transcript_viewport_card_digests: Vec::new(),
                transcript_viewport_card_hit_areas: Vec::new(),
                transcript_viewport_frame_snapshot: None,
            };
        }
        // standard shell에서는 tail 높이를 먼저 재고 live transcript line을 그 위 공간에 clip한다.
        let tail_band = layout.get(1).copied().unwrap_or(frame_area);
        let tail_area = fullscreen_tail_render_area(tail_band, &tail_view);
        let transcript_viewport_card_digests =
            transcript_card_rows.iter().map(|row| row.digest).collect();
        let transcript_receipt = render_fullscreen_transcript(
            frame,
            FullscreenTranscriptRenderRequest {
                area: layout[0],
                lines: transcript_lines,
                line_interactions: transcript_line_interactions,
                card_rows: transcript_card_rows,
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
        queue_receipt_undo_hit_area: render_tail_surface(frame, tail_area, tail_view, false),
        transcript_viewport_card_digests: Vec::new(),
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
    lines: Vec<Line<'static>>,
    line_interactions: Vec<ConversationTranscriptLineInteraction>,
    card_rows: Vec<ConversationTranscriptCardRow>,
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
        lines: transcript_lines,
        line_interactions: transcript_line_interactions,
        card_rows,
        scroll_offset,
        has_unseen_output,
        viewport_state: transcript_viewport_state,
    } = request;
    if transcript_lines.is_empty() || transcript_area.width == 0 || transcript_area.height == 0 {
        return RenderedTranscriptReceipt {
            card_hit_areas: Vec::new(),
            frame_snapshot: None,
        };
    }
    let projected_rows = card_rows
        .into_iter()
        .filter_map(|row| {
            if row.line_index >= transcript_lines.len() {
                return None;
            }
            let start =
                count_wrapped_rows(&transcript_lines[..row.line_index], transcript_area.width);
            let end =
                count_wrapped_rows(&transcript_lines[..=row.line_index], transcript_area.width);
            (end > start).then_some((row.digest, start, end))
        })
        .collect::<Vec<_>>();
    let wrapped_rows = transcript_wrapped_row_layout(
        &transcript_lines,
        &transcript_line_interactions,
        transcript_area.width,
    );
    let (window_start_line, window_scroll_offset) =
        transcript_window_for_scroll(&transcript_lines, transcript_area.width, scroll_offset);
    let paragraph = Paragraph::new(
        transcript_lines
            .into_iter()
            .skip(window_start_line)
            .collect::<Vec<_>>(),
    )
    .wrap(Wrap { trim: false });
    frame.render_widget(paragraph.scroll((window_scroll_offset, 0)), transcript_area);
    render_transcript_row_surfaces(frame, transcript_area, scroll_offset, &wrapped_rows);

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
        &wrapped_rows,
        transcript_viewport_state,
        unseen_badge_area.as_slice(),
    );

    let visible_end = scroll_offset.saturating_add(usize::from(transcript_area.height));
    let card_hit_areas = projected_rows
        .into_iter()
        .filter_map(|(digest, start, end)| {
            let clipped_start = start.max(scroll_offset);
            let clipped_end = end.min(visible_end);
            (clipped_end > clipped_start).then(|| TranscriptCardHitArea {
                digest,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct TranscriptWrappedRowLayout {
    logical_line_index: usize,
    soft_wrap_separator: String,
    selection_range_id: Option<u64>,
    selectable_from_column: u16,
    surface: ConversationTranscriptLineSurface,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TranscriptWrapGrapheme {
    symbol: String,
    source_index: usize,
}

impl TranscriptWrapGrapheme {
    fn width(&self) -> u16 {
        u16::try_from(self.symbol.as_str().width()).unwrap_or(u16::MAX)
    }

    fn is_whitespace(&self) -> bool {
        self.symbol == "\u{200b}"
            || (self.symbol.chars().all(char::is_whitespace) && self.symbol != "\u{00a0}")
    }
}

fn transcript_wrapped_row_layout(
    lines: &[Line<'_>],
    line_interactions: &[ConversationTranscriptLineInteraction],
    width: u16,
) -> Vec<TranscriptWrappedRowLayout> {
    let mut rows = Vec::new();
    for (logical_line_index, line) in lines.iter().enumerate() {
        let source = transcript_line_graphemes(line);
        let wrapped = wrap_transcript_graphemes(&source, width);
        let interaction = line_interactions
            .get(logical_line_index)
            .copied()
            .unwrap_or(ConversationTranscriptLineInteraction {
                selection_range_id: None,
                selectable_from_column: 0,
                surface: ConversationTranscriptLineSurface::Plain,
            });
        rows.extend(wrapped.iter().enumerate().map(|(row_index, row)| {
            let soft_wrap_separator = wrapped
                .get(row_index + 1)
                .map_or_else(String::new, |next_row| {
                    wrap_boundary_separator(&source, row, next_row)
                });
            TranscriptWrappedRowLayout {
                logical_line_index,
                soft_wrap_separator,
                selection_range_id: interaction.selection_range_id,
                selectable_from_column: if row_index == 0 {
                    interaction
                        .selectable_from_column
                        .min(width.saturating_sub(1))
                } else {
                    0
                },
                surface: interaction.surface,
            }
        }));
    }
    rows
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

fn transcript_line_graphemes(line: &Line<'_>) -> Vec<TranscriptWrapGrapheme> {
    line.spans
        .iter()
        .flat_map(|span| UnicodeSegmentation::graphemes(span.content.as_ref(), true))
        .enumerate()
        .map(|(source_index, symbol)| TranscriptWrapGrapheme {
            symbol: symbol.to_string(),
            source_index,
        })
        .collect()
}

/// Mirrors Ratatui's `WordWrapper` with `trim: false`, while retaining source
/// indices for whitespace that the renderer consumes at a wrap boundary.
fn wrap_transcript_graphemes(
    source: &[TranscriptWrapGrapheme],
    max_line_width: u16,
) -> Vec<Vec<TranscriptWrapGrapheme>> {
    if max_line_width == 0 {
        return Vec::new();
    }

    let mut wrapped_lines = Vec::new();
    let mut pending_line = Vec::new();
    let mut pending_word = Vec::new();
    let mut pending_whitespace: VecDeque<TranscriptWrapGrapheme> = VecDeque::new();
    let mut line_width = 0_u16;
    let mut word_width = 0_u16;
    let mut whitespace_width = 0_u16;
    let mut non_whitespace_previous = false;

    for grapheme in source.iter().cloned() {
        let is_whitespace = grapheme.is_whitespace();
        let symbol_width = grapheme.width();
        if symbol_width > max_line_width {
            continue;
        }

        let word_found = non_whitespace_previous && is_whitespace;
        let untrimmed_overflow = pending_line.is_empty()
            && word_width
                .saturating_add(whitespace_width)
                .saturating_add(symbol_width)
                > max_line_width;
        if word_found || untrimmed_overflow {
            pending_line.extend(pending_whitespace.drain(..));
            line_width = line_width.saturating_add(whitespace_width);
            pending_line.append(&mut pending_word);
            line_width = line_width.saturating_add(word_width);
            whitespace_width = 0;
            word_width = 0;
        }

        let line_full = line_width >= max_line_width;
        let pending_word_overflow = symbol_width > 0
            && line_width
                .saturating_add(whitespace_width)
                .saturating_add(word_width)
                >= max_line_width;
        if line_full || pending_word_overflow {
            let mut remaining_width = max_line_width.saturating_sub(line_width);
            wrapped_lines.push(mem::take(&mut pending_line));
            line_width = 0;

            while let Some(pending) = pending_whitespace.front() {
                let width = pending.width();
                if width > remaining_width {
                    break;
                }
                whitespace_width = whitespace_width.saturating_sub(width);
                remaining_width = remaining_width.saturating_sub(width);
                pending_whitespace.pop_front();
            }

            if is_whitespace && pending_whitespace.is_empty() {
                continue;
            }
        }

        if is_whitespace {
            whitespace_width = whitespace_width.saturating_add(symbol_width);
            pending_whitespace.push_back(grapheme);
        } else {
            word_width = word_width.saturating_add(symbol_width);
            pending_word.push(grapheme);
        }
        non_whitespace_previous = !is_whitespace;
    }

    pending_line.extend(pending_whitespace);
    pending_line.append(&mut pending_word);
    if !pending_line.is_empty() {
        wrapped_lines.push(pending_line);
    }
    if wrapped_lines.is_empty() {
        wrapped_lines.push(Vec::new());
    }
    wrapped_lines
}

fn wrap_boundary_separator(
    source: &[TranscriptWrapGrapheme],
    row: &[TranscriptWrapGrapheme],
    next_row: &[TranscriptWrapGrapheme],
) -> String {
    let Some(start) = row
        .last()
        .map(|grapheme| grapheme.source_index.saturating_add(1))
    else {
        return String::new();
    };
    let Some(end) = next_row.first().map(|grapheme| grapheme.source_index) else {
        return String::new();
    };
    source
        .get(start..end)
        .unwrap_or_default()
        .iter()
        .filter(|grapheme| grapheme.is_whitespace())
        .map(|grapheme| grapheme.symbol.as_str())
        .collect()
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

fn transcript_window_for_scroll(lines: &[Line<'_>], width: u16, top_row: usize) -> (usize, u16) {
    let mut preceding_rows = 0usize;
    for (line_index, line) in lines.iter().enumerate() {
        let line_rows = count_wrapped_rows(std::slice::from_ref(line), width).max(1);
        let following_rows = preceding_rows.saturating_add(line_rows);
        if top_row < following_rows {
            return (
                line_index,
                u16::try_from(top_row.saturating_sub(preceding_rows)).unwrap_or(u16::MAX),
            );
        }
        preceding_rows = following_rows;
    }
    (lines.len(), 0)
}

#[cfg(test)]
#[path = "fullscreen_rendering_tests.rs"]
mod tests;
