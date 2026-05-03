use super::capability_copy::{
    session_catalog_empty_action_hint_line, session_catalog_empty_message_line,
    session_catalog_empty_provider_line, session_catalog_loading_message,
    session_catalog_not_loaded_detail_line, session_catalog_not_loaded_message,
    session_catalog_partial_detail_line, session_catalog_partial_message,
    session_catalog_tier_line, session_catalog_unsupported_detail_line,
    session_catalog_unsupported_message, session_catalog_waiting_detail_line,
    session_catalog_warning_blocked_line, session_catalog_warning_waiting_line,
};
use super::overlays::{OverlayListEntryView, OverlayListView};
use super::{AkraTheme, NativeTuiApp};
use crate::adapter::inbound::tui::shell_chrome::SessionState;
use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogTier};
use crate::domain::session_browser::SessionProjectFilterOption;
use crate::domain::session_browser::{
    SessionBrowserPage, SessionBrowserProjection, SessionProjectFilter, build_session_browser_page,
};
use crate::domain::session_summary::SessionSummary;
use ratatui::text::Line;
#[path = "session_browser/empty_state.rs"]
mod empty_state;
use self::empty_state::{
    build_session_empty_detail_line, build_session_empty_hint_line, build_session_empty_message,
    build_session_project_context_line,
};

