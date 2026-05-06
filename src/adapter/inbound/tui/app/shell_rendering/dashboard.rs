use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use crate::adapter::inbound::tui::supersession_mud::build_supersession_mud_view;

use super::super::shell_presentation::{
    build_directions_maintenance_overlay_view, build_help_overlay_view,
    build_inline_live_transcript_lines, build_inline_tail_view, build_planning_init_overlay_view,
    build_queue_overlay_view, build_session_overlay_view, build_startup_overlay_view,
    build_supersession_overlay_view, build_task_intake_overlay_view,
};
use super::super::{
    AkraTheme, ConversationMessageKind, ConversationState, NativeTuiApp, ShellOverlay,
};
use super::set_cursor_if_visible;

pub(super) struct DashboardView {
    masthead_lines: Vec<Line<'static>>,
    panels: Vec<DashboardPanelView>,
    prompt_view: DashboardPromptView,
    key_lines: Vec<Line<'static>>,
}

pub(super) struct DashboardPanelView {
    title: &'static str,
    lines: Vec<Line<'static>>,
}

pub(super) struct DashboardPromptView {
    lines: Vec<Line<'static>>,
    cursor_offset: Option<(u16, u16)>,
}

pub(super) fn draw_dashboard_shell(frame: &mut Frame<'_>, app: &mut NativeTuiApp) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let view = build_dashboard_view(app, area.width);
    let layout = build_dashboard_layout(area);

    render_panel(frame, layout[0], "AKRA Command Deck", view.masthead_lines);
    render_dashboard_panels(frame, layout[1], view.panels);
    render_prompt(frame, layout[2], view.prompt_view);
    render_key_bar(frame, layout[3], view.key_lines);
}

fn build_dashboard_layout(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(prompt_height(area.height)),
            Constraint::Length(1),
        ])
        .split(area)
}

fn prompt_height(frame_height: u16) -> u16 {
    if frame_height < 22 { 5 } else { 7 }
}

fn render_dashboard_panels(frame: &mut Frame<'_>, area: Rect, panels: Vec<DashboardPanelView>) {
    if area.width < 96 {
        let visible_count = if area.height < 13 {
            3
        } else {
            panels.len().min(6)
        };
        let constraints = (0..visible_count)
            .map(|_| Constraint::Ratio(1, visible_count as u32))
            .collect::<Vec<_>>();
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);
        for (panel, panel_area) in panels.into_iter().take(visible_count).zip(rows.iter()) {
            render_panel(frame, *panel_area, panel.title, panel.lines);
        }
        return;
    }

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);
    let left = split_column(horizontal[0]);
    let middle = split_column(horizontal[1]);
    let right = split_column(horizontal[2]);
    let areas = [left[0], left[1], middle[0], middle[1], right[0], right[1]];
    for (panel, panel_area) in panels.into_iter().zip(areas) {
        render_panel(frame, panel_area, panel.title, panel.lines);
    }
}

fn split_column(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area)
}

