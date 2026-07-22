use super::super::{
    SessionBrowserScreenModel, SessionOverlayCatalogScreenModel, SessionOverlayScreenModel,
};
use super::AkraTheme;
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
use crate::domain::recent_sessions::SessionCatalogStatus;
use crate::domain::session_browser::SessionProjectFilterOption;
use crate::domain::session_browser::{SessionBrowserProjection, SessionProjectFilter};
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
    model: &SessionOverlayScreenModel,
) -> (OverlayListView, Vec<Line<'static>>) {
    match &model.catalog {
        SessionOverlayCatalogScreenModel::Idle => (
            OverlayListView {
                message_lines: Some(vec![Line::from(session_catalog_not_loaded_message(
                    model.can_open_session_list,
                ))]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![Line::from(session_catalog_not_loaded_detail_line(
                model.can_open_session_list,
            ))],
        ),
        SessionOverlayCatalogScreenModel::Loading => (
            OverlayListView {
                message_lines: Some(vec![Line::from(session_catalog_loading_message())]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![Line::from(session_catalog_waiting_detail_line())],
        ),
        SessionOverlayCatalogScreenModel::Failed(message) => (
            OverlayListView {
                message_lines: Some(vec![Line::from(message.clone())]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![Line::from(message.clone())],
        ),
        SessionOverlayCatalogScreenModel::Unsupported(status) => {
            build_non_queryable_session_catalog_content(status, false)
        }
        SessionOverlayCatalogScreenModel::Partial(status) => {
            build_non_queryable_session_catalog_content(status, true)
        }
        SessionOverlayCatalogScreenModel::Queryable(browser) => {
            let projection = &browser.projection;

            // A queryable provider can still return an empty catalog. That is
            // different from "no matches": it should point operators at the
            // provider/capture action instead of query or filter controls.
            if projection.total_session_count == 0 {
                let mut lines = build_session_browser_summary_lines(model, browser);
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
            if browser.visible_sessions.is_empty() {
                let search_query = model.committed_search_query.as_str();
                let mut lines = build_session_browser_summary_lines(model, browser);
                lines.push(Line::from(""));
                lines.push(Line::from(build_session_empty_detail_line(
                    projection,
                    search_query,
                )));
                lines.push(Line::from(build_session_empty_hint_line(projection)));
                return (
                    OverlayListView {
                        message_lines: Some(vec![Line::from(build_session_empty_message(
                            projection,
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
            let Some(selected_session) = browser.selected_session() else {
                let search_query = model.committed_search_query.as_str();
                let mut lines = build_session_browser_summary_lines(model, browser);
                lines.push(Line::from(""));
                lines.push(Line::from(build_session_empty_detail_line(
                    projection,
                    search_query,
                )));
                return (
                    OverlayListView {
                        message_lines: None,
                        items: browser
                            .visible_sessions
                            .iter()
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
                Line::from(format!("title: {}", selected_session.title())),
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
            if let Some(rename_editor) = model.rename_editor.as_ref()
                && rename_editor.thread_id == selected_session.id
            {
                lines.insert(
                    1,
                    Line::from(format!(
                        "{}: {}",
                        model.language.session_rename_label(),
                        rename_editor.buffer
                    )),
                );
                if let Some(feedback) = rename_editor.feedback.as_ref() {
                    lines.insert(2, Line::from(feedback.clone()));
                }
            }
            if let Some(branch) = &selected_session.git_branch {
                lines.push(Line::from(format!("git branch: {branch}")));
            }

            lines.extend(build_session_browser_summary_lines(model, browser));
            if browser.next_cursor_available {
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
                    items: browser
                        .visible_sessions
                        .iter()
                        .map(build_session_list_entry)
                        .collect(),
                    selected_index: browser.selected_index,
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
    model: &SessionOverlayScreenModel,
    browser: &SessionBrowserScreenModel,
) -> Vec<Line<'static>> {
    let active_filter_option = browser.projection.active_project_filter_option();
    let filter_label = active_filter_option
        .map(session_project_filter_option_label)
        .unwrap_or_else(|| session_project_filter_label(&SessionProjectFilter::AllProjects));
    let filter_session_count = active_filter_option
        .map(|option| option.session_count)
        .unwrap_or(browser.projection.filtered_session_count);
    let browser_query = model
        .search_query_editor
        .as_deref()
        .unwrap_or(model.committed_search_query.as_str());
    let mut lines = vec![
        Line::from(session_catalog_tier_line(browser.tier)),
        Line::from(format!(
            "{}: {}",
            if model.search_query_editor.is_some() {
                "query edit"
            } else {
                "query"
            },
            format_session_query_label(browser_query)
        )),
        Line::from(format_session_filter_line(
            &browser.projection,
            filter_label.as_str(),
            filter_session_count,
        )),
        Line::from(build_session_project_context_line(
            &browser.projection,
            &model.current_workspace_directory,
        )),
        Line::from(format_session_browser_line(
            &browser.projection,
            filter_label.as_str(),
        )),
    ];
    if model.search_query_editor.is_some() {
        lines.push(Line::from(
            "Enter applies the query. Esc keeps the saved browser state.",
        ));
    }

    lines
}

// Key copy is stateful because the same overlay can be a search editor,
// diagnostic surface for unsupported catalogs, or full browser. The shortcuts
// listed here must match what the shell controller accepts in each mode.
pub(super) fn build_session_key_lines(model: &SessionOverlayScreenModel) -> Vec<Line<'static>> {
    if let Some(rename_editor) = model.rename_editor.as_ref() {
        let key_lines = model
            .language
            .session_rename_key_lines(rename_editor.pending);
        return vec![
            AkraTheme::key_line(key_lines[0]),
            AkraTheme::key_line(key_lines[1]),
        ];
    }
    if model.search_query_editor.is_some() {
        return vec![
            AkraTheme::key_line("Type the session query directly. Spaces match multiple tokens."),
            AkraTheme::key_line("Enter: apply query    Esc/Ctrl+C: cancel    Backspace: delete"),
        ];
    }
    if !model.browser_available() {
        return vec![
            AkraTheme::key_line(
                "n: draft    r: reload    Ctrl+d: diagnostics    Esc/Ctrl+C: close",
            ),
            AkraTheme::key_line("Recent-session navigation requires a queryable catalog surface."),
        ];
    }

    vec![
        AkraTheme::key_line(
            "/: query    c: clear    Tab/Shift+Tab: filter    [ ] or PgUp/PgDn: page",
        ),
        AkraTheme::key_line("Up/Down or Home/End or g/G: move    Enter: open    Esc/Ctrl+C: close"),
        AkraTheme::key_line("e: rename    n: draft    r: reload    Ctrl+d: diagnostics"),
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
pub(super) fn build_session_warning_lines(model: &SessionOverlayScreenModel) -> Vec<Line<'static>> {
    match &model.catalog {
        SessionOverlayCatalogScreenModel::Unsupported(status)
        | SessionOverlayCatalogScreenModel::Partial(status)
            if !status.warnings.is_empty() =>
        {
            status
                .warnings
                .iter()
                .cloned()
                .map(Line::from)
                .collect::<Vec<_>>()
        }
        SessionOverlayCatalogScreenModel::Queryable(browser) if !browser.warnings.is_empty() => {
            browser
                .warnings
                .iter()
                .cloned()
                .map(Line::from)
                .collect::<Vec<_>>()
        }
        SessionOverlayCatalogScreenModel::Failed(message) => vec![Line::from(message.clone())],
        SessionOverlayCatalogScreenModel::Loading => {
            vec![Line::from(session_catalog_warning_waiting_line())]
        }
        SessionOverlayCatalogScreenModel::Idle if !model.can_open_session_list => {
            vec![Line::from(session_catalog_warning_blocked_line())]
        }
        _ => vec![Line::from("no warnings")],
    }
}

fn build_session_list_entry(session: &SessionSummary) -> OverlayListEntryView {
    OverlayListEntryView {
        lines: vec![
            Line::from(session.title()),
            Line::from(format!(
                "{}  {}  {}",
                session.updated_at_label(),
                session.workspace_label(),
                session.short_id(),
            )),
        ],
    }
}

// Unsupported and partial catalogs still carry useful capability metadata, but
// they cannot be searched or paged. Render them as diagnostics instead of
// forcing them through the normal browser projection.
fn build_non_queryable_session_catalog_content(
    status: &SessionCatalogStatus,
    partial: bool,
) -> (OverlayListView, Vec<Line<'static>>) {
    let mut lines = vec![Line::from(session_catalog_tier_line(status.tier))];
    if partial {
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
    } else {
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
    use super::*;
    use crate::adapter::inbound::tui::app::NativeTuiApp;
    use crate::adapter::inbound::tui::app::language::TuiLanguage;
    use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;
    use crate::adapter::inbound::tui::shell_chrome::{SessionState, ShellOverlay};
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogTier};

    fn session(id: &str, title: &str, cwd: &str) -> SessionSummary {
        SessionSummary {
            id: id.to_string(),
            name: Some(title.to_string()),
            preview: format!("{title} preview\nsecond line"),
            cwd: cwd.to_string(),
            source: "native".to_string(),
            model_provider: "openai".to_string(),
            updated_at_epoch: 1_700_000_000,
            status_type: "ready".to_string(),
            path: format!("{cwd}/{id}.json"),
            git_branch: Some("feature/session-browser".to_string()),
        }
    }

    fn ready_catalog(
        items: Vec<SessionSummary>,
        warnings: Vec<String>,
        next_cursor: Option<String>,
    ) -> SessionCatalog {
        SessionCatalog::ready(
            SessionCatalogTier::ProviderBackedCatalog,
            RecentSessions {
                items,
                warnings,
                next_cursor,
            },
        )
    }

    fn projection(
        active_project_filter: SessionProjectFilter,
        project_filter_options: Vec<SessionProjectFilterOption>,
        project_filtered_session_count: usize,
        filtered_session_count: usize,
        visible_session_range: Option<(usize, usize)>,
    ) -> SessionBrowserProjection {
        let total_session_count = project_filter_options
            .first()
            .map(|option| option.session_count)
            .unwrap_or(0);

        SessionBrowserProjection {
            active_project_filter,
            project_filter_options,
            current_workspace_session_count: 0,
            total_session_count,
            project_filtered_session_count,
            filtered_session_count,
            page_index: 0,
            total_pages: usize::from(visible_session_range.is_some()),
            visible_session_range,
            page_session_indexes: Vec::new(),
        }
    }

    fn lines_text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn list_text(list_view: &OverlayListView) -> String {
        let mut lines = list_view
            .message_lines
            .as_ref()
            .map(|lines| lines_text(lines))
            .unwrap_or_default();
        for item in &list_view.items {
            if !lines.is_empty() {
                lines.push('\n');
            }
            lines.push_str(&lines_text(&item.lines));
        }
        lines
    }

    fn screen_model(app: &NativeTuiApp) -> SessionOverlayScreenModel {
        SessionOverlayScreenModel::capture(app)
    }

    #[test]
    fn query_label_uses_all_text_placeholder_for_empty_query() {
        assert_eq!(format_session_query_label(""), "(all text)");
        assert_eq!(format_session_query_label("release"), "release");
    }

    #[test]
    fn overlay_content_covers_non_ready_and_non_queryable_catalogs() {
        let mut app = test_native_tui_app();

        let model = screen_model(&app);
        let (list_view, detail_lines) = build_session_overlay_content(&model);
        assert!(list_text(&list_view).contains("recent sessions unlock"));
        assert!(lines_text(&detail_lines).contains("startup diagnostics"));
        assert!(lines_text(&build_session_warning_lines(&model)).contains("remain unavailable"));
        assert!(
            lines_text(&build_session_key_lines(&model)).contains("requires a queryable catalog")
        );

        app.session_state = SessionState::Loading;
        let model = screen_model(&app);
        let (list_view, detail_lines) = build_session_overlay_content(&model);
        assert!(list_text(&list_view).contains("loading recent sessions"));
        assert!(lines_text(&detail_lines).contains("waiting for session list response"));
        assert!(
            lines_text(&build_session_warning_lines(&model)).contains("waiting for app-server")
        );

        app.session_state = SessionState::Failed("catalog unavailable".to_string());
        let model = screen_model(&app);
        let (list_view, detail_lines) = build_session_overlay_content(&model);
        assert!(list_text(&list_view).contains("catalog unavailable"));
        assert!(lines_text(&detail_lines).contains("catalog unavailable"));
        assert!(lines_text(&build_session_warning_lines(&model)).contains("catalog unavailable"));

        app.session_state = SessionState::Ready(SessionCatalog::unsupported(
            SessionCatalogTier::AttachOnly,
            "provider disabled",
            vec!["unsupported warning".to_string()],
        ));
        let model = screen_model(&app);
        let (list_view, detail_lines) = build_session_overlay_content(&model);
        assert!(list_text(&list_view).contains("does not expose a recent-session catalog"));
        let detail_text = lines_text(&detail_lines);
        assert!(detail_text.contains("catalog tier: attach-only"));
        assert!(detail_text.contains("detail: provider disabled"));
        assert!(lines_text(&build_session_warning_lines(&model)).contains("unsupported warning"));

        app.session_state = SessionState::Ready(SessionCatalog::partial(
            SessionCatalogTier::HandleBasedReattach,
            "handle only",
            vec!["partial warning".to_string()],
        ));
        let model = screen_model(&app);
        let (list_view, detail_lines) = build_session_overlay_content(&model);
        assert!(
            list_text(&list_view).contains("handle-based reattach is only partially available")
        );
        assert!(lines_text(&detail_lines).contains("handle only"));
        assert!(lines_text(&build_session_warning_lines(&model)).contains("partial warning"));
    }

    #[test]
    fn ready_catalog_content_covers_empty_detail_search_and_query_editing() {
        let mut app = test_native_tui_app();
        app.shell_overlay = ShellOverlay::Sessions;
        app.session_state = SessionState::Ready(ready_catalog(Vec::new(), Vec::new(), None));

        let model = screen_model(&app);
        let (list_view, detail_lines) = build_session_overlay_content(&model);
        assert_eq!(list_view.selected_index, None);
        assert!(list_text(&list_view).contains("no recent sessions have been recorded yet"));
        let detail_text = lines_text(&detail_lines);
        assert!(detail_text.contains("catalog tier: provider-backed catalog"));
        assert!(detail_text.contains("codex app-server has not returned any recent sessions"));
        assert!(detail_text.contains("Start a new draft with n"));
        assert!(lines_text(&build_session_warning_lines(&model)).contains("no warnings"));

        app.session_state = SessionState::Ready(ready_catalog(
            vec![
                session("thread-alpha", "Alpha task", "/tmp/root"),
                session("thread-beta", "Beta task", "/tmp/other"),
            ],
            vec!["catalog warning".to_string()],
            Some("next-cursor".to_string()),
        ));
        let model = screen_model(&app);
        let (list_view, detail_lines) = build_session_overlay_content(&model);
        assert_eq!(list_view.items.len(), 2);
        assert_eq!(list_view.selected_index, Some(0));
        assert!(list_text(&list_view).contains("thread-a"));
        assert_eq!(list_view.items[0].lines[0].to_string(), "Alpha task");
        let detail_text = lines_text(&detail_lines);
        assert_eq!(detail_lines[0].to_string(), "title: Alpha task");
        assert!(detail_text.contains("id: thread-alpha"));
        assert!(detail_text.contains("updated:"));
        assert!(detail_text.contains("workspace: /tmp/root"));
        assert!(detail_text.contains("source: native"));
        assert!(detail_text.contains("model provider: openai"));
        assert!(detail_text.contains("status: ready"));
        assert!(detail_text.contains("git branch: feature/session-browser"));
        assert!(detail_text.contains("query: (all text)"));
        assert!(
            detail_text.contains("filter: all projects (2 recent sessions across 2 workspaces)")
        );
        assert!(detail_text.contains("more threads are available in the next cursor"));
        assert!(detail_text.contains("Alpha task preview"));
        assert!(detail_text.contains("path: /tmp/root/thread-alpha.json"));
        assert!(lines_text(&build_session_warning_lines(&model)).contains("catalog warning"));
        assert!(lines_text(&build_session_key_lines(&model)).contains("/: query"));
        assert!(lines_text(&build_session_key_lines(&model)).contains("e: rename"));

        app.session_overlay_ui_state
            .set_project_filter(SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/root".to_string(),
            });
        let model = screen_model(&app);
        let (_, detail_lines) = build_session_overlay_content(&model);
        let detail_text = lines_text(&detail_lines);
        assert!(detail_text.contains("filter: /tmp/root (1 recent session)"));
        assert!(detail_text.contains("context: current workspace ("));

        app.session_overlay_ui_state
            .set_search_query("does-not-exist");
        let model = screen_model(&app);
        let (list_view, detail_lines) = build_session_overlay_content(&model);
        assert_eq!(list_view.items.len(), 0);
        assert!(list_text(&list_view).contains("no sessions in /tmp/root match query"));
        let detail_text = lines_text(&detail_lines);
        assert!(detail_text.contains("query: does-not-exist"));
        assert!(detail_text.contains("no session detail is available for /tmp/root and query"));
        assert!(detail_text.contains("Press c to clear the browser"));

        app.session_overlay_ui_state.start_search_query_edit();
        app.session_overlay_ui_state
            .push_search_query_character('!');
        let model = screen_model(&app);
        let (_, detail_lines) = build_session_overlay_content(&model);
        let detail_text = lines_text(&detail_lines);
        assert!(detail_text.contains("query edit: does-not-exist!"));
        assert!(detail_text.contains("Enter applies the query"));
        assert!(lines_text(&build_session_key_lines(&model)).contains("Type the session query"));
    }

    #[test]
    fn helper_copy_covers_filter_labels_browser_lines_entries_and_pluralization() {
        let all_projects = SessionProjectFilterOption {
            filter: SessionProjectFilter::AllProjects,
            session_count: 3,
            is_current_workspace: false,
        };
        let current_workspace = SessionProjectFilterOption {
            filter: SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/root".to_string(),
            },
            session_count: 1,
            is_current_workspace: true,
        };
        let other_workspace = SessionProjectFilterOption {
            filter: SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/docs".to_string(),
            },
            session_count: 2,
            is_current_workspace: false,
        };

        assert_eq!(
            session_project_filter_option_label(&current_workspace),
            "current workspace (/tmp/root)"
        );
        assert_eq!(
            session_project_filter_option_label(&other_workspace),
            "/tmp/docs"
        );
        assert_eq!(
            session_project_filter_label(&SessionProjectFilter::AllProjects),
            "all projects"
        );

        let all_projection = projection(
            SessionProjectFilter::AllProjects,
            vec![
                all_projects.clone(),
                current_workspace.clone(),
                other_workspace.clone(),
            ],
            3,
            3,
            Some((1, 3)),
        );
        assert_eq!(
            format_session_filter_line(&all_projection, "all projects", 3),
            "filter: all projects (3 recent sessions across 2 workspaces)"
        );
        assert_eq!(
            format_session_browser_line(&all_projection, "all projects"),
            "browser: page 1 of 1 | showing 1-3 of 3 matches"
        );

        let single_workspace_projection = projection(
            SessionProjectFilter::AllProjects,
            vec![all_projects.clone(), current_workspace.clone()],
            1,
            1,
            Some((1, 1)),
        );
        assert_eq!(
            format_session_filter_line(&single_workspace_projection, "all projects", 1),
            "filter: all projects (1 recent session)"
        );

        let project_projection = projection(
            SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/docs".to_string(),
            },
            vec![all_projects.clone(), other_workspace.clone()],
            2,
            2,
            Some((1, 2)),
        );
        assert_eq!(
            format_session_filter_line(&project_projection, "/tmp/docs", 2),
            "filter: /tmp/docs (2 recent sessions)"
        );

        let empty_projection =
            projection(SessionProjectFilter::AllProjects, Vec::new(), 0, 0, None);
        assert_eq!(
            format_session_browser_line(&empty_projection, "all projects"),
            "browser: no recent sessions loaded"
        );

        let no_all_matches = projection(
            SessionProjectFilter::AllProjects,
            vec![all_projects.clone()],
            3,
            0,
            None,
        );
        assert_eq!(
            format_session_browser_line(&no_all_matches, "all projects"),
            "browser: no matches in 3 recent sessions"
        );

        let no_project_matches = projection(
            SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/docs".to_string(),
            },
            vec![all_projects, other_workspace],
            2,
            0,
            None,
        );
        assert_eq!(
            format_session_browser_line(&no_project_matches, "/tmp/docs"),
            "browser: no matches in /tmp/docs across 2 recent sessions"
        );

        let entry = build_session_list_entry(&session("thread-gamma", "Gamma task", "/tmp/root"));
        assert_eq!(entry.lines[0].to_string(), "Gamma task");
        assert!(entry.lines[1].to_string().contains("thread-g"));
        assert!(!entry.lines[1].to_string().contains("native"));

        let long_title = "매우 긴 한국어 세션 제목 ".repeat(8);
        let entry = build_session_list_entry(&session("thread-korean", &long_title, "/tmp/root"));
        assert_eq!(entry.lines[0].to_string(), long_title);
        assert_eq!(plural_suffix(1), "");
        assert_eq!(plural_suffix(2), "s");
    }

    #[test]
    fn rename_editor_renders_exact_selected_thread_and_stateful_keys() {
        let mut app = test_native_tui_app();
        app.shell_overlay = ShellOverlay::Sessions;
        app.session_state = SessionState::Ready(ready_catalog(
            vec![session("thread-alpha", "Alpha task", "/tmp/root")],
            Vec::new(),
            None,
        ));
        app.session_overlay_ui_state
            .start_rename_edit("thread-alpha", "Alpha renamed");

        let model = screen_model(&app);
        let (_, detail_lines) = build_session_overlay_content(&model);
        assert_eq!(detail_lines[0].to_string(), "title: Alpha task");
        assert_eq!(detail_lines[1].to_string(), "rename: Alpha renamed");
        assert!(lines_text(&detail_lines).contains("id: thread-alpha"));
        assert!(lines_text(&build_session_key_lines(&model)).contains("Enter: rename"));

        let request = app
            .session_overlay_ui_state
            .prepare_rename_request(TuiLanguage::English)
            .expect("rename should enter pending state");
        assert!(app.session_overlay_ui_state.record_rename_admission(
            crate::core::app::SessionRenameCorrelation::new(1, request),
            TuiLanguage::English,
        ));
        let model = screen_model(&app);
        let keys = lines_text(&build_session_key_lines(&model));
        assert!(keys.contains("Rename pending"));
        assert!(keys.contains("duplicate submit"));

        app.tui_language = TuiLanguage::Korean;
        let model = screen_model(&app);
        let (_, detail_lines) = build_session_overlay_content(&model);
        assert_eq!(detail_lines[1].to_string(), "새 이름: Alpha renamed");
        let keys = lines_text(&build_session_key_lines(&model));
        assert!(keys.contains("이름 변경 확인 중"));
        assert!(keys.contains("중복 제출"));
    }

    #[test]
    fn captured_overlay_sections_stay_coherent_after_app_state_changes() {
        let mut app = test_native_tui_app();
        app.shell_overlay = ShellOverlay::Sessions;
        app.session_state = SessionState::Ready(ready_catalog(
            vec![session("thread-alpha", "Alpha task", "/tmp/root")],
            vec!["captured warning".to_string()],
            None,
        ));
        app.session_overlay_ui_state
            .start_rename_edit("thread-alpha", "Alpha renamed");

        let captured = screen_model(&app);

        app.session_state = SessionState::Failed("new catalog failure".to_string());
        app.session_overlay_ui_state.clear_browser_state();
        app.session_overlay_ui_state
            .cancel_rename_edit(TuiLanguage::Korean);
        app.tui_language = TuiLanguage::Korean;

        let (captured_list, captured_detail) = build_session_overlay_content(&captured);
        assert!(list_text(&captured_list).contains("Alpha task"));
        assert!(lines_text(&captured_detail).contains("rename: Alpha renamed"));
        assert!(lines_text(&build_session_warning_lines(&captured)).contains("captured warning"));
        assert!(lines_text(&build_session_key_lines(&captured)).contains("Enter: rename"));

        let fresh = screen_model(&app);
        let (fresh_list, fresh_detail) = build_session_overlay_content(&fresh);
        assert!(list_text(&fresh_list).contains("new catalog failure"));
        assert!(lines_text(&fresh_detail).contains("new catalog failure"));
        assert!(lines_text(&build_session_warning_lines(&fresh)).contains("new catalog failure"));
        assert!(!lines_text(&build_session_key_lines(&fresh)).contains("Enter: rename"));
    }
}
