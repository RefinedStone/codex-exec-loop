/*
 * git 커밋 그래프(plan 27 G1, G2)의 값 타입과 레인 배치 규칙이다. I/O 가 없다. 커밋 목록은 git 이 정한 순서
 * (자식이 부모보다 먼저)로 들어온다. 이 모듈은 커밋마다 그래프 열에서 어느 레인에 점을 찍고 어느 레인으로 선을
 * 잇는지 정한다.
 *
 * 배치는 위에서 아래로 한 줄씩 한다. 그래서 쪽 단위로 끊을 수 있다: 한 쪽의 끝에서 `GitGraphLaneState` 를
 * 넘기면 다음 쪽이 같은 그림을 이어 간다.
 */
use serde::{Deserialize, Serialize};

/// 한 번에 읽는 커밋 수의 최댓값이다.
pub const GIT_GRAPH_PAGE_LIMIT_MAX: usize = 1000;
/// 자리가 담을 수 있는 레인 수의 최댓값이다. 줄마다 지나가는 레인을 모두 적으므로, 레인 수는 답의 크기를 줄 수와
/// 곱으로 키운다. 클라이언트가 돌려준 자리가 이 수를 넘으면 받지 않는다.
pub const GIT_GRAPH_LANE_LIMIT: usize = 4096;
/// 요청이 개수를 주지 않을 때 읽는 커밋 수이다.
pub const GIT_GRAPH_PAGE_LIMIT_DEFAULT: usize = 300;
/// 백업 ref 의 전체 이름의 앞부분이다 (plan 27 §3.2). 뒤는 `<branch>/<op-id>` 이다.
pub const GIT_BACKUP_REF_PREFIX: &str = "refs/akra/backup/";

/// git 이 준 커밋 하나이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitGraphCommit {
    /// 전체 sha 이다.
    pub id: String,
    /// 부모의 전체 sha 이다. 첫 부모가 맨 앞이다.
    pub parents: Vec<String>,
    pub author_name: String,
    /// 작성 시각이다. Unix 초이다.
    pub author_time: i64,
    /// 커밋 메시지의 첫 줄이다.
    pub subject: String,
}

/// ref 의 종류이다. 화면이 라벨 모양을 고르는 데 쓴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GitGraphRefKind {
    LocalBranch,
    RemoteBranch,
    Tag,
    /// Akra 가 파괴적 작업 전에 쓴 백업 ref 이다 (`refs/akra/backup/*`).
    Backup,
}

/// 커밋을 가리키는 ref 하나이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitGraphRef {
    /// 짧은 이름이다 (`main`, `origin/main`, `v1.0`). 백업 ref 는 `<branch>/<op-id>` 이다.
    pub name: String,
    pub kind: GitGraphRefKind,
    /// 가리키는 커밋의 전체 sha 이다. 주석 태그는 태그가 가리키는 커밋이다.
    pub target: String,
    /// 이 작업 트리의 HEAD 가 이 브랜치이다.
    pub head: bool,
}

/// 그래프의 한 줄이다. 줄은 위쪽 경계에서 선을 받고 아래쪽 경계로 선을 보낸다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitGraphRow {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outside_filter: bool,
    pub id: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_time: i64,
    pub subject: String,
    /// 커밋의 점이 있는 레인이다.
    pub lane: usize,
    /// 위쪽 경계에서 점으로 들어오는 레인들이다. 이 커밋을 기다리던 자식들의 선이다.
    pub lanes_in: Vec<usize>,
    /// 점에서 아래쪽 경계로 나가는 레인들이다. 부모마다 하나이다.
    pub lanes_out: Vec<usize>,
    /// 점과 만나지 않고 위에서 아래로 지나가는 레인들이다.
    pub lanes_through: Vec<usize>,
    /// 백업 ref 에서만 닿는 커밋이다. 브랜치, 태그, HEAD 에서는 닿지 않는다. 화면은 흐리게 그린다.
    #[serde(skip_serializing_if = "is_false")]
    pub backup_only: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// 저장소의 작업 트리 하나이다. 화면은 이 값으로 스레드의 폴더를 저장소의 커밋에 잇는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitGraphWorktree {
    /// 작업 트리의 폴더이다. git 이 쓴 그대로이다 (Windows 에서도 `/` 이다).
    pub path: String,
    /// 체크아웃한 커밋의 전체 sha 이다. bare 저장소와 커밋이 없는 작업 트리는 없다.
    pub head: Option<String>,
    /// 체크아웃한 브랜치의 짧은 이름이다. 브랜치가 아니면 없다.
    pub branch: Option<String>,
}

/// 커밋 하나의 상세이다. 그래프의 줄에 없는 값(본문, 커밋한 사람, 변경)을 가진다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitDetail {
    pub id: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    /// Unix 초이다.
    pub author_time: i64,
    pub committer_name: String,
    /// Unix 초이다.
    pub committer_time: i64,
    /// 커밋 메시지 전체이다. 첫 줄이 제목이다.
    pub message: String,
    /// git diff 형식의 변경이다. 병합 커밋은 첫 부모와 비교한다. `diff_too_large` 이면 비어 있다.
    pub diff: String,
    /// 변경이 한도를 넘어서 읽지 않았다.
    pub diff_too_large: bool,
}