fn render_panel(frame: &mut Frame<'_>, area: Rect, title: &'static str, lines: Vec<Line<'static>>) {
    let paragraph = Paragraph::new(lines)
        .block(AkraTheme::panel_block(title))
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

fn render_prompt(frame: &mut Frame<'_>, area: Rect, prompt_view: DashboardPromptView) {
    let block = AkraTheme::panel_block("Prompt Primary");
    let inner = block.inner(area);
    let paragraph = Paragraph::new(prompt_view.lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
    set_cursor_if_visible(frame, inner, prompt_view.cursor_offset);
}

fn render_key_bar(frame: &mut Frame<'_>, area: Rect, key_lines: Vec<Line<'static>>) {
    let line = key_lines.into_iter().next().unwrap_or_else(|| {
        Line::from("Tab panels | Ctrl+d diagnostics | Ctrl+o sessions | Ctrl+q quit")
    });
    frame.render_widget(Paragraph::new(line), area);
}

fn build_dashboard_view(app: &NativeTuiApp, width: u16) -> DashboardView {
    let tail_view = build_inline_tail_view(app, width.saturating_sub(4));
    DashboardView {
        masthead_lines: build_masthead_lines(app),
        panels: build_dashboard_panels(app),
        prompt_view: DashboardPromptView {
            lines: tail_view.lines,
            cursor_offset: tail_view.prompt_cursor_offset,
        },
        key_lines: vec![Line::from(
            "Tab/Shift+Tab panels | arrows select | prompt owns text/Space/Backspace/Enter | Ctrl+d diagnostics | Ctrl+o sessions | Ctrl+q quit",
        )],
    }
}

fn build_masthead_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    vec![Line::from(vec![
        Span::styled("AKRA", AkraTheme::brand()),
        Span::raw(" / native dashboard / "),
        Span::raw(match app.shell_overlay {
            ShellOverlay::Hidden => "home",
            ShellOverlay::Startup => "startup",
            ShellOverlay::Sessions => "sessions",
            ShellOverlay::Supersession => "parallel",
            ShellOverlay::Queue => "queue",
            ShellOverlay::PlanningInit => "planning",
            ShellOverlay::TaskIntake => "task",
            ShellOverlay::DirectionsMaintenance => "directions",
            ShellOverlay::Help => "help",
        }),
    ])]
}

fn build_dashboard_panels(app: &NativeTuiApp) -> Vec<DashboardPanelView> {
    match app.shell_overlay {
        ShellOverlay::Startup => startup_panels(app),
        ShellOverlay::Sessions => session_panels(app),
        ShellOverlay::Supersession => supersession_panels(app),
        ShellOverlay::Queue => queue_panels(app),
        ShellOverlay::PlanningInit => planning_panels(app),
        ShellOverlay::TaskIntake => task_panels(app),
        ShellOverlay::DirectionsMaintenance => directions_panels(app),
        ShellOverlay::Help => help_panels(app),
        ShellOverlay::Hidden => home_panels(app),
    }
}

fn home_panels(app: &NativeTuiApp) -> Vec<DashboardPanelView> {
    vec![
        panel("Transcript", transcript_lines(app)),
        panel("Runtime Status", status_lines(app)),
        panel("Planning Queue", queue_summary_lines(app)),
        panel("Recent Events", recent_event_lines(app)),
        panel("Command Hints", command_hint_lines()),
        panel("System Status", system_status_lines(app)),
    ]
}

fn startup_panels(app: &NativeTuiApp) -> Vec<DashboardPanelView> {
    let view = build_startup_overlay_view(app);
    vec![
        panel("Diagnostics", view.header_lines),
        panel("Checks", view.check_lines),
        panel("Warnings", view.warning_lines),
        panel("Session Readiness", view.summary_lines),
        panel("Command Hints", view.key_lines),
        panel("System Status", system_status_lines(app)),
    ]
}

fn session_panels(app: &NativeTuiApp) -> Vec<DashboardPanelView> {
    let view = build_session_overlay_view(app);
    let mut browser_lines = view.list_view.message_lines.unwrap_or_default();
    browser_lines.extend(
        view.list_view
            .items
            .into_iter()
            .flat_map(|entry| entry.lines),
    );
    vec![
        panel("Session Browser", browser_lines),
        panel("Selected Session Detail", view.detail_lines),
        panel("Warnings", view.warning_lines),
        panel("Keys", view.key_lines),
        panel("Recent Events", recent_event_lines(app)),
        panel("System Status", view.header_lines),
    ]
}

fn supersession_panels(app: &NativeTuiApp) -> Vec<DashboardPanelView> {
    let overlay = build_supersession_overlay_view(app);
    let snapshot = app.parallel_mode_supervisor_snapshot();
    let mud = build_supersession_mud_view(&snapshot, &app.supersession_mud_ui_state);
    vec![
        panel("Agent Tavern", overlay.roster_lines),
        panel("Distributor", overlay.distributor_lines),
        panel("Worktree Pool", overlay.pool_lines),
        panel(
            "Realm Map",
            mud.pool_lines.into_iter().map(Line::from).collect(),
        ),
        panel("Quest Log", overlay.detail_lines),
        panel(
            "Event Feed / System Status",
            event_feed_lines(
                overlay.summary_lines,
                overlay.capability_lines,
                overlay.key_lines,
            ),
        ),
    ]
}

fn queue_panels(app: &NativeTuiApp) -> Vec<DashboardPanelView> {
    let view = build_queue_overlay_view(app);
    vec![
        panel("Planning Board", view.queue_lines),
        panel("Options", view.proposal_lines),
        panel("Validation", view.summary_lines),
        panel("Status", view.note_lines),
        panel("Keys", view.key_lines),
        panel("System Status", view.header_lines),
    ]
}

fn planning_panels(app: &NativeTuiApp) -> Vec<DashboardPanelView> {
    let view = build_planning_init_overlay_view(app);
    vec![
        panel("Planning Board", view.summary_lines),
        panel("Options", view.option_lines),
        panel("Editor Preview", Vec::new()),
        panel("Validation", view.status_lines),
        panel("Keys", view.key_lines),
        panel("System Status", view.header_lines),
    ]
}

fn task_panels(app: &NativeTuiApp) -> Vec<DashboardPanelView> {
    let view = build_task_intake_overlay_view(app);
    vec![
        panel("Planning Board", view.prompt_lines),
        panel("Options", view.preview_lines),
        panel("Validation", view.status_lines),
        panel("Keys", view.key_lines),
        panel("Recent Events", recent_event_lines(app)),
        panel("System Status", view.header_lines),
    ]
}

fn directions_panels(app: &NativeTuiApp) -> Vec<DashboardPanelView> {
    let view = build_directions_maintenance_overlay_view(app);
    vec![
        panel("Planning Board", view.summary_lines),
        panel("Options", view.option_lines),
        panel("Editor Preview", Vec::new()),
        panel("Validation", view.status_lines),
        panel("Keys", view.key_lines),
        panel("System Status", view.header_lines),
    ]
}

fn help_panels(app: &NativeTuiApp) -> Vec<DashboardPanelView> {
    let view = build_help_overlay_view();
    vec![
        panel("Command Catalog", view.command_lines),
        panel("Keymap", view.key_lines),
        panel("Runtime Status", status_lines(app)),
        panel("Planning Board", queue_summary_lines(app)),
        panel("Recent Events", recent_event_lines(app)),
        panel("System Status", system_status_lines(app)),
    ]
}

fn panel(title: &'static str, lines: Vec<Line<'static>>) -> DashboardPanelView {
    DashboardPanelView {
        title,
        lines: non_empty_lines(lines),
    }
}

fn non_empty_lines(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    if lines.is_empty() {
        vec![Line::from("none")]
    } else {
        lines
    }
}

fn transcript_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    let mut lines = match &app.conversation_state {
        ConversationState::Ready(conversation) => conversation.cached_conversation_lines.clone(),
        ConversationState::Loading => vec![Line::from("Loading thread history...")],
        ConversationState::Failed(message) => vec![Line::from(message.clone())],
    };
    lines.extend(build_inline_live_transcript_lines(app));
    non_empty_lines(lines)
}

