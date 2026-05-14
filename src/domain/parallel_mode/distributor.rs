use serde::{Deserialize, Serialize};

use super::{
    ParallelModeAgentRosterSnapshot, ParallelModePoolBoardSnapshot,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorState,
};

// distributor queue state는 병렬 작업이 slot 실행을 끝낸 뒤 prerelease로
// 들어가기까지의 통합 파이프라인 단계다. lease 상태와 달리 GitHub/merge 작업을 표현한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModeQueueItemState {
    Idle,
    Queued,
    Pushing,
    PrPending,
    MergePending,
    Integrating,
    Cleaning,
    Done,
    Blocked,
    Failed,
}

impl ParallelModeQueueItemState {
    // label은 TUI와 report가 공유하는 사람이 읽는 상태명이다. serialize 이름과
    // 별개로 공백을 포함할 수 있어 표시 계층의 문자열 규칙을 이곳에 고정한다.
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Queued => "queued",
            Self::Pushing => "pushing",
            Self::PrPending => "pr pending",
            Self::MergePending => "merge pending",
            Self::Integrating => "integrating",
            Self::Cleaning => "cleaning",
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
        }
    }

    // active queue는 Done/Failed처럼 terminal인 항목을 제외한 통합 압력이다.
    // Idle도 빈 자리 의미라서 depth와 blocking 계산에서 진행 중으로 세지 않는다.
    pub fn is_active(self) -> bool {
        !matches!(self, Self::Idle | Self::Done | Self::Failed)
    }
}

// completion feed는 완료 파이프라인의 짧은 사건 목록이다. 로그 원문 대신
// stage와 summary만 보존해 TUI panel이 안정적인 폭으로 최근 흐름을 보여준다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeCompletionFeedEntry {
    pub stage_label: String,
    pub summary: String,
}

impl ParallelModeCompletionFeedEntry {
    pub fn new(stage_label: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            stage_label: stage_label.into(),
            summary: summary.into(),
        }
    }
}

// runtime event feed는 authority store의 append-only orchestration log를 UI용으로 축약한 행이다.
// completion feed가 lifecycle 요약이라면 이 feed는 실제 projection write 순서와 planning revision을 보여준다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeRuntimeEventFeedEntry {
    pub sequence: i64,
    pub event_kind: String,
    pub projection_kind: String,
    pub projection_key: String,
    pub observed_planning_revision: i64,
    pub summary: String,
    pub recorded_at: String,
}

impl ParallelModeRuntimeEventFeedEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sequence: i64,
        event_kind: impl Into<String>,
        projection_kind: impl Into<String>,
        projection_key: impl Into<String>,
        observed_planning_revision: i64,
        summary: impl Into<String>,
        recorded_at: impl Into<String>,
    ) -> Self {
        Self {
            sequence,
            event_kind: event_kind.into(),
            projection_kind: projection_kind.into(),
            projection_key: projection_key.into(),
            observed_planning_revision,
            summary: summary.into(),
            recorded_at: recorded_at.into(),
        }
    }
}

// distributor queue item은 통합 대기열의 한 행이다. source agent와 branch/commit을
// 함께 둬서 supervisor가 어떤 병렬 산출물을 prerelease로 옮기는지 추적한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeDistributorQueueItem {
    pub source_agent: String,
    pub task_title: String,
    pub queue_state: ParallelModeQueueItemState,
    pub branch_name: String,
    pub commit_short_sha: String,
    pub integration_note: String,
}

impl ParallelModeDistributorQueueItem {
    pub fn new(
        source_agent: impl Into<String>,
        task_title: impl Into<String>,
        queue_state: ParallelModeQueueItemState,
        branch_name: impl Into<String>,
        commit_short_sha: impl Into<String>,
        integration_note: impl Into<String>,
    ) -> Self {
        Self {
            source_agent: source_agent.into(),
            task_title: task_title.into(),
            queue_state,
            branch_name: branch_name.into(),
            commit_short_sha: commit_short_sha.into(),
            integration_note: integration_note.into(),
        }
    }
}

