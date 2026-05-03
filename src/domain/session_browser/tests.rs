use super::*;

/*
 * These tests protect the pure domain contract behind the session browser. The
 * TUI owns popup state, key bindings, and rendering, but this module decides
 * which recent-session rows exist, how search and project filters combine, and
 * how selection survives refreshes.
 */
#[test]
fn search_query_resets_page_index() {
    /*
     * Search changes replace the result set. Resetting the page here keeps the
     * overlay from pointing at a page that belonged to the previous query.
     */
    let mut state = SessionBrowserState::new(5);
    state.page_index = 2;

    state.set_search_query("bugfix");

    assert_eq!(state.search_query, "bugfix");
    assert_eq!(state.page_index, 0);
}

#[test]
fn move_page_clamps_to_available_range() {
    // Keyboard page movement may race with catalog refresh, so state clamps locally too.
    let mut state = SessionBrowserState::new(5);

    state.move_page(3, 2);
    assert_eq!(state.page_index, 1);

    state.move_page(-9, 2);
    assert_eq!(state.page_index, 0);
}

#[test]
fn clear_resets_query_filter_and_page_index() {
    /*
     * Clear is the "return to default browser" command. It intentionally leaves
     * page_size alone because page size is a layout policy, not user filter state.
     */
    let mut state = SessionBrowserState::new(5);
    state.set_search_query("docs");
    state.set_project_filter(SessionProjectFilter::RecentProject {
        workspace_directory: "/tmp/root".to_string(),
    });
    state.page_index = 3;

    state.clear();

    assert!(state.search_query.is_empty());
    assert_eq!(state.page_index, 0);
    assert_eq!(state.project_filter, SessionProjectFilter::AllProjects);
}

#[test]
fn project_recent_sessions_filters_by_query_and_project() {
    /*
     * Projection applies the project boundary first, then search. The counts stay
     * separate so empty-state copy can say whether the user filtered out a project
     * or whether the search query removed rows inside that project.
     */
    let recent_sessions = RecentSessions {
        items: vec![
            sample_session("thread-1", "/tmp/root-a", "bugfix queue"),
            sample_session("thread-2", "/tmp/root-a", "docs refresh"),
            sample_session("thread-3", "/tmp/root-b", "bugfix release"),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    };
    let mut browser_state = SessionBrowserState::new(2);
    browser_state.set_search_query("bugfix");
    browser_state.set_project_filter(SessionProjectFilter::RecentProject {
        workspace_directory: "/tmp/root-b".to_string(),
    });
    let projection = project_recent_sessions(&recent_sessions, &browser_state, None);

    assert_eq!(projection.total_session_count, 3);
    assert_eq!(projection.project_filtered_session_count, 1);
    assert_eq!(projection.filtered_session_count, 1);
    assert_eq!(projection.total_pages, 1);
    assert_eq!(projection.visible_session_range, Some((1, 1)));
    assert_eq!(projection.page_session_indexes, vec![2]);
    assert_eq!(
        projection.active_project_filter,
        SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/root-b".to_string(),
        }
    );
}

#[test]
fn project_recent_sessions_clamps_stale_page_and_filter_state() {
    /*
     * A selected project can disappear after a reload. The domain falls back to
     * AllProjects and clamps the stale page index, giving the TUI a renderable
     * page instead of requiring adapter-side recovery.
     */
    let recent_sessions = RecentSessions {
        items: vec![
            sample_session("thread-1", "/tmp/root-a", "alpha"),
            sample_session("thread-2", "/tmp/root-a", "beta"),
            sample_session("thread-3", "/tmp/root-b", "gamma"),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    };
    let browser_state = SessionBrowserState {
        search_query: String::new(),
        page_index: 5,
        page_size: 2,
        project_filter: SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/missing".to_string(),
        },
    };
    let projection = project_recent_sessions(&recent_sessions, &browser_state, None);

    assert_eq!(
        projection.active_project_filter,
        SessionProjectFilter::AllProjects
    );
    assert_eq!(projection.total_session_count, 3);
    assert_eq!(projection.project_filtered_session_count, 3);
    assert_eq!(projection.total_pages, 2);
    assert_eq!(projection.page_index, 1);
    assert_eq!(projection.visible_session_range, Some((3, 3)));
    assert_eq!(projection.page_session_indexes, vec![2]);
}