fn status_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    build_inline_tail_view(app, 96)
        .lines
        .into_iter()
        .take(5)
        .collect()
}

fn queue_summary_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    build_queue_overlay_view(app)
        .summary_lines
        .into_iter()
        .take(8)
        .collect()
}

fn recent_event_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let ConversationState::Ready(conversation) = &app.conversation_state {
        lines.extend(
            conversation
                .messages
                .iter()
                .rev()
                .take(6)
                .map(|message| Line::from(format!("{}: {}", message_label(message), message.text))),
        );
    }
    non_empty_lines(lines)
}

fn message_label(message: &crate::domain::conversation::ConversationMessage) -> String {
    if let Some(label) = &message.display_label {
        return label.clone();
    }
    match message.kind {
        ConversationMessageKind::User => "user",
        ConversationMessageKind::Agent => "agent",
        ConversationMessageKind::Tool => "tool",
        ConversationMessageKind::Status => "status",
    }
    .to_string()
}

fn command_hint_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(":parallel opens the parallel board"),
        Line::from(":queue opens accepted planning work"),
        Line::from(":task drafts a planning task"),
        Line::from("Ctrl+j inserts a prompt newline"),
    ]
}

fn system_status_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    vec![
        Line::from(format!("workspace: {}", app.current_workspace_directory())),
        Line::from(format!(
            "parallel: {}",
            if app.parallel_mode_enabled() {
                "enabled"
            } else {
                "off"
            }
        )),
        Line::from(format!(
            "panel focus: {:?}",
            app.dashboard_ui_state.focused_panel()
        )),
    ]
}

fn event_feed_lines(
    mut summary: Vec<Line<'static>>,
    capability: Vec<Line<'static>>,
    keys: Vec<Line<'static>>,
) -> Vec<Line<'static>> {
    summary.extend(capability);
    summary.extend(keys);
    summary
}
