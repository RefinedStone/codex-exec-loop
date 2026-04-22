use ratatui::text::Line;

use super::NativeTuiApp;
use super::overlays::{OverlayListEntryView, OverlayListView};
use crate::adapter::inbound::tui::shell_chrome::{SessionState, StartupState};
#[cfg(test)]
use crate::application::service::session_service::SessionProjectFilterOption;
use crate::application::service::session_service::{
    SessionBrowserProjection, SessionBrowserView, SessionProjectFilter, build_session_browser_view,
};
use crate::domain::session_summary::SessionSummary;

pub(super) fn recent_session_status_label(app: &NativeTuiApp) -> String {
    if !app.can_open_session_list() {
        return match &app.startup_state {
            StartupState::Loading => "waiting for startup checks".to_string(),
            StartupState::Ready(_) | StartupState::Failed(_) => {
                "blocked by startup diagnostics".to_string()
            }
            StartupState::Idle => "not requested yet".to_string(),
        };
    }

    match &app.session_state {
        SessionState::Idle => "ready to load".to_string(),
        SessionState::Loading => "loading from codex app-server".to_string(),
        SessionState::Failed(_) => "load failed".to_string(),
        SessionState::Ready(recent_sessions) => format!("{} loaded", recent_sessions.items.len()),
    }
}

