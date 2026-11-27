/*
 * 커밋 그래프 읽기 서비스이다 (plan 27 G1, G2). ref 와 커밋을 port 로 읽고 domain 의 레인 배치로 줄을 만든다.
 *
 * 상태를 가지지 않는다. 쪽을 이어 읽는 데 필요한 값(읽은 수, 레인 상태, ref 지문)은 자리(`GitGraphCursor`)에 담아
 * 클라이언트가 돌려준다. 그래서 데몬이 다시 시작해도 이어 읽을 수 있고, 접속마다 기억을 지울 일이 없다.
 */
use std::sync::Arc;

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};

use crate::application::port::inbound::git_graph_port::{
    GitGraphCursor, GitGraphPage, GitGraphPageRequest, GitGraphPort, GitGraphRefsState,
};
use crate::application::port::outbound::git_history_port::{
    GitHistoryPort, GitHistoryRefs, GitWorktreeProgress,
};
use crate::domain::git_graph::{
    GIT_GRAPH_LANE_LIMIT, GitCommitDetail, GitGraphCommit, GitGraphLaneState, GitGraphRefKind,
    GitGraphRow, clamp_git_graph_page_limit, is_git_commit_id,
};

pub struct GitGraphService {
    history: Arc<dyn GitHistoryPort>,
}

impl GitGraphService {
    pub fn new(history: Arc<dyn GitHistoryPort>) -> Self {
        Self { history }
    }
}

impl GitGraphPort for GitGraphService {
    fn graph_page(&self, request: GitGraphPageRequest) -> Result<GitGraphPage> {
        request.filter.validate()?;
        let Some(refs) = self.history.read_refs(&request.repository_dir)? else {
            return Ok(empty_page(false, false, None));
        };
        let page_digest = refs_digest(&refs);
        let first_page = request.cursor.is_none();
        let (skip, mut lanes) = match request.cursor {
            Some(cursor)
                if cursor.refs_digest != page_digest || cursor.filter != request.filter =>
            {
                return Ok(empty_page(true, true, refs.head));
            }
            Some(cursor) if cursor.lanes.lane_count() > GIT_GRAPH_LANE_LIMIT => {
                bail!("the graph cursor has more than {GIT_GRAPH_LANE_LIMIT} lanes");
            }
            Some(cursor) => (cursor.skip, cursor.lanes),
            None => (0, GitGraphLaneState::default()),
        };
        let limit = clamp_git_graph_page_limit(request.limit);
        let filtered = request.filter.active();
        let hide = filtered && request.filter.hide;
        let commits = if hide {
            self.history.filtered_page(
                &request.repository_dir,
                &request.filter,
                &refs,
                skip..skip.saturating_add(limit + 1),
            )?
        } else {
            self.read_commits(&request.repository_dir, &refs, skip, limit)?
        };
        let has_more = commits.len() > limit;
        let mut rows = place_rows(&mut lanes, commits, limit);
        if filtered && !hide {
            let matches = self.history.matches_on_page(
                &request.repository_dir,
                &request.filter,
                &refs,
                &rows,
            )?;
            for row in &mut rows {
                row.outside_filter = !matches.contains(&row.id);
            }
        }
        self.mark_backup_only_rows(&request.repository_dir, &refs, &mut rows)?;
        // log와 백업 분류는 움직이는 ref를 읽는다. 읽는 동안 바뀌었으면
        // 이 쪽과 커서를 버려 서로 다른 이력의 줄을 이어 붙이지 않는다.
        let Some(current_refs) = self.history.read_refs(&request.repository_dir)? else {
            return Ok(empty_page(false, true, None));
        };
        if refs_digest(&current_refs) != page_digest {
            return Ok(empty_page(true, true, current_refs.head));
        }
        let next = has_more.then(|| GitGraphCursor {
            filter: request.filter,
            skip: skip.saturating_add(rows.len()),
            lanes,
            refs_digest: page_digest.clone(),
        });
        let (first_refs, first_worktrees) = if first_page {
            (refs.refs, refs.worktrees)
        } else {
            (Vec::new(), Vec::new())
        };
        Ok(GitGraphPage {
            repository: true,
            stale: false,
            head: refs.head,
            refs_digest: Some(page_digest),
            refs: first_refs,
            worktrees: first_worktrees,
            rows,
            next,
        })
    }