/// 커밋 id 로 받을 수 있는 글이다: 전체 sha 이다 (16 진수 40 자, SHA-256 저장소는 64 자). git 의 옵션이나 rev 식으로
/// 읽힐 글을 막는다. 짧은 sha 는 받지 않는다: git 은 짧은 16 진수 글을 같은 이름의 ref 로 먼저 읽고, 두 객체의
/// 앞부분이면 고르지 못한다.
pub fn is_git_commit_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// 줄과 줄 사이의 레인 상태이다. 레인마다 "다음에 나올 커밋"을 가진다. 빈 레인은 `None` 이다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GitGraphLaneState {
    lanes: Vec<Option<String>>,
}

impl GitGraphLaneState {
    /// 레인의 수이다. 빈 레인도 센다.
    pub fn lane_count(&self) -> usize {
        self.lanes.len()
    }

    /// 커밋 하나를 놓고 그 줄을 돌려준다. 상태는 이 줄의 아래쪽 경계로 바뀐다.
    pub fn place(&mut self, commit: GitGraphCommit) -> GitGraphRow {
        let lanes_in = self.lanes_waiting_for(&commit.id);
        let lane = match lanes_in.first() {
            Some(lane) => *lane,
            None => self.first_free_lane(),
        };
        for waiting in &lanes_in {
            self.lanes[*waiting] = None;
        }
        let lanes_through = self.occupied_lanes();
        let lanes_out = self.assign_parents(lane, &commit.parents);
        self.trim_free_tail();
        GitGraphRow {
            id: commit.id,
            parents: commit.parents,
            author_name: commit.author_name,
            author_time: commit.author_time,
            subject: commit.subject,
            lane,
            lanes_in,
            lanes_out,
            lanes_through,
            backup_only: false,
            outside_filter: false,
        }
    }

    fn lanes_waiting_for(&self, id: &str) -> Vec<usize> {
        self.lanes
            .iter()
            .enumerate()
            .filter(|(_, next)| next.as_deref() == Some(id))
            .map(|(lane, _)| lane)
            .collect()
    }

    fn occupied_lanes(&self) -> Vec<usize> {
        self.lanes
            .iter()
            .enumerate()
            .filter(|(_, next)| next.is_some())
            .map(|(lane, _)| lane)
            .collect()
    }

    /// 가장 왼쪽의 빈 레인이다. 없으면 오른쪽에 하나를 더한다.
    fn first_free_lane(&mut self) -> usize {
        if let Some(lane) = self.lanes.iter().position(Option::is_none) {
            return lane;
        }
        self.lanes.push(None);
        self.lanes.len() - 1
    }

    /// 부모마다 레인을 정한다. 이미 그 부모를 기다리는 레인이 있으면 그 레인으로 합친다. 첫 부모는 점의 레인을
    /// 이어받는다. 그래서 한 브랜치의 선이 곧게 내려간다.
    fn assign_parents(&mut self, commit_lane: usize, parents: &[String]) -> Vec<usize> {
        let mut lanes_out = Vec::with_capacity(parents.len());
        for parent in parents {
            let lane = self.lane_for_parent(commit_lane, parent);
            if !lanes_out.contains(&lane) {
                lanes_out.push(lane);
            }
        }
        lanes_out
    }

    fn lane_for_parent(&mut self, commit_lane: usize, parent: &str) -> usize {
        if let Some(lane) = self.lanes_waiting_for(parent).first() {
            return *lane;
        }
        let lane = if self.lanes.get(commit_lane).is_some_and(Option::is_none) {
            commit_lane
        } else {
            self.first_free_lane()
        };
        self.lanes[lane] = Some(parent.to_string());
        lane
    }

    fn trim_free_tail(&mut self) {
        while self.lanes.last().is_some_and(Option::is_none) {
            self.lanes.pop();
        }
    }
}

