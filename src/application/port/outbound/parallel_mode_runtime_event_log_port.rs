use anyhow::Result;

use crate::domain::parallel_mode::ParallelModeRuntimeEventsSnapshot;

const DEFAULT_RUNTIME_EVENT_LIMIT: usize = 24;
const MAX_RUNTIME_EVENT_LIMIT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * runtime event log request는 current projection snapshot과 별도로 bounded event feed를 읽기 위한
 * application-port 입력이다. UI가 최근 이벤트 몇 개만 필요할 때 전체 runtime_events 테이블을 끌어오지
 * 않도록 limit과 projection filter를 포트 계약에 명시한다.
 */
pub struct ParallelModeRuntimeEventLogRequest {
    pub limit: usize,
    pub projection_kind: Option<String>,
    pub projection_key: Option<String>,
    pub after_sequence: Option<i64>,
}

impl ParallelModeRuntimeEventLogRequest {
    pub fn recent(limit: usize) -> Self {
        Self {
            limit,
            projection_kind: None,
            projection_key: None,
            after_sequence: None,
        }
    }

    pub fn for_projection(
        projection_kind: impl Into<String>,
        projection_key: impl Into<String>,
        limit: usize,
    ) -> Self {
        Self {
            limit,
            projection_kind: Some(projection_kind.into()),
            projection_key: Some(projection_key.into()),
            after_sequence: None,
        }
    }

    pub fn after_sequence(mut self, sequence: i64) -> Self {
        self.after_sequence = Some(sequence);
        self
    }

    pub fn bounded_limit(&self) -> usize {
        self.limit.min(MAX_RUNTIME_EVENT_LIMIT)
    }
}

impl Default for ParallelModeRuntimeEventLogRequest {
    fn default() -> Self {
        Self::recent(DEFAULT_RUNTIME_EVENT_LIMIT)
    }
}

/*
 * ParallelModeRuntimeEventLogPort는 authority store에 남은 runtime_events audit feed를 읽는 좁은
 * 경계다. application service는 SQLite row shape나 payload JSON을 알지 않고, bounded read model만
 * 받아 operator-facing timeline으로 넘길 수 있다.
 */
pub trait ParallelModeRuntimeEventLogPort: Send + Sync {
    fn load_runtime_event_log(
        &self,
        workspace_dir: &str,
        request: ParallelModeRuntimeEventLogRequest,
    ) -> Result<ParallelModeRuntimeEventsSnapshot>;
}