// Session browser rendering has to preserve a single overlay contract across
// four catalog states: unavailable, loading, failed, and queryable. The left
// pane is always an OverlayListView, while the right pane explains either the
// selected thread or why no thread can be selected.
pub(super) fn build_session_overlay_content(
    app: &NativeTuiApp,
) -> (OverlayListView, Vec<Line<'static>>) {
    let current_workspace_directory = app.current_workspace_directory();
    match &app.session_state {
        SessionState::Idle => (
            OverlayListView {
                message_lines: Some(vec![Line::from(session_catalog_not_loaded_message(
                    app.can_open_session_list(),
                ))]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![Line::from(session_catalog_not_loaded_detail_line(
                app.can_open_session_list(),
            ))],
        ),
        SessionState::Loading => (
            OverlayListView {
                message_lines: Some(vec![Line::from(session_catalog_loading_message())]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![Line::from(session_catalog_waiting_detail_line())],
        ),
        SessionState::Failed(message) => (
            OverlayListView {
                message_lines: Some(vec![Line::from(message.clone())]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![Line::from(message.clone())],
        ),
        SessionState::Ready(catalog) => {
            let Some(recent_sessions) = catalog.recent_sessions() else {
                return build_non_queryable_session_catalog_content(catalog);
            };

            // Domain projection owns filtering, paging, and selection repair.
            // Presentation code only maps that projection into list rows and
            // detail copy so key handling and rendering stay in sync.
            let browser_page = build_session_browser_page(
                recent_sessions,
                app.session_overlay_ui_state.browser_state(),
                Some(current_workspace_directory.as_str()),
                app.session_overlay_ui_state.selected_session_id(),
                app.selected_session_index,
            );

            // A queryable provider can still return an empty catalog. That is
            // different from "no matches": it should point operators at the
            // provider/capture action instead of query or filter controls.
            if recent_sessions.items.is_empty() {
                let mut lines =
                    build_session_browser_summary_lines(app, &browser_page, catalog.tier());
                lines.push(Line::from(""));
                lines.push(Line::from(session_catalog_empty_provider_line()));
                lines.push(Line::from(session_catalog_empty_action_hint_line()));
                return (
                    OverlayListView {
                        message_lines: Some(vec![Line::from(session_catalog_empty_message_line())]),
                        items: Vec::new(),
                        selected_index: None,
                    },
                    lines,
                );
            }

            // Filters and search can hide every loaded session. Keep summary
            // context visible here so users can understand whether the empty
            // result came from text search, project filtering, or both.
            if browser_page.visible_sessions.is_empty() {
                let search_query = app
                    .session_overlay_ui_state
                    .browser_state()
                    .search_query
                    .as_str();
                let mut lines =
                    build_session_browser_summary_lines(app, &browser_page, catalog.tier());
                lines.push(Line::from(""));
                lines.push(Line::from(build_session_empty_detail_line(
                    &browser_page,
                    search_query,
                )));
                lines.push(Line::from(build_session_empty_hint_line(&browser_page)));
                return (
                    OverlayListView {
                        message_lines: Some(vec![Line::from(build_session_empty_message(
                            &browser_page,
                            search_query,
                        ))]),
                        items: Vec::new(),
                        selected_index: None,
                    },
                    lines,
                );
            }

            // Selection can be absent after state restoration or a page/filter
            // transition even while rows remain visible. Rendering rows without
            // detail keeps the overlay usable until the controller repairs the
            // selected id/index on the next input.
            let Some(selected_session) = browser_page.selected_session() else {
                let search_query = app
                    .session_overlay_ui_state
                    .browser_state()
                    .search_query
                    .as_str();
                let mut lines =
                    build_session_browser_summary_lines(app, &browser_page, catalog.tier());
                lines.push(Line::from(""));
                lines.push(Line::from(build_session_empty_detail_line(
                    &browser_page,
                    search_query,
                )));
                return (
                    OverlayListView {
                        message_lines: None,
                        items: browser_page
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

            // The detail pane favors durable reopen/debug identifiers before
            // showing preview text. This mirrors the native-first app-server
            // workflow where a session may be opened by id, path, workspace, or
            // source/provider clues.
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

            lines.extend(build_session_browser_summary_lines(
                app,
                &browser_page,
                catalog.tier(),
            ));
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
                    items: browser_page
                        .visible_sessions
                        .iter()
                        .copied()
                        .map(build_session_list_entry)
                        .collect(),
                    selected_index: browser_page.selected_index,
                },
                lines,
            )
        }
    }
}

// Summary lines are shared by normal, empty, and selection-repair states. They
// intentionally derive labels from the active projection so the right pane,
// footer keys, and list rows describe the same search/filter/page state.
fn build_session_browser_summary_lines(
    app: &NativeTuiApp,
    browser_page: &SessionBrowserPage<'_>,
    catalog_tier: SessionCatalogTier,
) -> Vec<Line<'static>> {
    let active_filter_option = browser_page.projection.active_project_filter_option();
    let filter_label = active_filter_option
        .map(session_project_filter_option_label)
        .unwrap_or_else(|| session_project_filter_label(&SessionProjectFilter::AllProjects));
    let filter_session_count = active_filter_option
        .map(|option| option.session_count)
        .unwrap_or(browser_page.projection.filtered_session_count);
    let browser_query = if app.session_overlay_ui_state.is_search_query_editing() {
        app.session_overlay_ui_state.search_query_editor_buffer()
    } else {
        &app.session_overlay_ui_state.browser_state().search_query
    };
    let mut lines = vec![
        Line::from(session_catalog_tier_line(catalog_tier)),
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
            &browser_page.projection,
            filter_label.as_str(),
            filter_session_count,
        )),
        Line::from(build_session_project_context_line(
            &browser_page.projection,
            &app.current_workspace_directory(),
        )),
        Line::from(format_session_browser_line(
            &browser_page.projection,
            filter_label.as_str(),
        )),
    ];
    if app.session_overlay_ui_state.is_search_query_editing() {
        lines.push(Line::from(
            "Enter applies the query. Esc keeps the saved browser state.",
        ));
    }

    lines
}

// Key copy is stateful because the same overlay can be a search editor,
// diagnostic surface for unsupported catalogs, or full browser. The shortcuts
// listed here must match what the shell controller accepts in each mode.
pub(super) fn build_session_key_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    if app.session_overlay_ui_state.is_search_query_editing() {
        return vec![
            AkraTheme::key_line("Type the session query directly. Spaces match multiple tokens."),
            AkraTheme::key_line("Enter: apply query    Esc/Ctrl+C: cancel    Backspace: delete"),
        ];
    }
    if !app.session_browser_available() {
        return vec![
            AkraTheme::key_line(
                "n: draft    r: reload    Ctrl+d: diagnostics    Esc/Ctrl+C: close",
            ),
            AkraTheme::key_line("Recent-session navigation requires a queryable catalog surface."),
        ];
    }

    vec![
        AkraTheme::key_line(
            "/: query    c: clear    Tab/BackTab: filter    [ ] or PgUp/PgDn: page",
        ),
        AkraTheme::key_line("Up/Down or Home/End or g/G: move    Enter: open    Esc/Ctrl+C: close"),
        AkraTheme::key_line("n: draft    r: reload    Ctrl+d: diagnostics"),
    ]
}

fn format_session_query_label(search_query: &str) -> &str {
    if search_query.is_empty() {
        "(all text)"
    } else {
        search_query
    }
}

// Warning lines are separated from the main overlay content so shell chrome can
// keep surfacing provider capability failures even when the browser body is
// showing loading, diagnostics, or an otherwise empty list.
pub(super) fn build_session_warning_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.session_state {
        SessionState::Ready(catalog) if !catalog.warnings().is_empty() => catalog
            .warnings()
            .iter()
            .cloned()
            .map(Line::from)
            .collect::<Vec<_>>(),
        SessionState::Failed(message) => vec![Line::from(message.clone())],
        SessionState::Loading => vec![Line::from(session_catalog_warning_waiting_line())],
        SessionState::Idle if !app.can_open_session_list() => {
            vec![Line::from(session_catalog_warning_blocked_line())]
        }
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

// Unsupported and partial catalogs still carry useful capability metadata, but
// they cannot be searched or paged. Render them as diagnostics instead of
// forcing them through the normal browser projection.
fn build_non_queryable_session_catalog_content(
    catalog: &SessionCatalog,
) -> (OverlayListView, Vec<Line<'static>>) {
    let mut lines = vec![Line::from(session_catalog_tier_line(catalog.tier()))];
    match catalog {
        SessionCatalog::Unsupported(status) => {
            lines.push(Line::from(session_catalog_unsupported_detail_line(
                status.tier,
            )));
            if !status.detail.is_empty() {
                lines.push(Line::from(format!("detail: {}", status.detail)));
            }
            (
                OverlayListView {
                    message_lines: Some(vec![Line::from(session_catalog_unsupported_message(
                        status.tier,
                    ))]),
                    items: Vec::new(),
                    selected_index: None,
                },
                lines,
            )
        }
        SessionCatalog::Partial(status) => {
            lines.push(Line::from(session_catalog_partial_detail_line(
                status.detail.as_str(),
            )));
            (
                OverlayListView {
                    message_lines: Some(vec![Line::from(session_catalog_partial_message(
                        status.tier,
                    ))]),
                    items: Vec::new(),
                    selected_index: None,
                },
                lines,
            )
        }
        SessionCatalog::Ready { .. } => unreachable!("ready catalogs should render a browser"),
    }
}

// Filter copy distinguishes "all projects" from a specific workspace because
// the all-projects tab can cover many workspaces. Counts come from the
// projection, not the visible page, so the summary remains stable while paging.
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

fn session_project_filter_option_label(option: &SessionProjectFilterOption) -> String {
    if option.is_current_workspace {
        return match &option.filter {
            SessionProjectFilter::RecentProject {
                workspace_directory,
            } => format!("current workspace ({workspace_directory})"),
            SessionProjectFilter::AllProjects => "current workspace".to_string(),
        };
    }

    session_project_filter_label(&option.filter)
}

fn session_project_filter_label(filter: &SessionProjectFilter) -> String {
    match filter {
        SessionProjectFilter::AllProjects => "all projects".to_string(),
        SessionProjectFilter::RecentProject {
            workspace_directory,
        } => workspace_directory.clone(),
    }
}

// Browser copy has three distinct zero states: no catalog rows, project/search
// filters eliminated every row, and a visible page. Keeping these cases separate
// prevents the overlay from implying pagination exists when there are no rows.
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
    use super::format_session_query_label;
    #[test]
    fn query_label_uses_all_text_placeholder_for_empty_query() {
        assert_eq!(format_session_query_label(""), "(all text)");
        assert_eq!(format_session_query_label("release"), "release");
    }
}
