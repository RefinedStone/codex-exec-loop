use std::rc::Rc;

use super::inline_frame_model::ApprovalInlineScreenModel;
pub(super) use super::inline_frame_model::{
    InlineConversationFrameProjection, InlineFrameRenderReceipt, InlineInspectionFrameModel,
    InlineShellFrameModel, apply_inline_frame_render_receipt, capture_inline_shell_frame_model,
};
use super::shell_presentation::TurnSteerConfirmationScreenModel;
#[cfg(test)]
use super::*;
use super::{AkraTheme, ShellFrontendMode, ShellOverlay};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph, Wrap};

/*
 * 이 파일은 native inline shell의 ratatui frame boundary다.
 * presentation layer가 Line 기반 read model을 만들고, inline_layout이 frame 분할을 정하면,
 * 이 module은 base inline conversation, 선택적 inline inspection, exit confirmation modal 순서로 layer를 적용한다.
 */
#[path = "shell_rendering/inline_inspection.rs"]
mod inline_inspection;
#[path = "shell_rendering/inline_layout.rs"]
mod inline_layout;

use inline_inspection::draw_inline_shell_inspection;
#[cfg(test)]
use inline_layout::centered_rect;
use inline_layout::{
    build_inline_terminal_flow_layout, centered_fixed_rect, inline_body_render_area,
    render_inline_body, render_inline_body_suffix, set_cursor_if_visible,
};
pub(in crate::adapter::inbound::tui::app) use inline_layout::{
    count_rendered_inline_rows, inline_section_height,
};

#[cfg(test)]
pub(super) fn prepare_render_state(app: &mut NativeTuiApp, mode: ShellFrontendMode, area: Rect) {
    let projection = InlineConversationFrameProjection::from_app(app, area.width);
    let model = capture_inline_shell_frame_model(app, mode, area, projection);
    let (_, _, receipt) = model.into_parts();
    assert!(apply_inline_frame_render_receipt(app, receipt));
}

pub(super) fn inline_parallel_event_stream_visible_rows(
    projection: &InlineConversationFrameProjection,
    frame_area: Rect,
) -> usize {
    if !projection.parallel_mode_enabled && projection.shell_overlay != ShellOverlay::Supersession {
        return 0;
    }

    let layout =
        build_inline_terminal_flow_layout(projection, frame_area, &projection.tail_view.lines);
    projection
        .supersession_overlay_view
        .as_deref()
        .map_or(0, |view| {
            inline_inspection::parallel_event_stream_visible_rows(view, layout[0])
        })
}

#[cfg(test)]
pub(super) fn draw(frame: &mut Frame<'_>, app: &mut NativeTuiApp, mode: ShellFrontendMode) {
    let projection = InlineConversationFrameProjection::from_app(app, frame.area().width);
    let model = capture_inline_shell_frame_model(app, mode, frame.area(), projection);
    let receipt = draw_projected(frame, mode, model);
    assert!(apply_inline_frame_render_receipt(app, receipt));
}

