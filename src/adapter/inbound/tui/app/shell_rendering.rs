use std::rc::Rc;

#[cfg(test)]
use ratatui::layout::{Constraint, Direction, Layout};
#[cfg(test)]
use ratatui::widgets::{List, ListItem};

use super::shell_presentation::build_inline_tail_view;
#[cfg(test)]
use super::shell_presentation::{
    AutomationOverlayView, ConversationShellFrameView, OverlayListView,
    PlanningDraftEditorOverlayView, PlanningInitOverlayView, SessionOverlayView,
    StartupOverlayView, SupersessionOverlayView, build_automation_overlay_view,
    build_conversation_shell_frame_view, build_input_prompt_cursor_offset,
    build_planning_draft_editor_overlay_view, build_planning_init_overlay_view,
    build_session_overlay_view, build_startup_overlay_view, build_supersession_overlay_view,
};
use super::*;

#[path = "shell_rendering/inline_inspection.rs"]
mod inline_inspection;
#[path = "shell_rendering/inline_layout.rs"]
mod inline_layout;

use inline_inspection::draw_inline_shell_inspection;
#[cfg(test)]
use inline_layout::clamp_scroll_offset;
use inline_layout::{
    build_inline_terminal_flow_layout, centered_rect, inline_body_render_area,
    inline_body_top_render_area, render_inline_body, set_cursor_if_visible,
};

