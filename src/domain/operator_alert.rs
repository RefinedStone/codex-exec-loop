/*
 * Operator alerts are cross-surface notifications. The domain value keeps the
 * alert intent transport-neutral so the TUI can render a terminal bell today and
 * a later Telegram adapter can forward the same event without parsing status
 * copy.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorAlert {
    pub(crate) kind: OperatorAlertKind,
    pub(crate) title: String,
    pub(crate) detail: String,
    pub(crate) audible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperatorAlertKind {
    PlanningQueueDrained,
}

impl OperatorAlert {
    pub(crate) fn planning_queue_drained() -> Self {
        Self {
            kind: OperatorAlertKind::PlanningQueueDrained,
            title: "All planning tasks complete".to_string(),
            detail: "No actionable or proposed planning work remains.".to_string(),
            audible: true,
        }
    }

    pub(crate) fn runtime_notice(&self) -> String {
        format!("alert: {} / {}", self.title, self.detail)
    }

    pub(crate) fn transcript_banner(&self) -> String {
        format!(
            "========================================\n{}\n{}\n========================================",
            self.title.to_ascii_uppercase(),
            self.detail
        )
    }
}