/// 요청의 개수를 허용 범위로 맞춘다.
pub fn clamp_git_graph_page_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(GIT_GRAPH_PAGE_LIMIT_DEFAULT)
        .clamp(1, GIT_GRAPH_PAGE_LIMIT_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: &str, parents: &[&str]) -> GitGraphCommit {
        GitGraphCommit {
            id: id.to_string(),
            parents: parents.iter().map(|parent| parent.to_string()).collect(),
            author_name: "author".to_string(),
            author_time: 0,
            subject: id.to_string(),
        }
    }

    fn place_all(state: &mut GitGraphLaneState, commits: &[(&str, &[&str])]) -> Vec<GitGraphRow> {
        commits
            .iter()
            .map(|(id, parents)| state.place(commit(id, parents)))
            .collect()
    }

    #[test]
    fn linear_history_stays_in_lane_zero() {
        let mut state = GitGraphLaneState::default();
        let rows = place_all(&mut state, &[("c", &["b"]), ("b", &["a"]), ("a", &[])]);

        assert!(rows.iter().all(|row| row.lane == 0));
        assert_eq!(rows[0].lanes_in, Vec::<usize>::new());
        assert_eq!(rows[0].lanes_out, vec![0]);
        assert_eq!(rows[1].lanes_in, vec![0]);
        assert_eq!(rows[2].lanes_out, Vec::<usize>::new());
        assert_eq!(state, GitGraphLaneState::default());
    }

    #[test]
    fn second_branch_tip_takes_the_next_lane_and_joins_at_the_fork_point() {
        let mut state = GitGraphLaneState::default();
        let rows = place_all(
            &mut state,
            &[("main2", &["base"]), ("topic", &["base"]), ("base", &[])],
        );

        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[1].lanes_through, vec![0]);
        // topic 의 부모는 이미 레인 0 이 기다린다. 새 레인을 쓰지 않고 그 레인으로 합친다.
        assert_eq!(rows[1].lanes_out, vec![0]);
        assert_eq!(rows[2].lane, 0);
        assert_eq!(rows[2].lanes_in, vec![0]);
    }

    #[test]
    fn merge_commit_opens_a_lane_for_the_second_parent() {
        let mut state = GitGraphLaneState::default();
        let rows = place_all(
            &mut state,
            &[
                ("merge", &["left", "right"]),
                ("right", &["base"]),
                ("left", &["base"]),
                ("base", &[]),
            ],
        );

        assert_eq!(rows[0].lanes_out, vec![0, 1]);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[1].lanes_through, vec![0]);
        assert_eq!(rows[2].lane, 0);
        assert_eq!(rows[2].lanes_through, vec![1]);
        // right 가 먼저 나와서 base 를 레인 1 에 둔다. left 는 그 레인으로 합친다.
        assert_eq!(rows[2].lanes_out, vec![1]);
        assert_eq!(rows[3].lanes_in, vec![1]);
        assert_eq!(rows[3].lane, 1);
        assert_eq!(state, GitGraphLaneState::default());
    }

    #[test]
    fn freed_lane_is_reused_by_the_next_unrelated_tip() {
        let mut state = GitGraphLaneState::default();
        let rows = place_all(
            &mut state,
            &[("a2", &["a1"]), ("b1", &[]), ("c1", &["a1"]), ("a1", &[])],
        );

        // b1 은 부모가 없다. 레인 1 은 b1 의 줄 뒤에 비고, c1 이 다시 쓴다.
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[2].lane, 1);
        assert_eq!(rows[2].lanes_out, vec![0]);
        assert_eq!(rows[3].lanes_in, vec![0]);
    }

    #[test]
    fn layout_continues_across_a_page_boundary() {
        let history: [(&str, &[&str]); 5] = [
            ("merge", &["left", "right"]),
            ("right", &["base"]),
            ("left", &["base"]),
            ("base", &["root"]),
            ("root", &[]),
        ];
        let mut whole_state = GitGraphLaneState::default();
        let whole = place_all(&mut whole_state, &history);

        let mut paged_state = GitGraphLaneState::default();
        let mut paged = place_all(&mut paged_state, &history[..2]);
        let carried = serde_json::to_string(&paged_state).expect("serialize lane state");
        let mut resumed: GitGraphLaneState =
            serde_json::from_str(&carried).expect("deserialize lane state");
        paged.extend(place_all(&mut resumed, &history[2..]));

        assert_eq!(paged, whole);
    }

    #[test]
    fn commit_id_accepts_only_hex_of_a_sha_length() {
        assert!(is_git_commit_id(&"a".repeat(40)));
        assert!(is_git_commit_id(&"A".repeat(64)));
        // 짧은 sha 는 ref 이름으로 읽힐 수 있다.
        assert!(!is_git_commit_id("abc1234"));
        assert!(!is_git_commit_id(&"a".repeat(41)));
        assert!(!is_git_commit_id(&"g".repeat(40)));
        assert!(!is_git_commit_id("--output=x"));
        assert!(!is_git_commit_id("HEAD~1^{tree}"));
    }

    #[test]
    fn a_row_writes_the_backup_mark_only_when_it_is_set() {
        let mut state = GitGraphLaneState::default();
        let mut row = state.place(commit("a", &[]));
        let plain = serde_json::to_value(&row).expect("serialize row");
        assert!(plain.get("backupOnly").is_none());
        row.backup_only = true;
        let marked = serde_json::to_value(&row).expect("serialize row");
        assert_eq!(marked["backupOnly"], serde_json::json!(true));
    }

    #[test]
    fn page_limit_is_clamped() {
        assert_eq!(
            clamp_git_graph_page_limit(None),
            GIT_GRAPH_PAGE_LIMIT_DEFAULT
        );
        assert_eq!(clamp_git_graph_page_limit(Some(0)), 1);
        assert_eq!(
            clamp_git_graph_page_limit(Some(usize::MAX)),
            GIT_GRAPH_PAGE_LIMIT_MAX
        );
    }
}

pub mod filter;
