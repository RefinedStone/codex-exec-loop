//! 이슈의 최근 표시 결과와 조회 상한이다. Kernel journal 해석은 application service가 소유한다.
pub const ISSUE_TIMELINE_LIMIT: usize = 20;
pub const ISSUE_TIMELINE_READ_LIMIT: usize = ISSUE_TIMELINE_LIMIT + 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueTimelineProjection {
    Ready {
        entries: Vec<IssueTimelineEntry>,
        /// 현재 보존된 기록 중 표시 범위보다 앞선 사건이 더 있다.
        has_older: bool,
    },
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueTimelineEntry {
    pub sequence: i64,
    pub recorded_at: String,
    pub kind: String,
    pub summary: String,
}
