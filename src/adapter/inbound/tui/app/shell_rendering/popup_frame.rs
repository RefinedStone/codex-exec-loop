#[cfg(test)]
use super::super::PlanningInitOverlayStep;
#[cfg(test)]
use super::super::shell_presentation::{
    ConversationShellFrameView, PlanningDraftEditorOverlayView, PlanningInitOverlayView,
    SessionOverlayView, StartupOverlayView, SupersessionOverlayView,
    build_conversation_shell_frame_view, build_input_prompt_cursor_offset,
    build_planning_draft_editor_overlay_view, build_planning_init_overlay_view,
    build_session_overlay_view, build_startup_overlay_view, build_supersession_overlay_view,
};
use super::super::{AkraTheme, Frame, Line};
#[cfg(test)]
use super::super::{NativeTuiApp, ShellOverlay};
#[cfg(test)]
use super::super::{ShellFrontendMode, block_height_for_lines};
use super::inline_layout::centered_rect;
#[cfg(test)]
use super::inline_layout::set_cursor_if_visible;
#[cfg(test)]
use super::popup_helpers::{draw_session_detail_panel, draw_session_list_panel};
#[cfg(test)]
use ratatui::layout::{Constraint, Direction, Layout};
#[cfg(test)]
use ratatui::widgets::List;
use ratatui::widgets::{Clear, Paragraph, Wrap};

// exit confirmation은 inspection surface 대부분이 inline으로 이동한 뒤에도 live shell path가 쓰는 유일한 popup-frame renderer다.
// centered area를 먼저 지워 startup chrome과 conversation chrome 어느 쪽 위에서도 modal처럼 보이게 한다.
pub(super) fn draw_exit_confirmation(frame: &mut Frame<'_>) {
    let popup_area = centered_rect(42, 22, frame.area());
    frame.render_widget(Clear, popup_area);
    let popup = Paragraph::new(vec![
        Line::from("You are already at the shell home."),
        Line::from("Exit codex-exec-loop?"),
        Line::from(""),
        AkraTheme::key_line("y: exit    n: stay"),
    ])
    .block(AkraTheme::panel_block(AkraTheme::title_line(
        "Confirm Exit",
        "",
    )))
    .wrap(Wrap { trim: true });

    frame.render_widget(popup, popup_area);
}

// 나머지 renderer는 shell_presentation view model을 감싸는 test-only adapter다.
// production code는 inline shell surface를 쓰지만, snapshot/contract test는 이 경계로 legacy popup layout 비교 가능성을 유지한다.
#[cfg(test)]
#[allow(dead_code)]
pub(super) fn draw_framed_conversation_shell(
    frame: &mut Frame<'_>,
    app: &mut NativeTuiApp,
    mode: ShellFrontendMode,
) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let shell_frame_view = build_conversation_shell_frame_view(app, mode, area);
    let ConversationShellFrameView {
        shell_title,
        header_lines,
        header_area,
        transcript_view,
        transcript_area,
        status_title,
        footer_lines,
        footer_area,
        input_title,
        input_lines,
        input_area,
    } = shell_frame_view;
    let header = Paragraph::new(header_lines)
        .block(AkraTheme::panel_block(shell_title))
        .wrap(Wrap { trim: true });
    frame.render_widget(header, header_area);
    let conversation = Paragraph::new(transcript_view.lines)
        .block(AkraTheme::panel_block(transcript_view.title))
        .scroll((transcript_view.scroll_offset, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(conversation, transcript_area);
    let footer = Paragraph::new(footer_lines)
        .block(AkraTheme::panel_block(status_title))
        .wrap(Wrap { trim: true });
    frame.render_widget(footer, footer_area);
    let input = Paragraph::new(input_lines)
        .block(AkraTheme::panel_block(input_title))
        .wrap(Wrap { trim: false });
    frame.render_widget(input, input_area);

    // popup snapshot은 modal이나 inspection overlay가 focus를 소유하지 않을 때만 prompt cursor를 보여야 한다.
    // live inline renderer의 cursor ownership rule과 같은 조건이다.
    if app.shell_overlay == ShellOverlay::Hidden && !app.is_exit_confirmation_visible() {
        let input_content_area = AkraTheme::panel_inner(input_area);
        set_cursor_if_visible(
            frame,
            input_content_area,
            build_input_prompt_cursor_offset(app, input_content_area.width),
        );
    }
}

#[cfg(test)]
#[allow(dead_code)]
pub(super) fn draw_startup_overlay(frame: &mut Frame<'_>, app: &NativeTuiApp) {
    let overlay_view = build_startup_overlay_view(app);
    let StartupOverlayView {
        header_lines,
        summary_lines,
        check_lines,
        warning_lines,
        key_lines,
    } = overlay_view;
    let popup_area = centered_rect(78, 72, frame.area());
    frame.render_widget(Clear, popup_area);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(7),
            Constraint::Min(10),
            Constraint::Length(6),
            Constraint::Length(3),
        ])
        .split(popup_area);
    let header = Paragraph::new(header_lines).block(AkraTheme::panel_block("Diagnostics"));
    frame.render_widget(header, layout[0]);

    frame.render_widget(
        Paragraph::new(summary_lines)
            .block(AkraTheme::panel_block("Startup"))
            .wrap(Wrap { trim: true }),
        layout[1],
    );

    frame.render_widget(
        List::new(check_lines).block(AkraTheme::panel_block("Checks")),
        layout[2],
    );

    frame.render_widget(
        Paragraph::new(warning_lines)
            .block(AkraTheme::panel_block("Warnings"))
            .wrap(Wrap { trim: true }),
        layout[3],
    );

    frame.render_widget(
        Paragraph::new(key_lines).block(AkraTheme::panel_block("Keys")),
        layout[4],
    );
}

