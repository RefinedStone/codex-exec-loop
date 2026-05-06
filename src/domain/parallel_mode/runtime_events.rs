/*
 * runtime_events.rs는 SQLite authority store가 남기는 parallel-mode runtime event log를
 * operator-facing read model로 정리한다. 현재 slot lease/session/detail/queue row는 최신 상태만
 * 보존하므로, 이 feed는 "무엇이 어떤 순서로 바뀌었는가"를 UI와 진단 경로가 볼 수 있게 하는
 * 시간축 projection이다.
 */

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeRuntimeEventEntry {
    // Authority store가 부여한 단조 증가 순서다. recorded_at이 같아도 sequence로 정렬이 안정된다.
    pub sequence: i64,
    // slot_lease_upsert, session_detail_upsert 같은 projection 변경 종류다.
    pub event_kind: String,
    // slot_lease, session_detail, distributor_queue처럼 변경된 runtime projection 영역이다.
    pub projection_kind: String,
    // projection_kind 안에서 대상 row를 식별하는 key다. 예를 들면 slot_id 또는 session_key다.
    pub projection_key: String,
    // 이 이벤트가 기록될 때 authority store가 관측한 planning revision이다.
    pub observed_planning_revision: i64,
    // 사람이 로그를 훑을 때 바로 이해할 수 있는 한 줄 설명이다.
    pub summary: String,
    // 이벤트 기록 시각이다. 표시 계층이 그대로 보여줄 수 있게 RFC3339 문자열로 둔다.
    pub recorded_at: String,
}

impl ParallelModeRuntimeEventEntry {
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

    pub fn target_label(&self) -> String {
        format!("{}:{}", self.projection_kind, self.projection_key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeRuntimeEventsSnapshot {
    // Entries are ordered newest-first by the port adapter.
    pub entries: Vec<ParallelModeRuntimeEventEntry>,
    // Total matching event count before the request limit is applied.
    pub total_event_count: usize,
    // Copy shown when no event rows are visible.
    pub empty_state: String,
}

impl ParallelModeRuntimeEventsSnapshot {
    pub fn new(
        entries: Vec<ParallelModeRuntimeEventEntry>,
        total_event_count: usize,
        empty_state: impl Into<String>,
    ) -> Self {
        Self {
            entries,
            total_event_count,
            empty_state: empty_state.into(),
        }
    }

    pub fn empty(empty_state: impl Into<String>) -> Self {
        Self::new(Vec::new(), 0, empty_state)
    }

    pub fn latest(&self) -> Option<&ParallelModeRuntimeEventEntry> {
        self.entries.first()
    }

    pub fn visible_count(&self) -> usize {
        self.entries.len()
    }

    pub fn compact_summary(&self) -> String {
        let Some(latest) = self.latest() else {
            return self.empty_state.clone();
        };

        format!(
            "events {}/{} / latest #{} {} {}",
            self.visible_count(),
            self.total_event_count,
            latest.sequence,
            latest.event_kind,
            latest.target_label()
        )
    }
}