    fn refs_state(&self, repository_dir: &str) -> Result<GitGraphRefsState> {
        let refs = self.history.read_refs(repository_dir)?;
        let progress = match refs {
            Some(_) => self.history.read_worktree_progress(repository_dir)?,
            None => GitWorktreeProgress::default(),
        };
        Ok(GitGraphRefsState {
            rewritten_remote: progress.rewritten_remote,
            repository: refs.is_some(),
            refs_digest: refs.as_ref().map(refs_digest),
            operation_in_progress: progress.operation,
            conflicted_files: progress.conflicted_files,
        })
    }

    fn commit_detail(&self, repository_dir: &str, commit: &str) -> Result<Option<GitCommitDetail>> {
        if !is_git_commit_id(commit) {
            bail!("the commit id must be a full sha of 40 or 64 hexadecimal characters");
        }
        self.history.read_commit_detail(repository_dir, commit)
    }
}

impl GitGraphService {
    /// `limit` 보다 하나 더 읽는다. 하나가 더 오면 다음 쪽이 있다.
    fn read_commits(
        &self,
        repository_dir: &str,
        refs: &GitHistoryRefs,
        skip: usize,
        limit: usize,
    ) -> Result<Vec<GitGraphCommit>> {
        if refs.head.is_none() && refs.refs.is_empty() {
            return Ok(Vec::new());
        }
        self.history
            .read_commits(repository_dir, refs.head.is_some(), skip, limit + 1)
    }

    /// 백업 ref 에서만 닿는 줄에 표를 한다. 백업 ref 가 없는 저장소에서는 git 을 부르지 않는다.
    fn mark_backup_only_rows(
        &self,
        repository_dir: &str,
        refs: &GitHistoryRefs,
        rows: &mut [GitGraphRow],
    ) -> Result<()> {
        let has_backup = refs
            .refs
            .iter()
            .any(|reference| reference.kind == GitGraphRefKind::Backup);
        if !has_backup || rows.is_empty() {
            return Ok(());
        }
        let backup_only = self
            .history
            .read_backup_only_commits(repository_dir, refs.head.is_some())?;
        for row in rows {
            row.backup_only = backup_only.contains(&row.id);
        }
        Ok(())
    }
}

fn place_rows(
    lanes: &mut GitGraphLaneState,
    commits: Vec<GitGraphCommit>,
    limit: usize,
) -> Vec<GitGraphRow> {
    commits
        .into_iter()
        .take(limit)
        .map(|commit| lanes.place(commit))
        .collect()
}

fn empty_page(repository: bool, stale: bool, head: Option<String>) -> GitGraphPage {
    GitGraphPage {
        repository,
        stale,
        head,
        refs_digest: None,
        refs: Vec::new(),
        worktrees: Vec::new(),
        rows: Vec::new(),
        next: None,
    }
}

