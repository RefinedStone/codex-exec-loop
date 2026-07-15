use std::rc::Rc;

use super::shell_presentation::{build_inline_live_transcript_lines, build_inline_tail_view};
use super::*;
use ratatui::widgets::{Paragraph, Wrap};

/*
 * 이 파일은 native inline shell의 ratatui frame boundary다.
 * presentation layer가 Line 기반 read model을 만들고, inline_layout이 frame 분할을 정하면,
 * 이 module은 base inline conversation, 선택적 inline inspection, exit confirmation modal 순서로 layer를 적용한다.
 */
#[path = "shell_rendering/inline_inspection.rs"]
mod inline_inspection;
#[path = "shell_rendering/inline_layout.rs"]
mod inline_layout;

#[cfg(test)]
use super::shell_presentation::build_planning_draft_editor_overlay_view;
use inline_inspection::{draw_inline_parallel_mode_inspection, draw_inline_shell_inspection};
#[cfg(test)]
use inline_layout::centered_rect;
use inline_layout::{
    build_inline_terminal_flow_layout, centered_fixed_rect, inline_body_render_area,
    render_inline_body, render_inline_body_suffix, set_cursor_if_visible,
};

pub(super) fn prepare_render_state(app: &mut NativeTuiApp, mode: ShellFrontendMode, area: Rect) {
    // prepare signature를 draw와 맞춰 두면, 나중에 frontend mode별 pre-render state가 필요해도 entrypoint를 늘리지 않는다.
    let _ = mode;
    // manual editor overlay만 render area를 알아야 scroll을 맞출 수 있다.
    // 다른 overlay는 textarea cursor를 소유하지 않으므로 일반 frame에서 editor state를 바꾸면 안 된다.
    let directions_editor_open = app.shell_overlay == ShellOverlay::DirectionsMaintenance
        && app.directions_maintenance_overlay_ui_state.step()
            == DirectionsMaintenanceOverlayStep::ManualEditor;
    let planning_editor_open = app.shell_overlay == ShellOverlay::PlanningInit
        && app.planning_init_overlay_ui_state.step() == PlanningInitOverlayStep::ManualEditor;
    if !directions_editor_open && !planning_editor_open {
        return;
    }
    // editor는 inspection area 안에 있고, 그 높이는 현재 tail에 따라 달라진다.
    // draw가 사용할 layout 입력을 그대로 다시 만들어 layout[0]에서 textarea viewport 높이를 산출한다.
    let tail_view = build_inline_tail_view(app, area.width);
    let inspection_area = build_inline_terminal_flow_layout(app, area, &tail_view.lines)[0];
    // editor chrome은 title, tabs, validation/status, borders로 고정 row를 소비한다.
    // 아주 작은 terminal에서도 cursor 계산이 정의되도록 작은 하한을 유지한다.
    let editor_content_height = inspection_area
        .height
        .saturating_sub(14)
        .max(6)
        .saturating_sub(1)
        .max(1);
    app.planning_draft_editor_ui_state
        .sync_editor_scroll(editor_content_height);
}

pub(super) fn inline_parallel_event_stream_visible_rows(
    app: &NativeTuiApp,
    frame_area: Rect,
) -> usize {
    if !app.parallel_mode_enabled() && app.shell_overlay != ShellOverlay::Supersession {
        return 0;
    }

    let tail_view = build_inline_tail_view(app, frame_area.width);
    let layout = build_inline_terminal_flow_layout(app, frame_area, &tail_view.lines);
    inline_inspection::parallel_event_stream_visible_rows(app, layout[0])
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &mut NativeTuiApp, mode: ShellFrontendMode) {
    // 현재 native shell renderer는 하나뿐이지만, mode 인자를 유지해 app runtime과 shell frontend 추상화를 한 경계에서 묶는다.
    let _ = mode;
    // Mouse coordinates belong to the last completed frame. Clear them before rebuilding this frame's geometry.
    app.clear_queue_receipt_undo_hit_area();
    let frame_area = frame.area();
    // tail view는 status/prompt line과 cursor offset을 함께 담는다.
    // 같은 tail 높이가 inline inspection/body 분할 기준도 된다.
    let tail_view = build_inline_tail_view(app, frame_area.width);
    let live_transcript_lines = build_inline_live_transcript_lines(app);
    let layout = build_inline_terminal_flow_layout(app, frame_area, &tail_view.lines);

    draw_inline_conversation_shell(frame, app, tail_view, live_transcript_lines, &layout);
    // inline inspection은 base shell 뒤에 그려 overlay가 고정된 prompt/status tail은 두고 상단 body만 대체하게 한다.
    if app.shell_overlay != ShellOverlay::Hidden {
        draw_inline_shell_inspection(frame, app, layout[0]);
    } else if app.parallel_mode_enabled() && !renders_parallel_viewport_handoff(app) {
        draw_inline_parallel_mode_inspection(frame, layout[0], app);
    }
    if app.is_turn_steer_confirmation_visible() {
        draw_turn_steer_confirmation(frame, app);
    }
    // exit confirmation은 모든 shell/overlay state 위의 modal이므로 마지막 draw operation이어야 한다.
    if app.is_exit_confirmation_visible() {
        draw_exit_confirmation(frame);
    }
}