// session snapshot은 왼쪽 navigable list, 오른쪽 selected-session detail, 아래 warning/key 영역으로 된 legacy two-column popup을 보존한다.
#[cfg(test)]
#[allow(dead_code)]
pub(super) fn draw_session_overlay(frame: &mut Frame<'_>, app: &mut NativeTuiApp) {
    let overlay_view = build_session_overlay_view(app);
    let SessionOverlayView {
        header_lines,
        list_view,
        detail_lines,
        warning_lines,
        key_lines,
    } = overlay_view;
    let popup_area = centered_rect(90, 78, frame.area());
    frame.render_widget(Clear, popup_area);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(block_height_for_lines(&header_lines, 3, 4)),
            Constraint::Min(12),
            Constraint::Length(block_height_for_lines(&warning_lines, 4, 6)),
            Constraint::Length(block_height_for_lines(&key_lines, 3, 5)),
        ])
        .split(popup_area);
    let header = Paragraph::new(header_lines).block(AkraTheme::panel_block("Sessions"));
    frame.render_widget(header, layout[0]);
    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(layout[1]);

    draw_session_list_panel(frame, content_layout[0], app, list_view);
    draw_session_detail_panel(frame, content_layout[1], detail_lines);

    frame.render_widget(
        Paragraph::new(warning_lines)
            .block(AkraTheme::panel_block("Session Warnings"))
            .wrap(Wrap { trim: true }),
        layout[2],
    );

    frame.render_widget(
        Paragraph::new(key_lines).block(AkraTheme::panel_block("Keys")),
        layout[3],
    );
}

// supersession diagnostics는 capability readiness, pool board, live roster, selected slot detail, distributor queue state를 한 popup에 압축한다.
// parallel-mode control surface 전반의 회귀를 contract test가 한 화면에서 잡을 수 있게 하는 구성이다.
#[cfg(test)]
#[allow(dead_code)]
pub(super) fn draw_supersession_overlay(frame: &mut Frame<'_>, app: &NativeTuiApp) {
    let overlay_view = build_supersession_overlay_view(app);
    let SupersessionOverlayView {
        header_lines,
        summary_lines,
        capability_lines,
        pool_lines,
        roster_lines,
        detail_lines,
        distributor_lines,
        key_lines,
    } = overlay_view;
    let popup_area = centered_rect(92, 78, frame.area());
    frame.render_widget(Clear, popup_area);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(block_height_for_lines(&header_lines, 3, 4)),
            Constraint::Length(block_height_for_lines(&summary_lines, 6, 9)),
            Constraint::Min(16),
            Constraint::Length(block_height_for_lines(&key_lines, 3, 5)),
        ])
        .split(popup_area);

    frame.render_widget(
        Paragraph::new(header_lines).block(AkraTheme::panel_block("Supersession")),
        layout[0],
    );
    frame.render_widget(
        Paragraph::new(summary_lines)
            .block(AkraTheme::panel_block("Summary"))
            .wrap(Wrap { trim: true }),
        layout[1],
    );
    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(layout[2]);
    let left_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(block_height_for_lines(&capability_lines, 5, 9)),
            Constraint::Length(block_height_for_lines(&pool_lines, 5, 9)),
            Constraint::Min(6),
        ])
        .split(content_layout[0]);
    let right_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(block_height_for_lines(&detail_lines, 5, 8)),
            Constraint::Min(8),
        ])
        .split(content_layout[1]);

    frame.render_widget(
        Paragraph::new(capability_lines)
            .block(AkraTheme::panel_block("Capabilities"))
            .wrap(Wrap { trim: false }),
        left_layout[0],
    );
    frame.render_widget(
        Paragraph::new(pool_lines)
            .block(AkraTheme::panel_block("Pool Board"))
            .wrap(Wrap { trim: false }),
        left_layout[1],
    );
    frame.render_widget(
        Paragraph::new(roster_lines)
            .block(AkraTheme::panel_block("Agent Roster"))
            .wrap(Wrap { trim: false }),
        left_layout[2],
    );
    frame.render_widget(
        Paragraph::new(detail_lines)
            .block(AkraTheme::panel_block("Selected Detail"))
            .wrap(Wrap { trim: false }),
        right_layout[0],
    );
    frame.render_widget(
        Paragraph::new(distributor_lines)
            .block(AkraTheme::panel_block("Distributor / Queue"))
            .wrap(Wrap { trim: false }),
        right_layout[1],
    );
    frame.render_widget(
        Paragraph::new(key_lines).block(AkraTheme::panel_block("Keys")),
        layout[3],
    );
}