/// ref 들의 지문이다. HEAD, ref 하나, 작업 트리 하나가 움직이면 값이 바뀐다.
fn refs_digest(refs: &GitHistoryRefs) -> String {
    let mut hasher = Sha256::new();
    hasher.update(refs.head.as_deref().unwrap_or(""));
    for reference in &refs.refs {
        hasher.update([0, reference.kind as u8]);
        hasher.update(reference.name.as_bytes());
        hasher.update([0]);
        hasher.update(reference.target.as_bytes());
        hasher.update([u8::from(reference.head)]);
    }
    for worktree in &refs.worktrees {
        hasher.update([1]);
        hasher.update(worktree.path.as_bytes());
        hasher.update([0]);
        hasher.update(worktree.head.as_deref().unwrap_or(""));
        hasher.update([0]);
        hasher.update(worktree.branch.as_deref().unwrap_or(""));
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Mutex;

    use super::*;
    use crate::domain::git_graph::{GitGraphRef, GitGraphWorktree};

    /// 한 줄 이력 `c3 -> c2 -> c1` 을 가진 가짜 저장소이다.
    struct FakeHistory {
        refs: Mutex<Option<GitHistoryRefs>>,
        commits: Vec<GitGraphCommit>,
        reads: Mutex<Vec<(bool, usize, usize)>>,
        backup_only: HashSet<String>,
        backup_reads: Mutex<usize>,
        refs_after_commit_read: Mutex<Option<Option<GitHistoryRefs>>>,
        progress: GitWorktreeProgress,
    }

    impl FakeHistory {
        fn linear(count: usize) -> Self {
            let commits: Vec<GitGraphCommit> = (1..=count)
                .rev()
                .map(|number| GitGraphCommit {
                    id: format!("c{number}"),
                    parents: if number > 1 {
                        vec![format!("c{}", number - 1)]
                    } else {
                        Vec::new()
                    },
                    author_name: "author".to_string(),
                    author_time: number as i64,
                    subject: format!("commit {number}"),
                })
                .collect();
            Self {
                refs: Mutex::new(Some(refs_at(&format!("c{count}")))),
                commits,
                reads: Mutex::new(Vec::new()),
                backup_only: HashSet::new(),
                backup_reads: Mutex::new(0),
                refs_after_commit_read: Mutex::new(None),
                progress: GitWorktreeProgress::default(),
            }
        }
    }

    fn refs_at(head: &str) -> GitHistoryRefs {
        GitHistoryRefs {
            head: Some(head.to_string()),
            refs: vec![GitGraphRef {
                name: "main".to_string(),
                kind: GitGraphRefKind::LocalBranch,
                target: head.to_string(),
                head: true,
            }],
            worktrees: vec![GitGraphWorktree {
                path: "/repo".to_string(),
                head: Some(head.to_string()),
                branch: Some("main".to_string()),
            }],
        }
    }

    fn backup_ref(target: &str) -> GitGraphRef {
        GitGraphRef {
            name: "main/op-1".to_string(),
            kind: GitGraphRefKind::Backup,
            target: target.to_string(),
            head: false,
        }
    }

    impl GitHistoryPort for FakeHistory {
        fn read_refs(&self, _repository_dir: &str) -> Result<Option<GitHistoryRefs>> {
            Ok(self.refs.lock().unwrap().clone())
        }

        fn read_commits(
            &self,
            _repository_dir: &str,
            include_head: bool,
            skip: usize,
            limit: usize,
        ) -> Result<Vec<GitGraphCommit>> {
            self.reads.lock().unwrap().push((include_head, skip, limit));
            if let Some(changed) = self.refs_after_commit_read.lock().unwrap().take() {
                *self.refs.lock().unwrap() = changed;
            }
            Ok(self
                .commits
                .iter()
                .skip(skip)
                .take(limit)
                .cloned()
                .collect())
        }

        fn read_backup_only_commits(
            &self,
            _repository_dir: &str,
            _include_head: bool,
        ) -> Result<HashSet<String>> {
            *self.backup_reads.lock().unwrap() += 1;
            Ok(self.backup_only.clone())
        }

        fn read_worktree_progress(&self, _repository_dir: &str) -> Result<GitWorktreeProgress> {
            Ok(self.progress.clone())
        }

        fn read_commit_detail(
            &self,
            _repository_dir: &str,
            _commit: &str,
        ) -> Result<Option<GitCommitDetail>> {
            Ok(None)
        }
    }

    fn request(limit: usize, cursor: Option<GitGraphCursor>) -> GitGraphPageRequest {
        GitGraphPageRequest {
            filter: Default::default(),
            repository_dir: "/repo".to_string(),
            limit: Some(limit),
            cursor,
        }
    }

    #[test]
    fn pages_continue_until_the_history_ends() {
        let history = Arc::new(FakeHistory::linear(3));
        let service = GitGraphService::new(history.clone());

        let first = service.graph_page(request(2, None)).unwrap();
        assert!(first.repository);
        assert_eq!(first.head.as_deref(), Some("c3"));
        assert_eq!(first.refs.len(), 1);
        assert_eq!(first.worktrees.len(), 1);
        assert!(first.refs_digest.is_some());
        assert_eq!(
            first.refs_digest,
            service.refs_state("/repo").unwrap().refs_digest
        );
        assert_eq!(
            first
                .rows
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            ["c3", "c2"]
        );
        let cursor = first.next.expect("a third commit remains");
        assert_eq!(cursor.skip, 2);

        let second = service.graph_page(request(2, Some(cursor))).unwrap();
        assert_eq!(second.rows.len(), 1);
        assert_eq!(second.rows[0].id, "c1");
        // 앞 쪽의 레인 상태를 이어받았다. c1 은 레인 0 에서 c2 의 선을 받는다.
        assert_eq!(second.rows[0].lanes_in, vec![0]);
        assert!(second.refs.is_empty());
        assert!(second.worktrees.is_empty());
        assert_eq!(second.refs_digest, first.refs_digest);
        assert_eq!(second.next, None);
        // 백업 ref 가 없다. 백업 커밋을 찾는 git 을 부르지 않는다.
        assert_eq!(*history.backup_reads.lock().unwrap(), 0);
        assert_eq!(*history.reads.lock().unwrap(), [(true, 0, 3), (true, 2, 3)]);
    }

    #[test]
    fn a_cursor_from_before_a_ref_change_is_stale() {
        let history = Arc::new(FakeHistory::linear(3));
        let service = GitGraphService::new(history.clone());
        let cursor = service.graph_page(request(1, None)).unwrap().next.unwrap();

        *history.refs.lock().unwrap() = Some(refs_at("c2"));
        let page = service.graph_page(request(1, Some(cursor))).unwrap();

        assert!(page.stale);
        assert!(page.rows.is_empty());
        assert_eq!(page.next, None);
    }

    #[test]
    fn a_ref_change_during_the_first_or_next_commit_read_discards_the_page() {
        for next_page in [false, true] {
            let history = Arc::new(FakeHistory::linear(3));
            let service = GitGraphService::new(history.clone());
            let cursor = if next_page {
                service.graph_page(request(1, None)).unwrap().next
            } else {
                None
            };
            *history.refs_after_commit_read.lock().unwrap() = Some(Some(refs_at("c2")));

            let page = service.graph_page(request(1, cursor)).unwrap();

            assert!(page.repository);
            assert!(page.stale);
            assert_eq!(page.head.as_deref(), Some("c2"));
            assert!(page.rows.is_empty());
            assert!(page.refs.is_empty());
            assert!(page.next.is_none());
            assert!(page.refs_digest.is_none());
        }
    }

    #[test]
    fn a_repository_removed_during_commit_read_discards_the_page() {
        let history = Arc::new(FakeHistory::linear(3));
        *history.refs_after_commit_read.lock().unwrap() = Some(None);
        let page = GitGraphService::new(history)
            .graph_page(request(1, None))
            .unwrap();

        assert!(!page.repository);
        assert!(page.stale);
        assert!(page.rows.is_empty());
        assert!(page.next.is_none());
    }

    #[test]
    fn the_refs_state_changes_when_a_ref_or_a_worktree_moves() {
        let history = Arc::new(FakeHistory::linear(3));
        let service = GitGraphService::new(history.clone());
        let before = service.refs_state("/repo").unwrap();
        assert!(before.repository);

        let mut with_worktree = refs_at("c3");
        with_worktree.worktrees.push(GitGraphWorktree {
            path: "/repo-topic".to_string(),
            head: Some("c2".to_string()),
            branch: None,
        });
        *history.refs.lock().unwrap() = Some(with_worktree);
        assert_ne!(
            service.refs_state("/repo").unwrap().refs_digest,
            before.refs_digest
        );

        *history.refs.lock().unwrap() = Some(refs_at("c2"));
        assert_ne!(
            service.refs_state("/repo").unwrap().refs_digest,
            before.refs_digest
        );

        *history.refs.lock().unwrap() = None;
        let outside = service.refs_state("/repo").unwrap();
        assert!(!outside.repository);
        assert_eq!(outside.refs_digest, None);
    }

    #[test]
    fn the_refs_state_carries_the_unfinished_work_of_the_worktree() {
        let mut history = FakeHistory::linear(2);
        history.progress = GitWorktreeProgress {
            rewritten_remote: None,
            operation: Some("rebase".to_string()),
            conflicted_files: 2,
        };
        let history = Arc::new(history);
        let service = GitGraphService::new(history.clone());
        let state = service.refs_state("/repo").unwrap();
        assert_eq!(state.operation_in_progress.as_deref(), Some("rebase"));
        assert_eq!(state.conflicted_files, 2);

        *history.refs.lock().unwrap() = None;
        let outside = service.refs_state("/repo").unwrap();
        assert_eq!(outside.operation_in_progress, None);
        assert_eq!(outside.conflicted_files, 0);
    }

    #[test]
    fn rows_reached_only_from_a_backup_ref_are_marked() {
        let mut history = FakeHistory::linear(3);
        history.backup_only = HashSet::from(["c3".to_string()]);
        let mut refs = refs_at("c2");
        refs.refs.push(backup_ref("c3"));
        *history.refs.lock().unwrap() = Some(refs);
        let history = Arc::new(history);

        let page = GitGraphService::new(history.clone())
            .graph_page(request(10, None))
            .unwrap();

        let marked: Vec<(&str, bool)> = page
            .rows
            .iter()
            .map(|row| (row.id.as_str(), row.backup_only))
            .collect();
        assert_eq!(marked, [("c3", true), ("c2", false), ("c1", false)]);
        assert_eq!(*history.backup_reads.lock().unwrap(), 1);
    }

    #[test]
    fn commit_detail_refuses_text_that_is_not_a_sha() {
        let service = GitGraphService::new(Arc::new(FakeHistory::linear(1)));

        assert!(service.commit_detail("/repo", "--all").is_err());
        assert!(service.commit_detail("/repo", "c1").is_err());
        assert!(service.commit_detail("/repo", "abcdef1").is_err());
        assert_eq!(
            service.commit_detail("/repo", &"a".repeat(40)).unwrap(),
            None
        );
    }

    #[test]
    fn a_cursor_with_too_many_lanes_is_refused() {
        let history = Arc::new(FakeHistory::linear(3));
        let service = GitGraphService::new(history.clone());
        let mut cursor = service.graph_page(request(1, None)).unwrap().next.unwrap();
        let lanes = vec![Some("x"); GIT_GRAPH_LANE_LIMIT + 1];
        cursor.lanes = serde_json::from_value(serde_json::json!(lanes)).unwrap();

        assert!(service.graph_page(request(1, Some(cursor))).is_err());
        // 첫 쪽의 읽기만 있었다. 거절한 자리로는 git 을 부르지 않는다.
        assert_eq!(history.reads.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_folder_that_is_not_a_repository_gives_an_empty_page() {
        let history = Arc::new(FakeHistory::linear(1));
        *history.refs.lock().unwrap() = None;
        let page = GitGraphService::new(history.clone())
            .graph_page(request(10, None))
            .unwrap();

        assert!(!page.repository);
        assert!(page.rows.is_empty());
        assert!(history.reads.lock().unwrap().is_empty());
    }

    #[test]
    fn a_repository_without_commits_reads_no_history() {
        let history = Arc::new(FakeHistory::linear(1));
        *history.refs.lock().unwrap() = Some(GitHistoryRefs {
            head: None,
            refs: Vec::new(),
            worktrees: Vec::new(),
        });
        let page = GitGraphService::new(history.clone())
            .graph_page(request(10, None))
            .unwrap();

        assert!(page.repository);
        assert!(page.rows.is_empty());
        assert!(history.reads.lock().unwrap().is_empty());
    }
}