pub(super) fn prepare_render_state(app: &mut NativeTuiApp, mode: ShellFrontendMode, area: Rect) {
    let _ = mode;
    let directions_editor_open = app.shell_overlay == ShellOverlay::DirectionsMaintenance
        && app.directions_maintenance_overlay_ui_state.step()
            == DirectionsMaintenanceOverlayStep::ManualEditor;
    let planning_editor_open = app.shell_overlay == ShellOverlay::PlanningInit
        && app.planning_init_overlay_ui_state.step() == PlanningInitOverlayStep::ManualEditor;
    if !directions_editor_open && !planning_editor_open {
        return;
    }

    let tail_view = build_inline_tail_view(app, area.width);
    let inspection_area = build_inline_terminal_flow_layout(app, area, &tail_view.lines)[0];
    let editor_content_height = inspection_area
        .height
        .saturating_sub(14)
        .max(6)
        .saturating_sub(1)
        .max(1);
    app.planning_draft_editor_ui_state
        .sync_editor_scroll(editor_content_height);
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &mut NativeTuiApp, mode: ShellFrontendMode) {
    let _ = mode;
    let frame_area = frame.area();
    let tail_view = build_inline_tail_view(app, frame_area.width);
    let layout = build_inline_terminal_flow_layout(app, frame_area, &tail_view.lines);

    draw_inline_conversation_shell(frame, app, tail_view, &layout);

    if app.shell_overlay != ShellOverlay::Hidden {
        draw_inline_shell_inspection(frame, app, layout[0]);
    }

    if app.is_exit_confirmation_visible() {
        draw_exit_confirmation(frame);
    }
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_session_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut NativeTuiApp,
    list_view: OverlayListView,
) {
    if let Some(message_lines) = list_view.message_lines {
        let widget = Paragraph::new(message_lines)
            .block(Block::default().borders(Borders::ALL).title("Threads"))
            .wrap(Wrap { trim: true });
        frame.render_widget(widget, area);
        return;
    }

    let list = List::new(
        list_view
            .items
            .into_iter()
            .map(|item| ListItem::new(item.lines)),
    )
    .block(Block::default().borders(Borders::ALL).title("Threads"))
    .highlight_style(
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(">> ");

    app.session_overlay_ui_state
        .sync_selected_session(list_view.selected_index);
    frame.render_stateful_widget(list, area, &mut app.session_overlay_ui_state.list_state);
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_session_detail_panel(frame: &mut Frame<'_>, area: Rect, lines: Vec<Line<'static>>) {
    let detail = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Selected Session"),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, area);
}

fn draw_inline_conversation_shell(
    frame: &mut Frame<'_>,
    app: &mut NativeTuiApp,
    tail_view: super::shell_presentation::InlineTailView,
    layout: &Rc<[Rect]>,
) {
    let frame_area = frame.area();
    frame.render_widget(Clear, frame_area);
    if app.shell_overlay == ShellOverlay::Hidden && !app.is_exit_confirmation_visible() {
        let tail_area = if tail_view.render_from_top {
            frame_area
        } else {
            inline_body_top_render_area(frame_area, &tail_view.lines)
        };
        render_inline_body(frame, tail_area, tail_view.lines, false);
        set_cursor_if_visible(frame, tail_area, tail_view.prompt_cursor_offset);
        return;
    }

    render_inline_body(
        frame,
        inline_body_render_area(layout[1], &tail_view.lines),
        tail_view.lines,
        false,
    );
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_framed_conversation_shell(
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
        .block(Block::default().borders(Borders::ALL).title(shell_title))
        .wrap(Wrap { trim: true });
    frame.render_widget(header, header_area);

    let conversation = Paragraph::new(transcript_view.lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(transcript_view.title),
        )
        .scroll((transcript_view.scroll_offset, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(conversation, transcript_area);

    let footer = Paragraph::new(footer_lines)
        .block(Block::default().borders(Borders::ALL).title(status_title))
        .wrap(Wrap { trim: true });
    frame.render_widget(footer, footer_area);

    let input = Paragraph::new(input_lines)
        .block(Block::default().borders(Borders::ALL).title(input_title))
        .wrap(Wrap { trim: false });
    frame.render_widget(input, input_area);

    if app.shell_overlay == ShellOverlay::Hidden && !app.is_exit_confirmation_visible() {
        let input_content_area = Block::default().borders(Borders::ALL).inner(input_area);
        set_cursor_if_visible(
            frame,
            input_content_area,
            build_input_prompt_cursor_offset(app, input_content_area.width),
        );
    }
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_startup_overlay(frame: &mut Frame<'_>, app: &NativeTuiApp) {
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

    let header = Paragraph::new(header_lines)
        .block(Block::default().borders(Borders::ALL).title("Diagnostics"));
    frame.render_widget(header, layout[0]);

    frame.render_widget(
        Paragraph::new(summary_lines)
            .block(Block::default().borders(Borders::ALL).title("Startup"))
            .wrap(Wrap { trim: true }),
        layout[1],
    );

    frame.render_widget(
        List::new(check_lines).block(Block::default().borders(Borders::ALL).title("Checks")),
        layout[2],
    );

    frame.render_widget(
        Paragraph::new(warning_lines)
            .block(Block::default().borders(Borders::ALL).title("Warnings"))
            .wrap(Wrap { trim: true }),
        layout[3],
    );

    frame.render_widget(
        Paragraph::new(key_lines).block(Block::default().borders(Borders::ALL).title("Keys")),
        layout[4],
    );
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_session_overlay(frame: &mut Frame<'_>, app: &mut NativeTuiApp) {
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

    let header = Paragraph::new(header_lines)
        .block(Block::default().borders(Borders::ALL).title("Sessions"));
    frame.render_widget(header, layout[0]);

    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(layout[1]);

    draw_session_list_panel(frame, content_layout[0], app, list_view);
    draw_session_detail_panel(frame, content_layout[1], detail_lines);

    frame.render_widget(
        Paragraph::new(warning_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Session Warnings"),
            )
            .wrap(Wrap { trim: true }),
        layout[2],
    );

    frame.render_widget(
        Paragraph::new(key_lines).block(Block::default().borders(Borders::ALL).title("Keys")),
        layout[3],
    );
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_supersession_overlay(frame: &mut Frame<'_>, app: &NativeTuiApp) {
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
        Paragraph::new(header_lines)
            .block(Block::default().borders(Borders::ALL).title("Supersession")),
        layout[0],
    );
    frame.render_widget(
        Paragraph::new(summary_lines)
            .block(Block::default().borders(Borders::ALL).title("Summary"))
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
            .block(Block::default().borders(Borders::ALL).title("Capabilities"))
            .wrap(Wrap { trim: false }),
        left_layout[0],
    );
    frame.render_widget(
        Paragraph::new(pool_lines)
            .block(Block::default().borders(Borders::ALL).title("Pool Board"))
            .wrap(Wrap { trim: false }),
        left_layout[1],
    );
    frame.render_widget(
        Paragraph::new(roster_lines)
            .block(Block::default().borders(Borders::ALL).title("Agent Roster"))
            .wrap(Wrap { trim: false }),
        left_layout[2],
    );
    frame.render_widget(
        Paragraph::new(detail_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Selected Detail"),
            )
            .wrap(Wrap { trim: false }),
        right_layout[0],
    );
    frame.render_widget(
        Paragraph::new(distributor_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Distributor / Queue"),
            )
            .wrap(Wrap { trim: false }),
        right_layout[1],
    );
    frame.render_widget(
        Paragraph::new(key_lines).block(Block::default().borders(Borders::ALL).title("Keys")),
        layout[3],
    );
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_automation_overlay(frame: &mut Frame<'_>, app: &mut NativeTuiApp) {
    let overlay_view = build_automation_overlay_view(app);
    let AutomationOverlayView {
        header_lines,
        list_view,
        preview_lines,
        status_lines,
        key_lines,
    } = overlay_view;
    let popup_area = centered_rect(92, 82, frame.area());
    frame.render_widget(Clear, popup_area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(14),
            Constraint::Length(block_height_for_lines(&status_lines, 6, 11)),
            Constraint::Length(block_height_for_lines(&key_lines, 5, 7)),
        ])
        .split(popup_area);

    let header = Paragraph::new(header_lines)
        .block(Block::default().borders(Borders::ALL).title("Automation"));
    frame.render_widget(header, layout[0]);

    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
        .split(layout[1]);

    let preview_scroll = clamp_scroll_offset(
        app.followup_overlay_ui_state.preview_scroll,
        &preview_lines,
        content_layout[1].width.saturating_sub(2),
        content_layout[1].height.saturating_sub(2),
    );
    app.followup_overlay_ui_state.preview_scroll = preview_scroll;

    draw_automation_list_panel(frame, content_layout[0], app, list_view);
    frame.render_widget(
        Paragraph::new(preview_lines)
            .block(Block::default().borders(Borders::ALL).title("Preview"))
            .scroll((preview_scroll, 0))
            .wrap(Wrap { trim: false }),
        content_layout[1],
    );

    frame.render_widget(
        Paragraph::new(status_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Auto Follow-Up State"),
            )
            .wrap(Wrap { trim: false }),
        layout[2],
    );

    frame.render_widget(
        Paragraph::new(key_lines).block(Block::default().borders(Borders::ALL).title("Keys")),
        layout[3],
    );
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_planning_init_overlay(frame: &mut Frame<'_>, app: &mut NativeTuiApp) {
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
        Paragraph::new(header_lines)
            .block(Block::default().borders(Borders::ALL).title("Planning")),
        layout[0],
    );
    frame.render_widget(
        Paragraph::new(summary_lines)
            .block(Block::default().borders(Borders::ALL).title("Summary"))
            .wrap(Wrap { trim: true }),
        layout[1],
    );
    frame.render_widget(
        Paragraph::new(option_lines)
            .block(Block::default().borders(Borders::ALL).title("Options"))
            .wrap(Wrap { trim: false }),
        layout[2],
    );
    frame.render_widget(
        Paragraph::new(status_lines)
            .block(Block::default().borders(Borders::ALL).title("Status"))
            .wrap(Wrap { trim: true }),
        layout[3],
    );
    frame.render_widget(
        Paragraph::new(key_lines).block(Block::default().borders(Borders::ALL).title("Keys")),
        layout[4],
    );
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_planning_draft_editor_overlay(frame: &mut Frame<'_>, app: &mut NativeTuiApp) {
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
        Paragraph::new(header_lines)
            .block(Block::default().borders(Borders::ALL).title("Planning")),
        layout[0],
    );

    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(28), Constraint::Percentage(72)])
        .split(layout[1]);

    frame.render_widget(
        Paragraph::new(file_lines)
            .block(Block::default().borders(Borders::ALL).title("Files"))
            .wrap(Wrap { trim: false }),
        content_layout[0],
    );
    frame.render_widget(
        Paragraph::new(editor_lines)
            .block(Block::default().borders(Borders::ALL).title(editor_title))
            .scroll((editor_scroll, 0))
            .wrap(Wrap { trim: false }),
        content_layout[1],
    );
    let editor_content_area = Block::default()
        .borders(Borders::ALL)
        .inner(content_layout[1]);
    set_cursor_if_visible(frame, editor_content_area, editor_cursor_offset);

    frame.render_widget(
        Paragraph::new(status_lines)
            .block(Block::default().borders(Borders::ALL).title("Status"))
            .wrap(Wrap { trim: false }),
        layout[2],
    );
    frame.render_widget(
        Paragraph::new(key_lines).block(Block::default().borders(Borders::ALL).title("Keys")),
        layout[3],
    );
}

fn draw_exit_confirmation(frame: &mut Frame<'_>) {
    let popup_area = centered_rect(42, 22, frame.area());
    frame.render_widget(Clear, popup_area);

    let popup = Paragraph::new(vec![
        Line::from("You are already at the shell home."),
        Line::from("Exit codex-exec-loop?"),
        Line::from(""),
        Line::from("y: exit    n: stay"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Confirm Exit"))
    .wrap(Wrap { trim: true });

    frame.render_widget(popup, popup_area);
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_automation_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut NativeTuiApp,
    list_view: OverlayListView,
) {
    if let Some(message_lines) = list_view.message_lines {
        let widget = Paragraph::new(message_lines)
            .block(Block::default().borders(Borders::ALL).title("Automation"))
            .wrap(Wrap { trim: true });
        frame.render_widget(widget, area);
        return;
    }

    let list = List::new(
        list_view
            .items
            .into_iter()
            .map(|item| ListItem::new(item.lines)),
    )
    .block(Block::default().borders(Borders::ALL).title("Automation"))
    .highlight_style(
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(">> ");

    app.followup_overlay_ui_state
        .list_state
        .select(list_view.selected_index);
    frame.render_stateful_widget(list, area, &mut app.followup_overlay_ui_state.list_state);
}

#[cfg(test)]
#[path = "shell_rendering_tests.rs"]
mod tests;