// orchestrator status는 queue head 바깥의 제어면 상태다. barrier, conflict,
// integration worktree readiness처럼 대기열 행만으로 설명되지 않는 멈춤 사유를 담는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeOrchestratorStatus {
    pub queue_head: String,
    pub barrier_state: String,
    pub blocked_reason: Option<String>,
    pub conflict_files: Vec<String>,
    pub held_queue_count: usize,
    pub integration_worktree_readiness: String,
    pub slot_return_wait_reason: Option<String>,
}

impl ParallelModeOrchestratorStatus {
    // idle 기본값은 아직 통합 worktree를 검사하지 않은 상태를 명시한다. 빈 문자열 대신
    // 사람이 읽는 기본 문구를 넣어 UI가 누락값과 정상 idle을 구분하지 않아도 된다.
    pub fn idle() -> Self {
        Self {
            queue_head: "none".to_string(),
            barrier_state: "idle".to_string(),
            blocked_reason: None,
            conflict_files: Vec::new(),
            held_queue_count: 0,
            integration_worktree_readiness: "not inspected".to_string(),
            slot_return_wait_reason: None,
        }
    }
}

// distributor snapshot은 통합 파이프라인의 읽기 모델이다. queue, completion feed,
// head 부가 설명, orchestrator 상태를 한 덩어리로 묶어 supervisor 화면에 넘긴다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeDistributorSnapshot {
    pub queue_items: Vec<ParallelModeDistributorQueueItem>,
    pub completion_feed: Vec<ParallelModeCompletionFeedEntry>,
    pub runtime_event_feed: Vec<ParallelModeRuntimeEventFeedEntry>,
    pub head_summary: String,
    pub note: String,
    pub head_blocked_detail: Option<String>,
    pub head_rebase_provenance: Option<String>,
    pub orchestrator_status: ParallelModeOrchestratorStatus,
}

impl ParallelModeDistributorSnapshot {
    // 기본 생성자는 head 요약과 note만 필수로 받고 상세 blocking 정보는 builder로 붙인다.
    // producer가 없는 값은 None으로 남겨 TUI가 섣부른 빈 섹션을 만들지 않게 한다.
    pub fn new(
        queue_items: Vec<ParallelModeDistributorQueueItem>,
        completion_feed: Vec<ParallelModeCompletionFeedEntry>,
        head_summary: impl Into<String>,
        note: impl Into<String>,
    ) -> Self {
        Self {
            queue_items,
            completion_feed,
            runtime_event_feed: Vec::new(),
            head_summary: head_summary.into(),
            note: note.into(),
            head_blocked_detail: None,
            head_rebase_provenance: None,
            orchestrator_status: ParallelModeOrchestratorStatus::idle(),
        }
    }

    // 공백뿐인 detail은 없는 값으로 정규화한다. 이 타입이 한번 정리하면
    // adapter마다 trim 여부를 다시 판단하지 않아도 된다.
    pub fn with_head_blocked_detail(mut self, detail: Option<String>) -> Self {
        self.head_blocked_detail = detail.filter(|detail| !detail.trim().is_empty());
        self
    }

    // rebase provenance는 head가 어떤 기준으로 갱신됐는지 설명하는 선택 정보다.
    // 빈 문자열을 걸러 실제 표시할 근거만 snapshot에 남긴다.
    pub fn with_head_rebase_provenance(mut self, provenance: Option<String>) -> Self {
        self.head_rebase_provenance = provenance.filter(|provenance| !provenance.trim().is_empty());
        self
    }

    pub fn with_orchestrator_status(mut self, status: ParallelModeOrchestratorStatus) -> Self {
        self.orchestrator_status = status;
        self
    }

    pub fn with_runtime_event_feed(
        mut self,
        runtime_event_feed: Vec<ParallelModeRuntimeEventFeedEntry>,
    ) -> Self {
        self.runtime_event_feed = runtime_event_feed;
        self
    }

    pub fn queue_depth(&self) -> usize {
        self.queue_items.len()
    }