fn draw_turn_steer_confirmation(frame: &mut Frame<'_>, app: &NativeTuiApp) {
    let Some(intent) = app.turn_steer_confirmation.as_ref() else {
        return;
    };
    let request = &intent.request;
    let title = AkraTheme::title_line(
        app.tui_language.turn_steer_confirmation_title(),
        " / exact turn",
    );
    let (prompt_preview, prompt_truncated) = steer_prompt_preview(&request.prompt, 240, 6);
    let mut lines = vec![
        Line::from(app.tui_language.turn_steer_confirmation_question()),
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
        lines.push(Line::from(app.tui_language.turn_steer_preview_truncated()));
    }
    lines.extend([
        Line::from(""),
        AkraTheme::key_line(app.tui_language.turn_steer_confirmation_keys()),
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
    app: &mut NativeTuiApp,
    tail_view: super::shell_presentation::InlineTailView,
    live_transcript_lines: Vec<Line<'static>>,
    layout: &Rc<[Rect]>,
) {
    // 더 좁은 overlay나 더 짧은 tail이 terminal buffer에 stale cell을 남기지 않도록 항상 전체 frame을 먼저 지운다.
    let frame_area = frame.area();
    frame.render_widget(Clear, frame_area);
    // hidden-overlay path는 일반 conversation shell이다.
    // inspection layout을 우회해 transcript가 tail 위의 전체 공간을 채우게 한다.
    if app.shell_overlay == ShellOverlay::Hidden {
        if app.parallel_mode_enabled() && !renders_parallel_viewport_handoff(app) {
            let tail_band = layout.get(1).copied().unwrap_or(frame_area);
            let tail_area = inline_body_render_area(tail_band, &tail_view.lines);
            render_bottom_anchored_tail(
                frame,
                app,
                tail_area,
                tail_view,
                !app.parallel_mode_prompt_input_locked() && !app.is_exit_confirmation_visible(),
            );
            return;
        }
        // startup banner 같은 presentation state는 의도적으로 상단부터 전체 frame을 소유하므로 bottom anchored가 아니어야 한다.
        if tail_view.render_from_top {
            bind_queue_receipt_undo_hit_area(
                app,
                frame_area,
                tail_view.queue_receipt_undo_hit_area,
            );
            render_inline_body(frame, frame_area, tail_view.lines, false);
            if !app.is_exit_confirmation_visible() {
                set_cursor_if_visible(frame, frame_area, tail_view.prompt_cursor_offset);
            }
            return;
        }
        // standard shell에서는 tail 높이를 먼저 재고 live transcript line을 그 위 공간에 clip한다.
        let tail_band = layout.get(1).copied().unwrap_or(frame_area);
        let tail_area = inline_body_render_area(tail_band, &tail_view.lines);
        render_inline_live_transcript(frame, frame_area, tail_area, live_transcript_lines);
        render_bottom_anchored_tail(
            frame,
            app,
            tail_area,
            tail_view,
            !app.is_exit_confirmation_visible(),
        );
        return;
    }
    // overlay/modal이 active이면 layout[0]은 inspection이 쓰고 layout[1]은 그 아래에 tail을 고정한다.
    // exit modal은 두 영역을 모두 덮어야 하므로 이 함수 밖에서 계속 그린다.
    let tail_area = inline_body_render_area(layout[1], &tail_view.lines);
    bind_queue_receipt_undo_hit_area(app, tail_area, tail_view.queue_receipt_undo_hit_area);
    render_inline_body(frame, tail_area, tail_view.lines, false);
    if app.shell_overlay == ShellOverlay::Supersession && !app.parallel_mode_prompt_input_locked() {
        set_cursor_if_visible(frame, tail_area, tail_view.prompt_cursor_offset);
    }
}

pub(super) fn renders_parallel_viewport_handoff(app: &NativeTuiApp) -> bool {
    app.parallel_mode_enabled() && renders_viewport_transcript_handoff(app)
}

pub(super) fn renders_viewport_transcript_handoff(app: &NativeTuiApp) -> bool {
    app.shell_overlay == ShellOverlay::Hidden
        && !app.is_exit_confirmation_visible()
        && !app.is_turn_steer_confirmation_visible()
        && matches!(
            app.inline_history_render_mode,
            InlineHistoryRenderMode::ViewportReplay
        )
        && matches!(
            &app.conversation_state,
            ConversationState::Ready(conversation)
                if conversation
                    .viewport_transcript_handoff_release_messages()
                    .is_some()
        )
}

fn render_bottom_anchored_tail(
    frame: &mut Frame<'_>,
    app: &mut NativeTuiApp,
    tail_area: Rect,
    tail_view: super::shell_presentation::InlineTailView,
    show_cursor: bool,
) {
    let focus_row = tail_view.prompt_cursor_offset.map(|(_, y)| y);
    let dropped_rows = render_inline_body_suffix(frame, tail_area, tail_view.lines, focus_row);
    let hit_area = tail_view
        .queue_receipt_undo_hit_area
        .and_then(|area| scroll_relative_rect(area, dropped_rows));
    let prompt_cursor_offset = tail_view
        .prompt_cursor_offset
        .and_then(|(x, y)| y.checked_sub(dropped_rows).map(|y| (x, y)));

    bind_queue_receipt_undo_hit_area(app, tail_area, hit_area);
    if show_cursor {
        set_cursor_if_visible(frame, tail_area, prompt_cursor_offset);
    }
}

fn scroll_relative_rect(area: Rect, dropped_rows: u16) -> Option<Rect> {
    Some(Rect::new(
        area.x,
        area.y.checked_sub(dropped_rows)?,
        area.width,
        area.height,
    ))
}

fn bind_queue_receipt_undo_hit_area(
    app: &mut NativeTuiApp,
    tail_area: Rect,
    relative_hit_area: Option<Rect>,
) {
    let hit_area = relative_hit_area.filter(|relative| {
        relative.right() <= tail_area.width && relative.bottom() <= tail_area.height
    });
    app.queue_overlay_ui_state
        .bind_receipt_undo_hit_area(hit_area.map(|relative| {
            Rect::new(
                tail_area.x.saturating_add(relative.x),
                tail_area.y.saturating_add(relative.y),
                relative.width,
                relative.height,
            )
        }));
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