#[test]
fn project_recent_sessions_matches_query_without_allocating_title_haystacks() {
    /*
     * Multi-token search is AND-based across searchable fields. The title-like
     * preview already lives on SessionSummary, so the projection should match both
     * tokens without building another display haystack in presentation code.
     */
    let recent_sessions = RecentSessions {
        items: vec![
            sample_session("thread-1", "/tmp/root-a", "Docs release prep"),
            sample_session("thread-2", "/tmp/root-b", "bugfix queue"),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    };
    let mut browser_state = SessionBrowserState::new(10);
    browser_state.set_search_query("docs release");
    let projection = project_recent_sessions(&recent_sessions, &browser_state, None);

    assert_eq!(projection.page_session_indexes, vec![0]);
}

#[test]
fn project_recent_sessions_ranks_name_and_branch_hits_ahead_of_preview_only_matches() {
    /*
     * Ranking reflects operator intent: friendly session names are stronger than
     * branch context, and branch context is stronger than a loose preview match.
     * Keeping this in the domain prevents the TUI row builder from duplicating
     * search scoring rules.
     */
    let recent_sessions = RecentSessions {
        items: vec![
            sample_named_session(
                "thread-preview",
                "/tmp/root-a",
                "release notes hidden in preview",
                None,
                Some("main"),
                1_700_000_300,
            ),
            sample_named_session(
                "thread-name",
                "/tmp/root-b",
                "maintenance",
                Some("release prep"),
                Some("main"),
                1_700_000_100,
            ),
            sample_named_session(
                "thread-branch",
                "/tmp/root-c",
                "maintenance",
                None,
                Some("release/final"),
                1_700_000_200,
            ),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    };
    let mut browser_state = SessionBrowserState::new(10);
    browser_state.set_search_query("release");
    let projection = project_recent_sessions(&recent_sessions, &browser_state, None);

    assert_eq!(
        projection.page_session_indexes,
        vec![1, 2, 0],
        "name hits should outrank branch hits, and branch hits should outrank preview-only hits"
    );
}

#[test]
fn project_recent_sessions_reports_visible_match_range_for_ranked_results() {
    /*
     * The visible range is display copy, but it is derived from the same ranked
     * indexes as the page rows. This prevents summary text from drifting away from
     * the actual rows after search reorders sessions.
     */
    let recent_sessions = RecentSessions {
        items: vec![
            sample_named_session(
                "thread-1",
                "/tmp/root-a",
                "docs checklist",
                Some("alpha"),
                Some("main"),
                1_700_000_000,
            ),
            sample_named_session(
                "thread-2",
                "/tmp/root-a",
                "release prep",
                Some("docs launch"),
                Some("main"),
                1_699_999_900,
            ),
            sample_named_session(
                "thread-3",
                "/tmp/root-a",
                "docs rollout",
                Some("zeta"),
                Some("main"),
                1_700_000_100,
            ),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    };
    let mut browser_state = SessionBrowserState::new(10);
    browser_state.set_search_query("docs");
    let projection = project_recent_sessions(&recent_sessions, &browser_state, None);

    assert_eq!(projection.page_session_indexes, vec![1, 2, 0]);
    assert_eq!(projection.visible_session_range, Some((1, 3)));
}

#[test]
fn project_recent_sessions_marks_current_workspace_filter_context() {
    /*
     * Current-workspace context is an annotation on filter options, not an
     * implicit filter. The shell can highlight the current project without hiding
     * other recent sessions until the operator cycles to that filter.
     */
    let recent_sessions = RecentSessions {
        items: vec![
            sample_session("thread-1", "/tmp/root-a", "alpha"),
            sample_session("thread-2", "/tmp/root-a", "beta"),
            sample_session("thread-3", "/tmp/root-b", "gamma"),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    };
    let browser_state = SessionBrowserState::default();
    let projection = project_recent_sessions(&recent_sessions, &browser_state, Some("/tmp/root-b"));

    assert_eq!(projection.current_workspace_session_count, 1);
    assert_eq!(
        projection
            .project_filter_options
            .iter()
            .find(|option| option.is_current_workspace)
            .map(|option| &option.filter),
        Some(&SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/root-b".to_string()
        })
    );
}

#[test]
fn cycled_project_filter_wraps_across_available_options() {
    /*
     * Tab/BackTab in the overlay delegates to this wraparound calculation. The
     * projection owns it because only the projection knows which project filters
     * are currently available after catalog reload.
     */
    let projection = SessionBrowserProjection {
        active_project_filter: SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/root-b".to_string(),
        },
        project_filter_options: vec![
            SessionProjectFilterOption {
                filter: SessionProjectFilter::AllProjects,
                session_count: 3,
                is_current_workspace: false,
            },
            SessionProjectFilterOption {
                filter: SessionProjectFilter::RecentProject {
                    workspace_directory: "/tmp/root-a".to_string(),
                },
                session_count: 2,
                is_current_workspace: false,
            },
            SessionProjectFilterOption {
                filter: SessionProjectFilter::RecentProject {
                    workspace_directory: "/tmp/root-b".to_string(),
                },
                session_count: 1,
                is_current_workspace: true,
            },
        ],
        current_workspace_session_count: 1,
        total_session_count: 3,
        project_filtered_session_count: 1,
        filtered_session_count: 1,
        page_index: 0,
        total_pages: 1,
        visible_session_range: Some((1, 1)),
        page_session_indexes: vec![2],
    };

    assert_eq!(
        projection.cycled_project_filter(1),
        Some(SessionProjectFilter::AllProjects)
    );
    assert_eq!(
        projection.cycled_project_filter(-1),
        Some(SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/root-a".to_string(),
        })
    );
}

#[test]
fn browser_page_clamps_selection_to_visible_page() {
    /*
     * Page construction is the bridge from projection indexes back to borrowed
     * SessionSummary rows. A stale selected index must clamp to the visible page
     * so the renderer always has a coherent highlighted row.
     */
    let recent_sessions = RecentSessions {
        items: vec![
            sample_session("thread-1", "/tmp/root-a", "alpha"),
            sample_session("thread-2", "/tmp/root-a", "beta"),
            sample_session("thread-3", "/tmp/root-b", "gamma"),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    };
    let browser_state = SessionBrowserState {
        search_query: String::new(),
        page_index: 1,
        page_size: 2,
        project_filter: SessionProjectFilter::AllProjects,
    };
    let browser_page = build_session_browser_page(&recent_sessions, &browser_state, None, None, 5);

    assert_eq!(browser_page.selected_index, Some(0));
    assert_eq!(
        browser_page
            .selected_session()
            .map(|session| session.id.as_str()),
        Some("thread-3")
    );
}

#[test]
fn browser_page_preserves_selected_session_by_id_after_filtering() {
    /*
     * Selection is restored by session id before falling back to row index. This
     * matters when search/filter changes reorder or shrink the page but the same
     * logical session remains visible.
     */
    let recent_sessions = RecentSessions {
        items: vec![
            sample_session("thread-1", "/tmp/root-a", "alpha"),
            sample_session("thread-2", "/tmp/root-a", "beta"),
            sample_session("thread-3", "/tmp/root-b", "docs release"),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    };
    let browser_state = SessionBrowserState {
        search_query: "docs".to_string(),
        page_index: 0,
        page_size: 10,
        project_filter: SessionProjectFilter::AllProjects,
    };
    let browser_page =
        build_session_browser_page(&recent_sessions, &browser_state, None, Some("thread-3"), 1);

    assert_eq!(browser_page.selected_index, Some(0));
    assert_eq!(
        browser_page
            .selected_session()
            .map(|session| session.id.as_str()),
        Some("thread-3")
    );
}

#[test]
fn browser_page_selection_after_delta_clamps_and_preserves_session_id() {
    /*
     * Arrow-key movement returns both visible-row index and session id. The index
     * lets the current frame update immediately; the id lets the next refreshed
     * page restore the same logical selection.
     */
    let recent_sessions = RecentSessions {
        items: vec![
            sample_session("thread-1", "/tmp/root-a", "alpha"),
            sample_session("thread-2", "/tmp/root-a", "beta"),
            sample_session("thread-3", "/tmp/root-b", "gamma"),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    };
    let browser_state = SessionBrowserState::default();
    let browser_page =
        build_session_browser_page(&recent_sessions, &browser_state, None, Some("thread-2"), 0);
    let selection = browser_page.selection_after_delta(5);

    assert_eq!(
        selection,
        SessionBrowserSelection {
            index: 2,
            session_id: Some("thread-3".to_string()),
        }
    );
}

#[test]
fn browser_page_last_selection_returns_last_visible_session() {
    // End-key behavior uses the same selection value shape as incremental movement.
    let recent_sessions = RecentSessions {
        items: vec![
            sample_session("thread-1", "/tmp/root-a", "alpha"),
            sample_session("thread-2", "/tmp/root-a", "beta"),
            sample_session("thread-3", "/tmp/root-b", "gamma"),
        ],
        warnings: Vec::new(),
        next_cursor: None,
    };
    let browser_state = SessionBrowserState::default();
    let browser_page = build_session_browser_page(&recent_sessions, &browser_state, None, None, 0);
    let selection = browser_page.last_selection();

    assert_eq!(
        selection,
        SessionBrowserSelection {
            index: 2,
            session_id: Some("thread-3".to_string()),
        }
    );
}

// Minimal ready session fixture with id/name/branch defaults for state-oriented tests.
fn sample_session(id: &str, cwd: &str, preview: &str) -> SessionSummary {
    sample_named_session(id, cwd, preview, Some(id), Some("main"), 1_700_000_000)
}

/*
 * Search-ranking tests need optional name and branch fields plus controlled
 * update times. Keeping the helper in this domain module makes every fixture use
 * the same SessionSummary shape as the real catalog adapter returns.
 */
fn sample_named_session(
    id: &str,
    cwd: &str,
    preview: &str,
    name: Option<&str>,
    git_branch: Option<&str>,
    updated_at_epoch: i64,
) -> SessionSummary {
    SessionSummary {
        id: id.to_string(),
        name: name.map(str::to_string),
        preview: preview.to_string(),
        cwd: cwd.to_string(),
        source: "codex".to_string(),
        model_provider: "openai".to_string(),
        updated_at_epoch,
        status_type: "ready".to_string(),
        path: format!("{cwd}/{id}.json"),
        git_branch: git_branch.map(str::to_string),
    }
}