    // compact summary는 상단 한 줄 표시용이다. 큐가 비어 있으면 depth를 붙이지 않아
    // idle 상태의 head_summary가 불필요한 숫자 noise 없이 그대로 보인다.
    pub fn compact_summary(&self) -> String {
        if self.queue_items.is_empty() {
            return self.head_summary.clone();
        }

        format!("{} / depth {}", self.head_summary, self.queue_depth())
    }
}

// supervisor snapshot은 병렬 모드 화면의 최상위 projection이다. pool, roster,
// detail, distributor를 동시에 잡아 한 tick 안에서 서로 같은 관측 시점을 공유하게 한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeSupervisorSnapshot {
    pub state: ParallelModeSupervisorState,
    pub workspace_path: String,
    pub pool: ParallelModePoolBoardSnapshot,
    pub roster: ParallelModeAgentRosterSnapshot,
    pub detail: ParallelModeSupervisorDetailSnapshot,
    pub distributor: ParallelModeDistributorSnapshot,
    pub top_notice: Option<String>,
}

impl ParallelModeSupervisorSnapshot {
    // 최상위 projection은 adapter가 이미 만든 하위 snapshot들을 조립만 한다.
    // 여기서 새 계산을 만들지 않아 각 하위 도메인의 책임 경계를 보존한다.
    pub fn new(
        state: ParallelModeSupervisorState,
        workspace_path: impl Into<String>,
        pool: ParallelModePoolBoardSnapshot,
        roster: ParallelModeAgentRosterSnapshot,
        detail: ParallelModeSupervisorDetailSnapshot,
        distributor: ParallelModeDistributorSnapshot,
        top_notice: Option<String>,
    ) -> Self {
        Self {
            state,
            workspace_path: workspace_path.into(),
            pool,
            roster,
            detail,
            distributor,
            top_notice,
        }
    }

    pub fn state_label(&self) -> &'static str {
        self.state.label()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_item_state_labels_and_active_flags_are_stable() {
        let cases = [
            (ParallelModeQueueItemState::Idle, "idle", false),
            (ParallelModeQueueItemState::Queued, "queued", true),
            (ParallelModeQueueItemState::Pushing, "pushing", true),
            (ParallelModeQueueItemState::PrPending, "pr pending", true),
            (
                ParallelModeQueueItemState::MergePending,
                "merge pending",
                true,
            ),
            (ParallelModeQueueItemState::Integrating, "integrating", true),
            (ParallelModeQueueItemState::Cleaning, "cleaning", true),
            (ParallelModeQueueItemState::Done, "done", false),
            (ParallelModeQueueItemState::Blocked, "blocked", true),
            (ParallelModeQueueItemState::Failed, "failed", false),
        ];

        for (state, label, active) in cases {
            assert_eq!(state.label(), label);
            assert_eq!(state.is_active(), active, "{state:?}");
        }
    }

    #[test]
    fn feed_and_queue_entries_store_display_fields() {
        let completion = ParallelModeCompletionFeedEntry::new("merge", "merged branch");
        assert_eq!(completion.stage_label, "merge");
        assert_eq!(completion.summary, "merged branch");

        let runtime = ParallelModeRuntimeEventFeedEntry::new(
            42,
            "projection_written",
            "distributor_queue",
            "queue-1",
            7,
            "queue updated",
            "2026-05-14T07:00:00Z",
        );
        assert_eq!(runtime.sequence, 42);
        assert_eq!(runtime.event_kind, "projection_written");
        assert_eq!(runtime.projection_kind, "distributor_queue");
        assert_eq!(runtime.projection_key, "queue-1");
        assert_eq!(runtime.observed_planning_revision, 7);
        assert_eq!(runtime.summary, "queue updated");
        assert_eq!(runtime.recorded_at, "2026-05-14T07:00:00Z");

        let item = ParallelModeDistributorQueueItem::new(
            "agent-one",
            "Cover distributor",
            ParallelModeQueueItemState::Queued,
            "akra-agent/slot-1",
            "abc1234",
            "waiting for push",
        );
        assert_eq!(item.source_agent, "agent-one");
        assert_eq!(item.task_title, "Cover distributor");
        assert_eq!(item.queue_state, ParallelModeQueueItemState::Queued);
        assert_eq!(item.branch_name, "akra-agent/slot-1");
        assert_eq!(item.commit_short_sha, "abc1234");
        assert_eq!(item.integration_note, "waiting for push");
    }

