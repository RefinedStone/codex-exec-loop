use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::issue_automation::timeline::{IssueTimelineEntry, IssueTimelineProjection};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueTimelineState {
    Ready {
        entries: Vec<IssueTimelineEntryState>,
        has_older: bool,
    },
    Unavailable {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueTimelineEntryState {
    pub sequence: i64,
    pub recorded_at: String,
    pub kind: String,
    pub summary: String,
}

impl From<&IssueTimelineProjection> for IssueTimelineState {
    fn from(projection: &IssueTimelineProjection) -> Self {
        match projection {
            IssueTimelineProjection::Ready { entries, has_older } => Self::Ready {
                entries: entries.iter().map(Into::into).collect(),
                has_older: *has_older,
            },
            IssueTimelineProjection::Unavailable => Self::Unavailable {},
        }
    }
}

impl From<&IssueTimelineEntry> for IssueTimelineEntryState {
    fn from(entry: &IssueTimelineEntry) -> Self {
        Self {
            sequence: entry.sequence,
            recorded_at: entry.recorded_at.clone(),
            kind: entry.kind.clone(),
            summary: entry.summary.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_timeline_wire_distinguishes_empty_history_from_unavailable() {
        let empty = IssueTimelineState::from(&IssueTimelineProjection::Ready {
            entries: Vec::new(),
            has_older: false,
        });
        assert_eq!(
            serde_json::to_value(empty).unwrap(),
            serde_json::json!({"status": "ready", "entries": [], "has_older": false})
        );
        let unavailable = IssueTimelineState::from(&IssueTimelineProjection::Unavailable);
        assert_eq!(
            serde_json::to_value(unavailable).unwrap(),
            serde_json::json!({"status": "unavailable"})
        );
    }
}
