/*
 * 커밋 그래프 읽기 서비스이다 (plan 27 G1). ref 와 커밋을 port 로 읽고 domain 의 레인 배치로 줄을 만든다.
 *
 * 상태를 가지지 않는다. 쪽을 이어 읽는 데 필요한 값(읽은 수, 레인 상태, ref 지문)은 자리(`GitGraphCursor`)에 담아
 * 클라이언트가 돌려준다. 그래서 데몬이 다시 시작해도 이어 읽을 수 있고, 접속마다 기억을 지울 일이 없다.
 */
use std::sync::Arc;

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::application::port::inbound::git_graph_port::{
    GitGraphCursor, GitGraphPage, GitGraphPageRequest, GitGraphPort,
};
use crate::application::port::outbound::git_history_port::{GitHistoryPort, GitHistoryRefs};
use crate::domain::git_graph::{
    GitGraphCommit, GitGraphLaneState, GitGraphRow, clamp_git_graph_page_limit,
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
        let Some(refs) = self.history.read_refs(&request.repository_dir)? else {
            return Ok(empty_page(false, false, None));
        };
        let refs_digest = refs_digest(&refs);
        let first_page = request.cursor.is_none();
        let (skip, mut lanes) = match request.cursor {
            Some(cursor) if cursor.refs_digest != refs_digest => {
                return Ok(empty_page(true, true, refs.head));
            }
            Some(cursor) => (cursor.skip, cursor.lanes),
            None => (0, GitGraphLaneState::default()),
        };
        let limit = clamp_git_graph_page_limit(request.limit);
        let commits = self.read_commits(&request.repository_dir, &refs, skip, limit)?;
        let has_more = commits.len() > limit;
        let rows = place_rows(&mut lanes, commits, limit);
        let next = has_more.then(|| GitGraphCursor {
            skip: skip + rows.len(),
            lanes,
            refs_digest,
        });
        Ok(GitGraphPage {
            repository: true,
            stale: false,
            head: refs.head,
            refs: if first_page { refs.refs } else { Vec::new() },
            rows,
            next,
        })
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
        refs: Vec::new(),
        rows: Vec::new(),
        next: None,
    }
}

/// ref 들의 지문이다. HEAD 나 ref 하나가 움직이면 값이 바뀐다.
fn refs_digest(refs: &GitHistoryRefs) -> String {
    let mut hasher = Sha256::new();
    hasher.update(refs.head.as_deref().unwrap_or(""));
    for reference in &refs.refs {
        hasher.update([0]);
        hasher.update(reference.name.as_bytes());
        hasher.update([0]);
        hasher.update(reference.target.as_bytes());
        hasher.update([u8::from(reference.head)]);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::domain::git_graph::{GitGraphRef, GitGraphRefKind};

    /// 한 줄 이력 `c3 -> c2 -> c1` 을 가진 가짜 저장소이다.
    struct FakeHistory {
        refs: Mutex<Option<GitHistoryRefs>>,
        commits: Vec<GitGraphCommit>,
        reads: Mutex<Vec<(bool, usize, usize)>>,
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
            Ok(self
                .commits
                .iter()
                .skip(skip)
                .take(limit)
                .cloned()
                .collect())
        }
    }

    fn request(limit: usize, cursor: Option<GitGraphCursor>) -> GitGraphPageRequest {
        GitGraphPageRequest {
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
        assert_eq!(second.next, None);
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
        });
        let page = GitGraphService::new(history.clone())
            .graph_page(request(10, None))
            .unwrap();

        assert!(page.repository);
        assert!(page.rows.is_empty());
        assert!(history.reads.lock().unwrap().is_empty());
    }
}