    #[test]
    fn distributor_snapshot_filters_blank_optional_details_and_summarizes_depth() {
        let runtime_event = ParallelModeRuntimeEventFeedEntry::new(
            1,
            "event",
            "projection",
            "key",
            3,
            "summary",
            "now",
        );
        let status = ParallelModeOrchestratorStatus {
            queue_head: "queue-1".to_string(),
            barrier_state: "blocked".to_string(),
            blocked_reason: Some("conflict".to_string()),
            conflict_files: vec!["src/lib.rs".to_string()],
            held_queue_count: 2,
            integration_worktree_readiness: "dirty".to_string(),
            slot_return_wait_reason: Some("slot busy".to_string()),
        };
        let empty = ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "none")
            .with_head_blocked_detail(Some("   ".to_string()))
            .with_head_rebase_provenance(Some(String::new()));

        assert_eq!(empty.queue_depth(), 0);
        assert_eq!(empty.compact_summary(), "idle");
        assert_eq!(empty.head_blocked_detail, None);
        assert_eq!(empty.head_rebase_provenance, None);
        assert_eq!(
            empty.orchestrator_status,
            ParallelModeOrchestratorStatus::idle()
        );

        let item = ParallelModeDistributorQueueItem::new(
            "agent-one",
            "Cover distributor",
            ParallelModeQueueItemState::MergePending,
            "branch",
            "abc1234",
            "ready",
        );
        let snapshot = ParallelModeDistributorSnapshot::new(
            vec![item],
            vec![ParallelModeCompletionFeedEntry::new(
                "push",
                "pushed branch",
            )],
            "merge pending",
            "waiting",
        )
        .with_head_blocked_detail(Some(" blocked by checks ".to_string()))
        .with_head_rebase_provenance(Some("origin/prerelease".to_string()))
        .with_orchestrator_status(status.clone())
        .with_runtime_event_feed(vec![runtime_event.clone()]);

        assert_eq!(snapshot.queue_depth(), 1);
        assert_eq!(snapshot.compact_summary(), "merge pending / depth 1");
        assert_eq!(
            snapshot.head_blocked_detail.as_deref(),
            Some(" blocked by checks ")
        );
        assert_eq!(
            snapshot.head_rebase_provenance.as_deref(),
            Some("origin/prerelease")
        );
        assert_eq!(snapshot.orchestrator_status, status);
        assert_eq!(snapshot.runtime_event_feed, vec![runtime_event]);
        assert_eq!(snapshot.completion_feed[0].summary, "pushed branch");
    }

    #[test]
    fn supervisor_snapshot_keeps_child_snapshots_and_state_label() {
        let pool = ParallelModePoolBoardSnapshot::new(0, "pool", "ready", Vec::new());
        let roster = ParallelModeAgentRosterSnapshot::new(Vec::new(), "no agents");
        let detail = ParallelModeSupervisorDetailSnapshot::new(None, "no detail");
        let distributor =
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "none");

        let snapshot = ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Recover,
            "/workspace",
            pool,
            roster,
            detail,
            distributor,
            Some("readiness blocked".to_string()),
        );

        assert_eq!(snapshot.state_label(), "recover");
        assert_eq!(snapshot.workspace_path, "/workspace");
        assert_eq!(snapshot.pool.pool_root_label, "pool");
        assert_eq!(snapshot.roster.empty_state, "no agents");
        assert_eq!(snapshot.detail.empty_state, "no detail");
        assert_eq!(snapshot.distributor.head_summary, "idle");
        assert_eq!(snapshot.top_notice.as_deref(), Some("readiness blocked"));
    }
}