// planning init은 option picker를 직접 render하거나 draft editor step으로 위임한다.
// 이 branch를 renderer 경계에 남겨 test가 live terminal input 없이도 controller와 같은 view-state 전이를 구동하게 한다.
#[cfg(test)]
#[allow(dead_code)]
pub(super) fn draw_planning_init_overlay(frame: &mut Frame<'_>, app: &mut NativeTuiApp) {
    if app.planning_init_overlay_ui_state.step() == PlanningInitOverlayStep::ManualEditor {
        draw_planning_draft_editor_overlay(frame, app);
        return;
    }
    let overlay_view = build_planning_init_overlay_view(app);
    let PlanningInitOverlayView {
        header_lines,
        summary_lines,
        option_lines,
        status_lines,
        key_lines,
    } = overlay_view;
    let popup_area = centered_rect(86, 70, frame.area());
    frame.render_widget(Clear, popup_area);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(block_height_for_lines(&header_lines, 3, 4)),
            Constraint::Length(block_height_for_lines(&summary_lines, 4, 6)),
            Constraint::Min(8),
            Constraint::Length(block_height_for_lines(&status_lines, 4, 6)),
            Constraint::Length(block_height_for_lines(&key_lines, 3, 5)),
        ])
        .split(popup_area);

    frame.render_widget(
        Paragraph::new(header_lines).block(AkraTheme::panel_block("Planning")),
        layout[0],
    );
    frame.render_widget(
        Paragraph::new(summary_lines)
            .block(AkraTheme::panel_block("Summary"))
            .wrap(Wrap { trim: true }),
        layout[1],
    );
    frame.render_widget(
        Paragraph::new(option_lines)
            .block(AkraTheme::panel_block("Options"))
            .wrap(Wrap { trim: false }),
        layout[2],
    );
    frame.render_widget(
        Paragraph::new(status_lines)
            .block(AkraTheme::panel_block("Status"))
            .wrap(Wrap { trim: true }),
        layout[3],
    );
    frame.render_widget(
        Paragraph::new(key_lines).block(AkraTheme::panel_block("Keys")),
        layout[4],
    );
}

// draft editor popup은 file selection과 text editing을 동시에 보이게 유지한다.
// cursor placement는 presentation view에서 계산해 inline planning inspection과 같은 scroll/cursor projection을 test가 검증하게 한다.
#[cfg(test)]
#[allow(dead_code)]
pub(super) fn draw_planning_draft_editor_overlay(frame: &mut Frame<'_>, app: &mut NativeTuiApp) {
    let popup_area = centered_rect(92, 82, frame.area());
    frame.render_widget(Clear, popup_area);
    let editor_height = popup_area.height.saturating_sub(14).max(8);
    let Some(overlay_view) =
        build_planning_draft_editor_overlay_view(app, editor_height.saturating_sub(2).max(1))
    else {
        return;
    };
    let PlanningDraftEditorOverlayView {
        header_lines,
        file_lines,
        editor_title,
        editor_lines,
        editor_scroll,
        editor_cursor_offset,
        status_lines,
        key_lines,
    } = overlay_view;
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(block_height_for_lines(&header_lines, 3, 4)),
            Constraint::Min(editor_height),
            Constraint::Length(block_height_for_lines(&status_lines, 5, 8)),
            Constraint::Length(block_height_for_lines(&key_lines, 4, 6)),
        ])
        .split(popup_area);

    frame.render_widget(
        Paragraph::new(header_lines).block(AkraTheme::panel_block("Planning")),
        layout[0],
    );
    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(28), Constraint::Percentage(72)])
        .split(layout[1]);

    frame.render_widget(
        Paragraph::new(file_lines)
            .block(AkraTheme::panel_block("Files"))
            .wrap(Wrap { trim: false }),
        content_layout[0],
    );
    frame.render_widget(
        Paragraph::new(editor_lines)
            .block(AkraTheme::panel_block(editor_title))
            .scroll((editor_scroll, 0))
            .wrap(Wrap { trim: false }),
        content_layout[1],
    );
    let editor_content_area = AkraTheme::panel_inner(content_layout[1]);
    set_cursor_if_visible(frame, editor_content_area, editor_cursor_offset);

    frame.render_widget(
        Paragraph::new(status_lines)
            .block(AkraTheme::panel_block("Status"))
            .wrap(Wrap { trim: false }),
        layout[2],
    );
    frame.render_widget(
        Paragraph::new(key_lines).block(AkraTheme::panel_block("Keys")),
        layout[3],
    );
}
