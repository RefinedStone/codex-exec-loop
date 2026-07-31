use std::fmt;

use sha2::{Digest, Sha256};

use crate::domain::conversation_item_lifecycle::{
    ConversationItemKind, ConversationItemLifecycleConsistency,
    ConversationItemLifecycleObservation, ConversationItemLifecyclePhase,
    MAX_CONVERSATION_ITEM_IDENTIFIER_BYTES, MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS,
};
use crate::domain::conversation_progressive_activity::{
    ConversationProgressiveActivityObservation, ConversationProgressiveActivityPayload,
    ConversationProgressiveActivityProjectionSnapshot, ConversationProgressivePlanStepStatus,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressiveActivityItemKind {
    UserMessage,
    HookPrompt,
    AgentMessage,
    Plan,
    Reasoning,
    CommandExecution,
    FileChange,
    McpToolCall,
    DynamicToolCall,
    CollaborationAgentToolCall,
    SubAgentActivity,
    WebSearch,
    ImageView,
    Sleep,
    ImageGeneration,
    ReviewMode,
    ContextCompaction,
    Unknown,
}

impl From<&ConversationItemKind> for ProgressiveActivityItemKind {
    fn from(kind: &ConversationItemKind) -> Self {
        match kind {
            ConversationItemKind::UserMessage => Self::UserMessage,
            ConversationItemKind::HookPrompt => Self::HookPrompt,
            ConversationItemKind::AgentMessage => Self::AgentMessage,
            ConversationItemKind::Plan => Self::Plan,
            ConversationItemKind::Reasoning => Self::Reasoning,
            ConversationItemKind::CommandExecution => Self::CommandExecution,
            ConversationItemKind::FileChange => Self::FileChange,
            ConversationItemKind::McpToolCall => Self::McpToolCall,
            ConversationItemKind::DynamicToolCall => Self::DynamicToolCall,
            ConversationItemKind::CollaborationAgentToolCall => Self::CollaborationAgentToolCall,
            ConversationItemKind::SubAgentActivity => Self::SubAgentActivity,
            ConversationItemKind::WebSearch => Self::WebSearch,
            ConversationItemKind::ImageView => Self::ImageView,
            ConversationItemKind::Sleep => Self::Sleep,
            ConversationItemKind::ImageGeneration => Self::ImageGeneration,
            ConversationItemKind::EnteredReviewMode | ConversationItemKind::ExitedReviewMode => {
                Self::ReviewMode
            }
            ConversationItemKind::ContextCompaction => Self::ContextCompaction,
            ConversationItemKind::Unknown(_) => Self::Unknown,
        }
    }
}

impl ProgressiveActivityItemKind {
    const fn rail_priority(self) -> u8 {
        match self {
            Self::CommandExecution => 0,
            Self::FileChange => 1,
            Self::Plan => 2,
            Self::McpToolCall => 3,
            Self::Reasoning => 4,
            Self::CollaborationAgentToolCall => 5,
            Self::SubAgentActivity => 6,
            Self::DynamicToolCall => 7,
            Self::WebSearch => 8,
            Self::ImageGeneration => 9,
            Self::ImageView => 10,
            Self::AgentMessage => 11,
            Self::ReviewMode => 12,
            Self::ContextCompaction => 13,
            Self::Sleep => 14,
            Self::HookPrompt => 15,
            Self::UserMessage => 16,
            Self::Unknown => 17,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct ActiveProgressiveItem {
    item_id_digest: [u8; 32],
    kind: ProgressiveActivityItemKind,
    command_line_count: u64,
    patch_count: u64,
    mcp_update_count: u64,
}

impl ActiveProgressiveItem {
    const fn new(item_id_digest: [u8; 32], kind: ProgressiveActivityItemKind) -> Self {
        Self {
            item_id_digest,
            kind,
            command_line_count: 0,
            patch_count: 0,
            mcp_update_count: 0,
        }
    }

    fn reset(&mut self, kind: ProgressiveActivityItemKind) {
        self.kind = kind;
        self.command_line_count = 0;
        self.patch_count = 0;
        self.mcp_update_count = 0;
    }

    fn assign_payload(&mut self, payload: &ConversationProgressiveActivityPayload) {
        match (self.kind, payload) {
            (
                ProgressiveActivityItemKind::CommandExecution,
                ConversationProgressiveActivityPayload::CommandOutput { .. },
            ) => {
                self.command_line_count = payload.command_output_line_count().unwrap_or_default();
            }
            (
                ProgressiveActivityItemKind::FileChange,
                ConversationProgressiveActivityPayload::FileChangePatch {
                    changes,
                    omitted_change_count,
                    ..
                },
            ) => {
                self.patch_count = u64::try_from(changes.len())
                    .unwrap_or(u64::MAX)
                    .saturating_add(*omitted_change_count);
            }
            (
                ProgressiveActivityItemKind::McpToolCall,
                ConversationProgressiveActivityPayload::McpProgress { update_count, .. },
            ) => {
                self.mcp_update_count = *update_count;
            }
            _ => {}
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct ProgressiveActivityState {
    active_items: Vec<ActiveProgressiveItem>,
    retrying_summary: Option<String>,
    plan_completed_count: u64,
    plan_total_count: u64,
    plan_omitted_count: u64,
    diff_addition_count: u64,
    diff_deletion_count: u64,
    diff_hunk_count: u64,
    context_pressure_basis_points: Option<u16>,
    bounded_history: bool,
}

impl ProgressiveActivityState {
    pub(crate) fn observe_item_lifecycle(
        &mut self,
        observation: &ConversationItemLifecycleObservation,
        consistency: Option<ConversationItemLifecycleConsistency>,
    ) {
        if matches!(
            consistency,
            Some(
                ConversationItemLifecycleConsistency::Accepted
                    | ConversationItemLifecycleConsistency::SnapshotObserved
                    | ConversationItemLifecycleConsistency::CompletionWithoutStart
                    | ConversationItemLifecycleConsistency::TimestampRegression
            )
        ) {
            self.clear_turn_retrying();
        }
        match observation.phase {
            ConversationItemLifecyclePhase::Started => {
                if consistency != Some(ConversationItemLifecycleConsistency::Accepted) {
                    return;
                }
                let Some(item_id_digest) = bounded_item_id_digest(&observation.item_id) else {
                    return;
                };
                let kind = ProgressiveActivityItemKind::from(&observation.kind);
                if let Some(active_item) = self
                    .active_items
                    .iter_mut()
                    .find(|active_item| active_item.item_id_digest == item_id_digest)
                {
                    active_item.reset(kind);
                    return;
                }
                if self.active_items.len() == MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS {
                    self.bounded_history = true;
                    return;
                }
                self.active_items
                    .push(ActiveProgressiveItem::new(item_id_digest, kind));
            }
            ConversationItemLifecyclePhase::Completed => {
                if !matches!(
                    consistency,
                    Some(
                        ConversationItemLifecycleConsistency::Accepted
                            | ConversationItemLifecycleConsistency::CompletionWithoutStart
                            | ConversationItemLifecycleConsistency::TimestampRegression
                    )
                ) {
                    return;
                }
                let Some(item_id_digest) = bounded_item_id_digest(&observation.item_id) else {
                    return;
                };
                let Some(index) = self
                    .active_items
                    .iter()
                    .position(|active_item| active_item.item_id_digest == item_id_digest)
                else {
                    return;
                };
                if consistency == Some(ConversationItemLifecycleConsistency::CompletionWithoutStart)
                {
                    self.bounded_history = true;
                }
                self.active_items.swap_remove(index);
            }
            ConversationItemLifecyclePhase::SnapshotObserved => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_projection_update(
        &mut self,
        projection: &ConversationProgressiveActivityProjectionSnapshot,
        first_sequence: Option<u64>,
        last_sequence: Option<u64>,
        payload_truncation_count: u64,
        dropped_observation_count: u64,
        invalid_observation_count: u64,
        unknown_observation_count: u64,
    ) {
        self.clear_turn_retrying();
        self.bounded_history |= projection.payload_truncation_count > 0
            || projection.dropped_observation_count > 0
            || projection.invalid_observation_count > 0
            || projection.unknown_observation_count > 0
            || payload_truncation_count > 0
            || dropped_observation_count > 0
            || invalid_observation_count > 0
            || unknown_observation_count > 0;

        let (Some(first_sequence), Some(last_sequence)) = (first_sequence, last_sequence) else {
            return;
        };
        if first_sequence > last_sequence {
            return;
        }

        for record in projection.records.iter().filter(|record| {
            record.last_sequence() >= first_sequence && record.last_sequence() <= last_sequence
        }) {
            self.assign_observation(record.observation());
        }
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn record_turn_retrying(&mut self, summary: &str) {
        self.retrying_summary = Some(summary.chars().take(512).collect());
    }

    pub(crate) fn clear_turn_retrying(&mut self) {
        self.retrying_summary = None;
    }

    pub(crate) fn retrying_summary(&self) -> Option<&str> {
        self.retrying_summary.as_deref()
    }

    pub(crate) fn active_item_kind(&self) -> Option<ProgressiveActivityItemKind> {
        self.active_items
            .iter()
            .map(|active_item| active_item.kind)
            .min_by_key(|kind| kind.rail_priority())
    }

    pub(crate) fn active_terminal_count(&self) -> usize {
        self.active_items
            .iter()
            .filter(|active_item| active_item.kind == ProgressiveActivityItemKind::CommandExecution)
            .count()
    }

    pub(crate) fn command_line_count(&self) -> u64 {
        self.active_items.iter().fold(0u64, |total, active_item| {
            total.saturating_add(active_item.command_line_count)
        })
    }

    pub(crate) fn patch_count(&self) -> u64 {
        self.active_items.iter().fold(0u64, |total, active_item| {
            total.saturating_add(active_item.patch_count)
        })
    }

    pub(crate) const fn plan_completed_count(&self) -> u64 {
        self.plan_completed_count
    }

    pub(crate) const fn plan_total_count(&self) -> u64 {
        self.plan_total_count
    }

    pub(crate) const fn plan_omitted_count(&self) -> u64 {
        self.plan_omitted_count
    }

    pub(crate) const fn diff_addition_count(&self) -> u64 {
        self.diff_addition_count
    }

    pub(crate) const fn diff_deletion_count(&self) -> u64 {
        self.diff_deletion_count
    }

    pub(crate) const fn diff_hunk_count(&self) -> u64 {
        self.diff_hunk_count
    }

    pub(crate) const fn context_pressure_basis_points(&self) -> Option<u16> {
        self.context_pressure_basis_points
    }

    pub(crate) fn mcp_update_count(&self) -> u64 {
        self.active_items.iter().fold(0u64, |total, active_item| {
            total.saturating_add(active_item.mcp_update_count)
        })
    }

    pub(crate) const fn bounded_history(&self) -> bool {
        self.bounded_history
    }

    pub(crate) fn has_primary_fact(&self) -> bool {
        !self.active_items.is_empty()
            || self.command_line_count() > 0
            || self.patch_count() > 0
            || self.mcp_update_count() > 0
            || self.plan_total_count > 0
            || self.diff_addition_count > 0
            || self.diff_deletion_count > 0
            || self.diff_hunk_count > 0
    }

    fn assign_observation(&mut self, observation: &ConversationProgressiveActivityObservation) {
        if observation.kind.requires_item_identity() {
            let Some(item_id_digest) = observation
                .item_id
                .as_deref()
                .and_then(bounded_item_id_digest)
            else {
                return;
            };
            let Some(active_item) = self
                .active_items
                .iter_mut()
                .find(|active_item| active_item.item_id_digest == item_id_digest)
            else {
                return;
            };
            active_item.assign_payload(&observation.payload);
            return;
        }

        match &observation.payload {
            ConversationProgressiveActivityPayload::TurnDiff {
                addition_count,
                deletion_count,
                hunk_count,
                ..
            } => {
                self.diff_addition_count = *addition_count;
                self.diff_deletion_count = *deletion_count;
                self.diff_hunk_count = *hunk_count;
            }
            ConversationProgressiveActivityPayload::TurnPlan {
                steps,
                omitted_step_count,
                ..
            } => {
                self.plan_completed_count = steps
                    .iter()
                    .filter(|step| step.status == ConversationProgressivePlanStepStatus::Completed)
                    .count()
                    .try_into()
                    .unwrap_or(u64::MAX);
                self.plan_omitted_count = *omitted_step_count;
                self.plan_total_count = u64::try_from(steps.len())
                    .unwrap_or(u64::MAX)
                    .saturating_add(*omitted_step_count);
            }
            ConversationProgressiveActivityPayload::TokenUsage { usage } => {
                self.context_pressure_basis_points = usage.context_pressure_basis_points();
            }
            ConversationProgressiveActivityPayload::AgentMessageDelta { .. }
            | ConversationProgressiveActivityPayload::PlanDelta { .. }
            | ConversationProgressiveActivityPayload::CommandOutput { .. }
            | ConversationProgressiveActivityPayload::TerminalInteraction { .. }
            | ConversationProgressiveActivityPayload::FileChangePatch { .. }
            | ConversationProgressiveActivityPayload::McpProgress { .. }
            | ConversationProgressiveActivityPayload::ReasoningSummaryTextDelta { .. }
            | ConversationProgressiveActivityPayload::ReasoningSummaryPartAdded { .. }
            | ConversationProgressiveActivityPayload::ReasoningTextDelta { .. }
            | ConversationProgressiveActivityPayload::Moderation { .. }
            | ConversationProgressiveActivityPayload::GuardianWarning { .. }
            | ConversationProgressiveActivityPayload::Unknown { .. } => {}
        }
    }
}

fn bounded_item_id_digest(item_id: &str) -> Option<[u8; 32]> {
    if item_id.is_empty() || item_id.len() > MAX_CONVERSATION_ITEM_IDENTIFIER_BYTES {
        return None;
    }
    Some(Sha256::digest(item_id.as_bytes()).into())
}

impl fmt::Debug for ProgressiveActivityState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProgressiveActivityState")
            .field("active_item_count", &self.active_items.len())
            .field("turn_retrying", &self.retrying_summary.is_some())
            .field("active_item_kind", &self.active_item_kind())
            .field("command_line_count", &self.command_line_count())
            .field("patch_count", &self.patch_count())
            .field("plan_completed_count", &self.plan_completed_count)
            .field("plan_total_count", &self.plan_total_count)
            .field("plan_omitted_count", &self.plan_omitted_count)
            .field("diff_addition_count", &self.diff_addition_count)
            .field("diff_deletion_count", &self.diff_deletion_count)
            .field("diff_hunk_count", &self.diff_hunk_count)
            .field(
                "context_pressure_basis_points",
                &self.context_pressure_basis_points,
            )
            .field("mcp_update_count", &self.mcp_update_count())
            .field("bounded_history", &self.bounded_history)
            .finish()
    }
}

#[cfg(test)]
#[path = "progressive_activity_tests.rs"]
mod tests;
