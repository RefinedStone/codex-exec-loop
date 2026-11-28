use super::*;
use crate::application::port::inbound::git_graph_port::{GitGraphPageRequest, GitGraphPort};
use crate::application::service::git_graph::GitGraphService;
use crate::domain::git_graph::filter::{BranchScope, GraphFilter};
use std::sync::Arc;

#[test]
fn literal_path_author_scope_and_cursor_share_one_filtered_walk() {
    let directory = temp_repository("paged-filter");
    initialize(&directory);
    let cwd = directory.display().to_string();
    let selected = "[a].txt";
    let mut matches = Vec::new();
    for (name, text) in [
        (selected, "one"),
        ("a.txt", "wrong literal"),
        (selected, "two"),
    ] {
        std::fs::write(directory.join(name), text).unwrap();
        git(&directory, &["add", name]);
        commit(&directory, text);
        if name == selected {
            matches.push(head_sha(&directory));
        }
    }
    git(&directory, &["switch", "-c", "other"]);
    std::fs::write(directory.join(selected), "third").unwrap();
    git(&directory, &["add", selected]);
    commit(&directory, "off branch");
    git(&directory, &["switch", "main"]);
    let service = GitGraphService::new(Arc::new(GitHistoryAdapter));
    let filter = GraphFilter {
        branch: BranchScope::Thread,
        path: selected.into(),
        author: "Akra Test".into(),
        hide: true,
        ..Default::default()
    };
    let page = |limit, cursor, filter| {
        service
            .graph_page(GitGraphPageRequest {
                repository_dir: cwd.clone(),
                limit: Some(limit),
                cursor,
                filter,
            })
            .unwrap()
    };
    let first = page(1, None, filter.clone());
    assert_eq!(first.rows[0].id, matches[1]);
    let second = page(1, first.next.clone(), filter.clone());
    assert_eq!(second.rows[0].id, matches[0]);
    assert!(second.next.is_none());
    let highlight = GraphFilter {
        hide: false,
        ..filter
    };
    assert!(page(1, first.next, highlight.clone()).stale);
    let full = page(10, None, highlight);
    assert_eq!(full.rows.len(), 4);
    for row in full.rows {
        assert_eq!(!row.outside_filter, matches.contains(&row.id));
    }
    std::fs::remove_dir_all(directory).unwrap();
}