pub(super) fn draw_projected(
    frame: &mut Frame<'_>,
    mode: ShellFrontendMode,
    model: InlineShellFrameModel,
) -> InlineFrameRenderReceipt {
    let (mut projection, inspection, mut receipt) = model.into_parts();
    // 현재 native shell renderer는 하나뿐이지만, mode 인자를 유지해 app runtime과 shell frontend 추상화를 한 경계에서 묶는다.
    let _ = mode;
    let frame_area = frame.area();
    // tail view는 status/prompt line과 cursor offset을 함께 담는다.
    // 같은 tail 높이가 inline inspection/body 분할 기준도 된다.
    let layout =
        build_inline_terminal_flow_layout(&projection, frame_area, &projection.tail_view.lines);
    let turn_steer_confirmation = projection.turn_steer_confirmation.take();
    let exit_confirmation_visible = projection.exit_confirmation_visible;

    let queue_receipt_undo_hit_area = draw_inline_conversation_shell(frame, projection, &layout);
    receipt.record_queue_receipt_undo_hit_area(queue_receipt_undo_hit_area);
    if let Some(list_state) = draw_inline_shell_inspection(frame, layout[0], inspection) {
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

pub(super) fn inline_frame_inspection_area(
    projection: &InlineConversationFrameProjection,
    area: Rect,
) -> Rect {
    build_inline_terminal_flow_layout(projection, area, &projection.tail_view.lines)[0]
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

fn draw_inline_conversation_shell(
    frame: &mut Frame<'_>,
    projection: InlineConversationFrameProjection,
    layout: &Rc<[Rect]>,
) -> Option<Rect> {
    let InlineConversationFrameProjection {
        tail_view,
        live_transcript_lines,
        shell_overlay,
        parallel_mode_enabled,
        renders_parallel_viewport_handoff,
        ..
    } = projection;
    // 더 좁은 overlay나 더 짧은 tail이 terminal buffer에 stale cell을 남기지 않도록 항상 전체 frame을 먼저 지운다.
    let frame_area = frame.area();
    frame.render_widget(Clear, frame_area);
    // hidden-overlay path는 일반 conversation shell이다.
    // inspection layout을 우회해 transcript가 tail 위의 전체 공간을 채우게 한다.
    if shell_overlay == ShellOverlay::Hidden {
        if parallel_mode_enabled && !renders_parallel_viewport_handoff {
            let tail_band = layout.get(1).copied().unwrap_or(frame_area);
            let tail_area = inline_body_render_area(tail_band, &tail_view.lines);
            return render_bottom_anchored_tail(frame, tail_area, tail_view);
        }
        // startup banner 같은 presentation state는 의도적으로 상단부터 전체 frame을 소유하므로 bottom anchored가 아니어야 한다.
        if tail_view.render_from_top {
            let hit_area = resolve_queue_receipt_undo_hit_area(
                frame_area,
                tail_view.queue_receipt_undo_hit_area,
            );
            render_inline_body(frame, frame_area, tail_view.lines, false);
            set_cursor_if_visible(frame, frame_area, tail_view.prompt_cursor_offset);
            return hit_area;
        }
        // standard shell에서는 tail 높이를 먼저 재고 live transcript line을 그 위 공간에 clip한다.
        let tail_band = layout.get(1).copied().unwrap_or(frame_area);
        let tail_area = inline_body_render_area(tail_band, &tail_view.lines);
        render_inline_live_transcript(frame, frame_area, tail_area, live_transcript_lines);
        return render_bottom_anchored_tail(frame, tail_area, tail_view);
    }
    // overlay/modal이 active이면 layout[0]은 inspection이 쓰고 layout[1]은 그 아래에 tail을 고정한다.
    // exit modal은 두 영역을 모두 덮어야 하므로 이 함수 밖에서 계속 그린다.
    let tail_area = inline_body_render_area(layout[1], &tail_view.lines);
    let hit_area =
        resolve_queue_receipt_undo_hit_area(tail_area, tail_view.queue_receipt_undo_hit_area);
    render_inline_body(frame, tail_area, tail_view.lines, false);
    if shell_overlay == ShellOverlay::Supersession {
        set_cursor_if_visible(frame, tail_area, tail_view.prompt_cursor_offset);
    }
    hit_area
}

fn render_bottom_anchored_tail(
    frame: &mut Frame<'_>,
    tail_area: Rect,
    tail_view: super::shell_presentation::InlineTailView,
) -> Option<Rect> {
    let focus_row = tail_view.prompt_cursor_offset.map(|(_, y)| y);
    let dropped_rows = render_inline_body_suffix(frame, tail_area, tail_view.lines, focus_row);
    let hit_area = tail_view
        .queue_receipt_undo_hit_area
        .and_then(|area| scroll_relative_rect(area, dropped_rows));
    let prompt_cursor_offset = tail_view
        .prompt_cursor_offset
        .and_then(|(x, y)| y.checked_sub(dropped_rows).map(|y| (x, y)));

    let hit_area = resolve_queue_receipt_undo_hit_area(tail_area, hit_area);
    set_cursor_if_visible(frame, tail_area, prompt_cursor_offset);
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

fn render_inline_live_transcript(
    frame: &mut Frame<'_>,
    frame_area: Rect,
    tail_area: Rect,
    live_transcript_lines: Vec<Line<'static>>,
) {
    // transcript line이 없거나 tail 위의 vertical space가 없으면 live region에 그릴 유효 내용이 없다.
    if live_transcript_lines.is_empty() || tail_area.y <= frame_area.y {
        return;
    }
    // live container는 frame 상단부터 prompt tail 직전 row까지다.
    // inner render area를 bottom-align해 최신 출력이 prompt에 가장 가깝게 앉게 한다.
    let live_container = Rect::new(
        frame_area.x,
        frame_area.y,
        frame_area.width,
        tail_area.y.saturating_sub(frame_area.y),
    );
    let live_area = inline_body_render_area(live_container, &live_transcript_lines);
    render_inline_body_suffix(frame, live_area, live_transcript_lines, None);
}

#[cfg(test)]
// contract test는 overlay layout, inline tail behavior, viewport replay를 고정한다.
#[path = "shell_rendering_contract_tests.rs"]
mod contract_tests;
#[cfg(test)]
// snapshot test는 runtime state별 대표 shell frame을 고정한다.
#[path = "shell_rendering_tests.rs"]
mod tests;