pub(super) fn build_session_overlay_content(
    app: &NativeTuiApp,
) -> (OverlayListView, Vec<Line<'static>>) {
    let current_workspace_directory = app.current_workspace_directory();

    match &app.session_state {
        SessionState::Idle => (
            OverlayListView {
                message_lines: Some(vec![Line::from(if app.can_open_session_list() {
                    "session list has not loaded yet"
                } else {
                    "recent sessions unlock after startup diagnostics pass"
                })]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![Line::from(if app.can_open_session_list() {
                "session details are not available yet"
            } else {
                "startup diagnostics must pass before recent-session detail is available"
            })],
        ),
        SessionState::Loading => (
            OverlayListView {
                message_lines: Some(vec![Line::from(
                    "loading recent sessions from codex app-server",
                )]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![Line::from("waiting for session list response")],
        ),
        SessionState::Failed(message) => (
            OverlayListView {
                message_lines: Some(vec![Line::from(message.clone())]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![Line::from(message.clone())],
        ),
        SessionState::Ready(recent_sessions) => {
            let browser_view = build_session_browser_view(
                recent_sessions,
                app.session_overlay_ui_state.browser_state(),
                Some(current_workspace_directory.as_str()),
                app.session_overlay_ui_state.selected_session_id(),
                app.selected_session_index,
            );
            if recent_sessions.items.is_empty() {
                let mut lines = build_session_browser_summary_lines(app, &browser_view);
                lines.push(Line::from(""));
                lines.push(Line::from(
                    "codex app-server has not returned any recent sessions yet",
                ));
                lines.push(Line::from(
                    "Start a new draft with n, then reload the browser with r.",
                ));
                return (
                    OverlayListView {
                        message_lines: Some(vec![Line::from(
                            "no recent sessions have been recorded yet",
                        )]),
                        items: Vec::new(),
                        selected_index: None,
                    },
                    lines,
                );
            }

            if browser_view.visible_sessions.is_empty() {
                let search_query = app
                    .session_overlay_ui_state
                    .browser_state()
                    .search_query
                    .as_str();
                let mut lines = build_session_browser_summary_lines(app, &browser_view);
                lines.push(Line::from(""));
                lines.push(Line::from(build_session_empty_detail_line(
                    &browser_view,
                    search_query,
                )));
                lines.push(Line::from(build_session_empty_hint_line(&browser_view)));
                return (
                    OverlayListView {
                        message_lines: Some(vec![Line::from(build_session_empty_message(
                            &browser_view,
                            search_query,
                        ))]),
                        items: Vec::new(),
                        selected_index: None,
                    },
                    lines,
                );
            }

            let Some(selected_session) = browser_view.selected_session() else {
                let search_query = app
                    .session_overlay_ui_state
                    .browser_state()
                    .search_query
                    .as_str();
                let mut lines = build_session_browser_summary_lines(app, &browser_view);
                lines.push(Line::from(""));
                lines.push(Line::from(build_session_empty_detail_line(
                    &browser_view,
                    search_query,
                )));
                return (
                    OverlayListView {
                        message_lines: None,
                        items: browser_view
                            .visible_sessions
                            .iter()
                            .copied()
                            .map(build_session_list_entry)
                            .collect(),
                        selected_index: None,
                    },
                    lines,
                );
            };

            let mut lines = vec![
                Line::from(format!("id: {}", selected_session.id)),
                Line::from(format!("updated: {}", selected_session.updated_at_label())),
                Line::from(format!("workspace: {}", selected_session.cwd)),
                Line::from(format!("source: {}", selected_session.source)),
                Line::from(format!(
                    "model provider: {}",
                    selected_session.model_provider
                )),
                Line::from(format!("status: {}", selected_session.status_type)),
            ];

            if let Some(branch) = &selected_session.git_branch {
                lines.push(Line::from(format!("git branch: {branch}")));
            }

            lines.extend(build_session_browser_summary_lines(app, &browser_view));

            if recent_sessions.next_cursor.is_some() {
                lines.push(Line::from("more threads are available in the next cursor"));
            }

            lines.push(Line::from(""));
            lines.push(Line::from("preview"));
            lines.push(Line::from(selected_session.preview_block()));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("path: {}", selected_session.path)));
            (
                OverlayListView {
                    message_lines: None,
                    items: browser_view
                        .visible_sessions
                        .iter()
                        .copied()
                        .map(build_session_list_entry)
                        .collect(),
                    selected_index: browser_view.selected_index,
                },
                lines,
            )
        }
    }
}

fn build_session_browser_summary_lines(
    app: &NativeTuiApp,
    browser_view: &SessionBrowserView<'_>,
) -> Vec<Line<'static>> {
    let active_filter_option = browser_view.projection.active_project_filter_option();
    let filter_label = active_filter_option
        .map(|option| option.label.as_str())
        .unwrap_or("all projects");
    let filter_session_count = active_filter_option
        .map(|option| option.session_count)
        .unwrap_or(browser_view.projection.filtered_session_count);
    let browser_query = if app.session_overlay_ui_state.is_search_query_editing() {
        app.session_overlay_ui_state.search_query_editor_buffer()
    } else {
        &app.session_overlay_ui_state.browser_state().search_query
    };
    let mut lines = vec![
        Line::from(format!(
            "{}: {}",
            if app.session_overlay_ui_state.is_search_query_editing() {
                "query edit"
            } else {
                "query"
            },
            format_session_query_label(browser_query)
        )),
        Line::from(format_session_filter_line(
            &browser_view.projection,
            &filter_label,
            filter_session_count,
        )),
        Line::from(build_session_project_context_line(
            &browser_view.projection,
            &app.current_workspace_directory(),
        )),
        Line::from(format_session_browser_line(
            &browser_view.projection,
            &filter_label,
        )),
    ];

    if app.session_overlay_ui_state.is_search_query_editing() {
        lines.push(Line::from(
            "Enter applies the query. Esc keeps the saved browser state.",
        ));
    }

    lines
}

pub(super) fn build_session_key_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    if app.session_overlay_ui_state.is_search_query_editing() {
        return vec![
            Line::from("Type the session query directly. Spaces match multiple tokens."),
            Line::from("Enter: apply query    Esc/Ctrl+C: cancel    Backspace: delete"),
        ];
    }

    vec![
        Line::from("/: query    c: clear    Tab/BackTab: filter    [ ] or PgUp/PgDn: page"),
        Line::from("Up/Down or Home/End or g/G: move    Enter: open    Esc/Ctrl+C: close"),
        Line::from("n: draft    r: reload    Ctrl+d: diagnostics"),
    ]
}

fn format_session_query_label(search_query: &str) -> &str {
    if search_query.is_empty() {
        "(all text)"
    } else {
        search_query
    }
}

fn build_session_project_context_line(
    projection: &SessionBrowserProjection,
    current_workspace_directory: &str,
) -> String {
    let current_workspace_label = format!("current workspace ({current_workspace_directory})");
    let Some(active_filter_option) = projection.active_project_filter_option() else {
        return format!("context: {current_workspace_label}");
    };

    if active_filter_option.is_current_workspace {
        return format!("context: showing only {current_workspace_label}");
    }

    match projection.current_workspace_session_count {
        0 => format!("context: {current_workspace_label} has no recent sessions"),
        1 => format!("context: {current_workspace_label} has 1 recent session"),
        count => format!("context: {current_workspace_label} has {count} recent sessions"),
    }
}

fn build_session_empty_message(
    browser_view: &SessionBrowserView<'_>,
    search_query: &str,
) -> String {
    format_session_empty_message(
        &browser_view.projection.active_project_filter,
        search_query,
        browser_view
            .projection
            .active_project_filter_option()
            .map(|option| option.label.as_str()),
        browser_view
            .projection
            .active_project_filter_option()
            .is_some_and(|option| option.is_current_workspace),
        browser_view.projection.filtered_session_count,
    )
}

fn build_session_empty_detail_line(
    browser_view: &SessionBrowserView<'_>,
    search_query: &str,
) -> String {
    format_session_empty_detail_line(
        &browser_view.projection.active_project_filter,
        search_query,
        browser_view
            .projection
            .active_project_filter_option()
            .map(|option| option.label.as_str()),
        browser_view
            .projection
            .active_project_filter_option()
            .is_some_and(|option| option.is_current_workspace),
        browser_view.projection.filtered_session_count,
    )
}

fn build_session_empty_hint_line(browser_view: &SessionBrowserView<'_>) -> String {
    if browser_view.projection.filtered_session_count == 0 {
        "Press c to clear the browser, Tab/BackTab to cycle filters, or r to reload.".to_string()
    } else {
        "Use Up/Down or Home/End to pick another session, or reload with r.".to_string()
    }
}

fn format_session_empty_message(
    active_project_filter: &SessionProjectFilter,
    search_query: &str,
    active_filter_label: Option<&str>,
    is_current_workspace_filter: bool,
    filtered_session_count: usize,
) -> String {
    if filtered_session_count > 0 {
        return "the current page has no visible session selection".to_string();
    }

    match active_project_filter {
        SessionProjectFilter::AllProjects if search_query.is_empty() => {
            "no sessions match the current browser state".to_string()
        }
        SessionProjectFilter::AllProjects => {
            format!("no sessions match query \"{search_query}\"")
        }
        SessionProjectFilter::RecentProject { .. }
            if is_current_workspace_filter && search_query.is_empty() =>
        {
            "no current-workspace sessions match the current browser state".to_string()
        }
        SessionProjectFilter::RecentProject { .. } if is_current_workspace_filter => {
            format!("no current-workspace sessions match query \"{search_query}\"")
        }
        SessionProjectFilter::RecentProject { .. } if search_query.is_empty() => format!(
            "no sessions in {} match the current browser state",
            active_filter_label.unwrap_or("the selected project")
        ),
        SessionProjectFilter::RecentProject { .. } => format!(
            "no sessions in {} match query \"{}\"",
            active_filter_label.unwrap_or("the selected project"),
            search_query,
        ),
    }
}

fn format_session_empty_detail_line(
    active_project_filter: &SessionProjectFilter,
    search_query: &str,
    active_filter_label: Option<&str>,
    is_current_workspace_filter: bool,
    filtered_session_count: usize,
) -> String {
    if filtered_session_count > 0 {
        return "no session detail is available for the current browser page".to_string();
    }

    match active_project_filter {
        SessionProjectFilter::AllProjects if search_query.is_empty() => {
            "no session detail is available for the current browser state".to_string()
        }
        SessionProjectFilter::AllProjects => {
            format!("no session detail is available for query \"{search_query}\"")
        }
        SessionProjectFilter::RecentProject { .. }
            if is_current_workspace_filter && search_query.is_empty() =>
        {
            "no session detail is available for the current workspace filter".to_string()
        }
        SessionProjectFilter::RecentProject { .. } if is_current_workspace_filter => {
            format!("no current-workspace session detail is available for query \"{search_query}\"")
        }
        SessionProjectFilter::RecentProject { .. } if search_query.is_empty() => format!(
            "no session detail is available for {}",
            active_filter_label.unwrap_or("the selected project filter")
        ),
        SessionProjectFilter::RecentProject { .. } => format!(
            "no session detail is available for {} and query \"{}\"",
            active_filter_label.unwrap_or("the selected project filter"),
            search_query,
        ),
    }
}

pub(super) fn build_session_warning_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.session_state {
        SessionState::Ready(recent_sessions) if !recent_sessions.warnings.is_empty() => {
            recent_sessions
                .warnings
                .iter()
                .cloned()
                .map(Line::from)
                .collect::<Vec<_>>()
        }
        SessionState::Failed(message) => vec![Line::from(message.clone())],
        SessionState::Loading => vec![Line::from("waiting for app-server response")],
        SessionState::Idle if !app.can_open_session_list() => vec![Line::from(
            "recent sessions remain unavailable until startup diagnostics succeed",
        )],
        _ => vec![Line::from("no warnings")],
    }
}

fn build_session_list_entry(session: &SessionSummary) -> OverlayListEntryView {
    OverlayListEntryView {
        lines: vec![
            Line::from(format!(
                "{}  {}  {}",
                session.short_id(),
                session.updated_at_label(),
                session.workspace_label(),
            )),
            Line::from(format!(
                "{} [{} / {}]",
                session.title(),
                session.source,
                session.model_provider,
            )),
        ],
    }
}

fn format_session_filter_line(
    projection: &SessionBrowserProjection,
    filter_label: &str,
    filter_session_count: usize,
) -> String {
    let session_suffix = plural_suffix(filter_session_count);
    match &projection.active_project_filter {
        SessionProjectFilter::AllProjects => {
            let workspace_count = projection.project_filter_options.len().saturating_sub(1);
            let workspace_suffix = plural_suffix(workspace_count);
            if workspace_count > 1 {
                format!(
                    "filter: {filter_label} ({filter_session_count} recent session{session_suffix} across {workspace_count} workspace{workspace_suffix})"
                )
            } else {
                format!(
                    "filter: {filter_label} ({filter_session_count} recent session{session_suffix})"
                )
            }
        }
        SessionProjectFilter::RecentProject { .. } => {
            format!(
                "filter: {filter_label} ({filter_session_count} recent session{session_suffix})"
            )
        }
    }
}

fn format_session_browser_line(
    projection: &SessionBrowserProjection,
    filter_label: &str,
) -> String {
    if projection.total_session_count == 0 {
        return "browser: no recent sessions loaded".to_string();
    }

    if projection.filtered_session_count == 0 {
        return match &projection.active_project_filter {
            SessionProjectFilter::AllProjects => {
                format!(
                    "browser: no matches in {} recent session{}",
                    projection.project_filtered_session_count,
                    plural_suffix(projection.project_filtered_session_count)
                )
            }
            SessionProjectFilter::RecentProject { .. } => format!(
                "browser: no matches in {filter_label} across {} recent session{}",
                projection.project_filtered_session_count,
                plural_suffix(projection.project_filtered_session_count)
            ),
        };
    }

    let (visible_start, visible_end) = projection
        .visible_session_range
        .expect("visible range should exist when filtered sessions are visible");
    format!(
        "browser: page {} of {} | showing {}-{} of {} matches",
        projection.page_index + 1,
        projection.total_pages.max(1),
        visible_start,
        visible_end,
        projection.filtered_session_count,
    )
}

fn plural_suffix(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_context_line_surfaces_current_workspace_session_count() {
        let projection = sample_projection(
            SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/docs".to_string(),
            },
            vec![
                SessionProjectFilterOption {
                    filter: SessionProjectFilter::AllProjects,
                    label: "all projects".to_string(),
                    session_count: 5,
                    is_current_workspace: false,
                },
                SessionProjectFilterOption {
                    filter: SessionProjectFilter::RecentProject {
                        workspace_directory: "/tmp/docs".to_string(),
                    },
                    label: "/tmp/docs".to_string(),
                    session_count: 3,
                    is_current_workspace: false,
                },
                SessionProjectFilterOption {
                    filter: SessionProjectFilter::RecentProject {
                        workspace_directory: "/tmp/root".to_string(),
                    },
                    label: "current workspace (/tmp/root)".to_string(),
                    session_count: 2,
                    is_current_workspace: true,
                },
            ],
            2,
            3,
        );

        let line = build_session_project_context_line(&projection, "/tmp/root");

        assert_eq!(
            line,
            "context: current workspace (/tmp/root) has 2 recent sessions"
        );
    }

    #[test]
    fn empty_state_messages_include_query_for_current_workspace_filter() {
        let message = format_session_empty_message(
            &SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/root".to_string(),
            },
            "release",
            Some("current workspace (/tmp/root)"),
            true,
            0,
        );
        let detail = format_session_empty_detail_line(
            &SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/root".to_string(),
            },
            "release",
            Some("current workspace (/tmp/root)"),
            true,
            0,
        );

        assert_eq!(
            message,
            "no current-workspace sessions match query \"release\""
        );
        assert_eq!(
            detail,
            "no current-workspace session detail is available for query \"release\""
        );
    }

    fn sample_projection(
        active_project_filter: SessionProjectFilter,
        project_filter_options: Vec<SessionProjectFilterOption>,
        current_workspace_session_count: usize,
        filtered_session_count: usize,
    ) -> SessionBrowserProjection {
        let total_session_count = project_filter_options
            .first()
            .map(|option| option.session_count)
            .unwrap_or(filtered_session_count);
        let project_filtered_session_count = project_filter_options
            .iter()
            .find(|option| option.filter == active_project_filter)
            .map(|option| option.session_count)
            .unwrap_or(filtered_session_count);
        SessionBrowserProjection {
            active_project_filter,
            project_filter_options,
            current_workspace_session_count,
            total_session_count,
            project_filtered_session_count,
            filtered_session_count,
            page_index: 0,
            total_pages: 1,
            visible_session_range: Some((1, 1)),
            page_session_indexes: vec![0],
        }
    }
}
