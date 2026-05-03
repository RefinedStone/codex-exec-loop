use crate::domain::session_browser::{
    SessionBrowserPage, SessionBrowserProjection, SessionProjectFilter,
};

use super::session_project_filter_option_label;

/*
 * Empty-state copy bridges the pure session-browser projection and the overlay
 * list/detail panels. Projection counts tell us whether project filtering,
 * search filtering, or selection drift removed the visible session; this module
 * turns that into short operator-facing reasons and recovery hints.
 */
pub(super) fn build_session_project_context_line(
    projection: &SessionBrowserProjection,
    current_workspace_directory: &str,
) -> String {
    // Keep the current workspace visible even while browsing all projects or a
    // different recent project. It tells the operator whether switching filters
    // is likely to reveal sessions for the repository they are currently in.
    let current_workspace_label = format!("current workspace ({current_workspace_directory})");
    let Some(active_filter_option) = projection.active_project_filter_option() else {
        // During catalog refresh the active filter can be missing from options.
        // The adapter still knows cwd, so keep a useful context line instead of
        // letting a stale projection erase the header.
        return format!("context: {current_workspace_label}");
    };
    if active_filter_option.is_current_workspace {
        // When the active filter already is the current workspace, the list range
        // carries the count and the context line confirms scope.
        return format!("context: showing only {current_workspace_label}");
    }
    // Otherwise, use the separately preserved current-workspace count as a hint
    // that Tab/BackTab may reveal relevant sessions.
    match projection.current_workspace_session_count {
        0 => format!("context: {current_workspace_label} has no recent sessions"),
        1 => format!("context: {current_workspace_label} has 1 recent session"),
        count => format!("context: {current_workspace_label} has {count} recent sessions"),
    }
}

pub(super) fn build_session_empty_message(
    browser_page: &SessionBrowserPage<'_>,
    search_query: &str,
) -> String {
    // The list-panel message should stay compact. Resolve the human label from
    // the page projection here so the formatter only needs primitive context.
    let active_filter_label = browser_page
        .projection
        .active_project_filter_option()
        .map(session_project_filter_option_label);
    format_session_empty_message(
        &browser_page.projection.active_project_filter,
        search_query,
        active_filter_label.as_deref(),
        browser_page
            .projection
            .active_project_filter_option()
            .is_some_and(|option| option.is_current_workspace),
        browser_page.projection.filtered_session_count,
    )
}

pub(super) fn build_session_empty_detail_line(
    browser_page: &SessionBrowserPage<'_>,
    search_query: &str,
) -> String {
    // Detail copy mirrors list empty-state logic but names the missing detail.
    // This covers both true empty results and transient no-selection pages.
    let active_filter_label = browser_page
        .projection
        .active_project_filter_option()
        .map(session_project_filter_option_label);
    format_session_empty_detail_line(
        &browser_page.projection.active_project_filter,
        search_query,
        active_filter_label.as_deref(),
        browser_page
            .projection
            .active_project_filter_option()
            .is_some_and(|option| option.is_current_workspace),
        browser_page.projection.filtered_session_count,
    )
}

pub(super) fn build_session_empty_hint_line(browser_page: &SessionBrowserPage<'_>) -> String {
    // A zero filtered count points to filter/search recovery; rows without a
    // selected item point to navigation or reload.
    if browser_page.projection.filtered_session_count == 0 {
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
    // Nonzero filtered rows mean the result set exists but the current page has
    // no usable selected row. Keep that distinct from true "no matches" copy.
    if filtered_session_count > 0 {
        return "the current page has no visible session selection".to_string();
    }
    /*
     * Explain the active filter stack in the same order the user applies it:
     * all-projects, current workspace, then another recent project. The saved
     * query is echoed verbatim so the operator can see which search narrowed the
     * result set to zero.
     */
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
    // Keep the right pane aligned with the left pane's reason, but phrase it as
    // missing detail so the overlay does not imply the catalog itself failed.
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

#[cfg(test)]
mod tests {
    use super::{SessionBrowserProjection, format_session_empty_message};
    use super::{build_session_project_context_line, format_session_empty_detail_line};
    use crate::domain::session_browser::{SessionProjectFilter, SessionProjectFilterOption};

    #[test]
    fn project_context_line_surfaces_current_workspace_session_count() {
        // The active filter is a different project; the context line should still
        // expose the separately projected current-workspace count.
        let projection = sample_projection(
            SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/docs".to_string(),
            },
            vec![
                SessionProjectFilterOption {
                    filter: SessionProjectFilter::AllProjects,
                    session_count: 5,
                    is_current_workspace: false,
                },
                SessionProjectFilterOption {
                    filter: SessionProjectFilter::RecentProject {
                        workspace_directory: "/tmp/docs".to_string(),
                    },
                    session_count: 3,
                    is_current_workspace: false,
                },
                SessionProjectFilterOption {
                    filter: SessionProjectFilter::RecentProject {
                        workspace_directory: "/tmp/root".to_string(),
                    },
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
        // Current-workspace empty copy is not the same as generic recent-project
        // copy. It preserves the active search query and the special scope name.
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
        // Keep aggregate counts coherent with the provided options so tests read
        // like real projection output rather than isolated string fixtures.
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
