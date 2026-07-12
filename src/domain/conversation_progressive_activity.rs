use std::fmt;
use std::sync::Arc;

pub const MAX_PROGRESSIVE_ACTIVITY_IDENTIFIER_BYTES: usize = 4 * 1024;
pub const MAX_PROGRESSIVE_ACTIVITY_PATH_BYTES: usize = 16 * 1024;
pub const MAX_PROGRESSIVE_AGENT_DRAFT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PROGRESSIVE_COMMAND_DETAIL_BYTES: usize = 64 * 1024;
pub const MAX_PROGRESSIVE_MCP_DETAIL_BYTES: usize = 64 * 1024;
pub const MAX_PROGRESSIVE_DIFF_DETAIL_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PROGRESSIVE_PLAN_STEPS: usize = 64;
pub const MAX_PROGRESSIVE_PLAN_STEP_BYTES: usize = 2 * 1024;
pub const MAX_PROGRESSIVE_PLAN_DETAIL_BYTES: usize = 128 * 1024;
pub const MAX_PROGRESSIVE_GUARDIAN_WARNING_BYTES: usize = 4 * 1024;
pub const MAX_RETAINED_PROGRESSIVE_ACTIVITY_RECORDS: usize = 64;
pub const MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct ConversationProgressiveActivityObservation {
    pub sequence: u64,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub kind: ConversationProgressiveActivityKind,
    pub payload: ConversationProgressiveActivityPayload,
}

impl ConversationProgressiveActivityObservation {
    pub fn validate(&self) -> Result<(), ConversationProgressiveActivityRejection> {
        validate_identifier("threadId", &self.thread_id)?;
        if let Some(turn_id) = self.turn_id.as_deref() {
            validate_identifier("turnId", turn_id)?;
        }
        if let Some(item_id) = self.item_id.as_deref() {
            validate_identifier("itemId", item_id)?;
        }
        if self.kind.requires_turn_identity() && self.turn_id.is_none() {
            return Err(ConversationProgressiveActivityRejection::MissingIdentity {
                field: "turnId",
            });
        }
        if self.kind.requires_item_identity() && self.item_id.is_none() {
            return Err(ConversationProgressiveActivityRejection::MissingIdentity {
                field: "itemId",
            });
        }
        if !self.kind.requires_item_identity() && self.item_id.is_some() {
            return Err(
                ConversationProgressiveActivityRejection::UnexpectedIdentity { field: "itemId" },
            );
        }
        if matches!(
            self.kind,
            ConversationProgressiveActivityKind::GuardianWarning
        ) && self.turn_id.is_some()
        {
            return Err(
                ConversationProgressiveActivityRejection::UnexpectedIdentity { field: "turnId" },
            );
        }
        if let ConversationProgressiveActivityKind::Unknown(label) = &self.kind {
            validate_identifier("method", label)?;
        }
        if !self.payload.matches_kind(&self.kind) {
            return Err(ConversationProgressiveActivityRejection::KindPayloadMismatch);
        }
        self.payload.validate()
    }

    pub fn retained_dynamic_bytes(&self) -> usize {
        self.thread_id
            .len()
            .saturating_add(self.turn_id.as_ref().map_or(0, String::len))
            .saturating_add(self.item_id.as_ref().map_or(0, String::len))
            .saturating_add(match &self.kind {
                ConversationProgressiveActivityKind::Unknown(label) => label.len(),
                _ => 0,
            })
            .saturating_add(self.payload.retained_dynamic_bytes())
    }

    pub fn source_bytes(&self) -> u64 {
        self.payload.source_bytes()
    }

    pub fn truncated_bytes(&self) -> u64 {
        self.payload.truncated_bytes()
    }

    fn same_coalescing_key(&self, other: &Self) -> bool {
        self.thread_id == other.thread_id
            && self.turn_id == other.turn_id
            && self.item_id == other.item_id
            && self.kind == other.kind
            && self.payload.coalescing_index() == other.payload.coalescing_index()
    }

    fn merge_newer(&mut self, newer: Self) {
        debug_assert!(self.same_coalescing_key(&newer));
        self.sequence = newer.sequence;
        self.payload.merge_newer(newer.payload);
    }
}

impl fmt::Debug for ConversationProgressiveActivityObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConversationProgressiveActivityObservation")
            .field("sequence", &self.sequence)
            .field("thread_id_bytes", &self.thread_id.len())
            .field("turn_id_bytes", &self.turn_id.as_ref().map(String::len))
            .field("item_id_bytes", &self.item_id.as_ref().map(String::len))
            .field("kind", &self.kind)
            .field("payload", &self.payload)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum ConversationProgressiveActivityKind {
    AgentMessageDelta,
    PlanDelta,
    CommandOutput,
    TerminalInteraction,
    FileChangePatch,
    McpProgress,
    ReasoningSummaryTextDelta,
    ReasoningSummaryPartAdded,
    ReasoningTextDelta,
    TurnDiff,
    TurnPlan,
    TokenUsage,
    Moderation,
    GuardianWarning,
    Unknown(String),
}

impl ConversationProgressiveActivityKind {
    pub const fn requires_item_identity(&self) -> bool {
        matches!(
            self,
            Self::AgentMessageDelta
                | Self::PlanDelta
                | Self::CommandOutput
                | Self::TerminalInteraction
                | Self::FileChangePatch
                | Self::McpProgress
                | Self::ReasoningSummaryTextDelta
                | Self::ReasoningSummaryPartAdded
                | Self::ReasoningTextDelta
        )
    }

    pub const fn requires_turn_identity(&self) -> bool {
        !matches!(self, Self::GuardianWarning | Self::Unknown(_))
    }
}

impl fmt::Debug for ConversationProgressiveActivityKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(label) => formatter
                .debug_tuple("Unknown")
                .field(&format_args!("<redacted:{} bytes>", label.len()))
                .finish(),
            _ => formatter.write_str(match self {
                Self::AgentMessageDelta => "AgentMessageDelta",
                Self::PlanDelta => "PlanDelta",
                Self::CommandOutput => "CommandOutput",
                Self::TerminalInteraction => "TerminalInteraction",
                Self::FileChangePatch => "FileChangePatch",
                Self::McpProgress => "McpProgress",
                Self::ReasoningSummaryTextDelta => "ReasoningSummaryTextDelta",
                Self::ReasoningSummaryPartAdded => "ReasoningSummaryPartAdded",
                Self::ReasoningTextDelta => "ReasoningTextDelta",
                Self::TurnDiff => "TurnDiff",
                Self::TurnPlan => "TurnPlan",
                Self::TokenUsage => "TokenUsage",
                Self::Moderation => "Moderation",
                Self::GuardianWarning => "GuardianWarning",
                Self::Unknown(_) => unreachable!(),
            }),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum ConversationProgressiveActivityPayload {
    AgentMessageDelta {
        phase: Option<String>,
        text: String,
        source_bytes: u64,
        truncated_bytes: u64,
    },
    PlanDelta {
        chunk_count: u64,
        source_bytes: u64,
    },
    CommandOutput {
        tail: String,
        chunk_count: u64,
        source_bytes: u64,
        newline_count: u64,
        ends_with_newline: bool,
        truncated_bytes: u64,
    },
    TerminalInteraction {
        process_id: String,
        interaction_count: u64,
        input_bytes: u64,
    },
    FileChangePatch {
        changes: Vec<ConversationProgressiveFileChange>,
        omitted_change_count: u64,
        source_bytes: u64,
        truncated_bytes: u64,
    },
    McpProgress {
        message: String,
        update_count: u64,
        source_bytes: u64,
        truncated_bytes: u64,
    },
    ReasoningSummaryTextDelta {
        summary_index: i64,
        chunk_count: u64,
        source_bytes: u64,
    },
    ReasoningSummaryPartAdded {
        summary_index: i64,
        part_count: u64,
    },
    ReasoningTextDelta {
        content_index: i64,
        chunk_count: u64,
        source_bytes: u64,
    },
    TurnDiff {
        detail: String,
        source_bytes: u64,
        line_count: u64,
        addition_count: u64,
        deletion_count: u64,
        hunk_count: u64,
        truncated_bytes: u64,
    },
    TurnPlan {
        explanation: Option<String>,
        steps: Vec<ConversationProgressivePlanStep>,
        omitted_step_count: u64,
        source_bytes: u64,
        truncated_bytes: u64,
    },
    TokenUsage {
        usage: ConversationProgressiveTokenUsage,
    },
    Moderation {
        update_count: u64,
        metadata_bytes: u64,
    },
    GuardianWarning {
        message: String,
        update_count: u64,
        source_bytes: u64,
        truncated_bytes: u64,
    },
    Unknown {
        payload_bytes: u64,
    },
}

impl ConversationProgressiveActivityPayload {
    pub fn command_output_line_count(&self) -> Option<u64> {
        match self {
            Self::CommandOutput {
                source_bytes,
                newline_count,
                ends_with_newline,
                ..
            } => Some(if *source_bytes == 0 {
                0
            } else {
                newline_count.saturating_add(u64::from(!*ends_with_newline))
            }),
            _ => None,
        }
    }

    fn matches_kind(&self, kind: &ConversationProgressiveActivityKind) -> bool {
        matches!(
            (self, kind),
            (
                Self::AgentMessageDelta { .. },
                ConversationProgressiveActivityKind::AgentMessageDelta
            ) | (
                Self::PlanDelta { .. },
                ConversationProgressiveActivityKind::PlanDelta
            ) | (
                Self::CommandOutput { .. },
                ConversationProgressiveActivityKind::CommandOutput
            ) | (
                Self::TerminalInteraction { .. },
                ConversationProgressiveActivityKind::TerminalInteraction
            ) | (
                Self::FileChangePatch { .. },
                ConversationProgressiveActivityKind::FileChangePatch
            ) | (
                Self::McpProgress { .. },
                ConversationProgressiveActivityKind::McpProgress
            ) | (
                Self::ReasoningSummaryTextDelta { .. },
                ConversationProgressiveActivityKind::ReasoningSummaryTextDelta
            ) | (
                Self::ReasoningSummaryPartAdded { .. },
                ConversationProgressiveActivityKind::ReasoningSummaryPartAdded
            ) | (
                Self::ReasoningTextDelta { .. },
                ConversationProgressiveActivityKind::ReasoningTextDelta
            ) | (
                Self::TurnDiff { .. },
                ConversationProgressiveActivityKind::TurnDiff
            ) | (
                Self::TurnPlan { .. },
                ConversationProgressiveActivityKind::TurnPlan
            ) | (
                Self::TokenUsage { .. },
                ConversationProgressiveActivityKind::TokenUsage
            ) | (
                Self::Moderation { .. },
                ConversationProgressiveActivityKind::Moderation
            ) | (
                Self::GuardianWarning { .. },
                ConversationProgressiveActivityKind::GuardianWarning
            ) | (
                Self::Unknown { .. },
                ConversationProgressiveActivityKind::Unknown(_)
            )
        )
    }

    fn validate(&self) -> Result<(), ConversationProgressiveActivityRejection> {
        match self {
            Self::AgentMessageDelta {
                phase,
                text,
                source_bytes,
                truncated_bytes,
            } => {
                if phase.as_ref().is_some_and(|value| {
                    value.is_empty() || value.len() > MAX_PROGRESSIVE_ACTIVITY_IDENTIFIER_BYTES
                }) {
                    return Err(ConversationProgressiveActivityRejection::InvalidPayload);
                }
                validate_detail(text, MAX_PROGRESSIVE_AGENT_DRAFT_BYTES)?;
                validate_source_accounting(text.len(), *source_bytes, *truncated_bytes)
            }
            Self::CommandOutput {
                tail,
                chunk_count,
                source_bytes,
                newline_count,
                ends_with_newline,
                truncated_bytes,
            } => {
                if *chunk_count == 0
                    || *newline_count > *source_bytes
                    || (*source_bytes == 0 && *ends_with_newline)
                {
                    return Err(ConversationProgressiveActivityRejection::InvalidPayload);
                }
                validate_detail(tail, MAX_PROGRESSIVE_COMMAND_DETAIL_BYTES)?;
                validate_source_accounting(tail.len(), *source_bytes, *truncated_bytes)
            }
            Self::TerminalInteraction {
                process_id,
                interaction_count,
                ..
            } => {
                if *interaction_count == 0 {
                    return Err(ConversationProgressiveActivityRejection::InvalidPayload);
                }
                validate_identifier("processId", process_id)
            }
            Self::FileChangePatch {
                changes,
                source_bytes,
                truncated_bytes,
                ..
            } => {
                if changes.len() > MAX_RETAINED_PROGRESSIVE_ACTIVITY_RECORDS {
                    return Err(ConversationProgressiveActivityRejection::InvalidPayload);
                }
                for change in changes {
                    change.validate()?;
                }
                let bytes = changes.iter().fold(0usize, |total, change| {
                    total.saturating_add(change.retained_dynamic_bytes())
                });
                if bytes > MAX_PROGRESSIVE_DIFF_DETAIL_BYTES {
                    return Err(ConversationProgressiveActivityRejection::PayloadTooLarge);
                }
                validate_source_accounting(bytes, *source_bytes, *truncated_bytes)
            }
            Self::McpProgress {
                message,
                update_count,
                source_bytes,
                truncated_bytes,
            } => {
                if *update_count == 0 {
                    return Err(ConversationProgressiveActivityRejection::InvalidPayload);
                }
                validate_detail(message, MAX_PROGRESSIVE_MCP_DETAIL_BYTES)?;
                validate_source_accounting(message.len(), *source_bytes, *truncated_bytes)
            }
            Self::TurnDiff {
                detail,
                source_bytes,
                line_count,
                addition_count,
                deletion_count,
                hunk_count,
                truncated_bytes,
            } => {
                if *line_count > *source_bytes
                    || u128::from(*addition_count)
                        + u128::from(*deletion_count)
                        + u128::from(*hunk_count)
                        > u128::from(*line_count)
                {
                    return Err(ConversationProgressiveActivityRejection::InvalidPayload);
                }
                validate_detail(detail, MAX_PROGRESSIVE_DIFF_DETAIL_BYTES)?;
                validate_source_accounting(detail.len(), *source_bytes, *truncated_bytes)
            }
            Self::TurnPlan {
                explanation,
                steps,
                source_bytes,
                truncated_bytes,
                ..
            } => {
                if steps.len() > MAX_PROGRESSIVE_PLAN_STEPS
                    || steps
                        .iter()
                        .any(|step| step.text.len() > MAX_PROGRESSIVE_PLAN_STEP_BYTES)
                {
                    return Err(ConversationProgressiveActivityRejection::PayloadTooLarge);
                }
                let bytes = explanation.as_ref().map_or(0, String::len).saturating_add(
                    steps.iter().fold(0usize, |total, step| {
                        total.saturating_add(step.retained_dynamic_bytes())
                    }),
                );
                if bytes > MAX_PROGRESSIVE_PLAN_DETAIL_BYTES {
                    return Err(ConversationProgressiveActivityRejection::PayloadTooLarge);
                }
                validate_source_accounting(bytes, *source_bytes, *truncated_bytes)
            }
            Self::TokenUsage { usage } => usage.validate(),
            Self::GuardianWarning {
                message,
                update_count,
                source_bytes,
                truncated_bytes,
            } => {
                if *update_count == 0 {
                    return Err(ConversationProgressiveActivityRejection::InvalidPayload);
                }
                validate_detail(message, MAX_PROGRESSIVE_GUARDIAN_WARNING_BYTES)?;
                validate_source_accounting(message.len(), *source_bytes, *truncated_bytes)
            }
            Self::PlanDelta { chunk_count, .. }
            | Self::ReasoningSummaryTextDelta { chunk_count, .. }
            | Self::ReasoningTextDelta { chunk_count, .. } => (*chunk_count > 0)
                .then_some(())
                .ok_or(ConversationProgressiveActivityRejection::InvalidPayload),
            Self::ReasoningSummaryPartAdded { part_count, .. } => (*part_count > 0)
                .then_some(())
                .ok_or(ConversationProgressiveActivityRejection::InvalidPayload),
            Self::Moderation { update_count, .. } => (*update_count > 0)
                .then_some(())
                .ok_or(ConversationProgressiveActivityRejection::InvalidPayload),
            Self::Unknown { .. } => Ok(()),
        }
    }

    fn validate_single_source_accounting(
        &self,
    ) -> Result<(), ConversationProgressiveActivityRejection> {
        let accounting = match self {
            Self::AgentMessageDelta {
                text,
                source_bytes,
                truncated_bytes,
                ..
            } => Some((text.len(), *source_bytes, *truncated_bytes)),
            Self::CommandOutput {
                tail,
                source_bytes,
                truncated_bytes,
                ..
            } => Some((tail.len(), *source_bytes, *truncated_bytes)),
            Self::FileChangePatch {
                changes,
                source_bytes,
                truncated_bytes,
                ..
            } => Some((
                changes.iter().fold(0usize, |total, change| {
                    total.saturating_add(change.retained_dynamic_bytes())
                }),
                *source_bytes,
                *truncated_bytes,
            )),
            Self::McpProgress {
                message,
                source_bytes,
                truncated_bytes,
                ..
            }
            | Self::GuardianWarning {
                message,
                source_bytes,
                truncated_bytes,
                ..
            } => Some((message.len(), *source_bytes, *truncated_bytes)),
            Self::TurnDiff {
                detail,
                source_bytes,
                truncated_bytes,
                ..
            } => Some((detail.len(), *source_bytes, *truncated_bytes)),
            Self::TurnPlan {
                explanation,
                steps,
                source_bytes,
                truncated_bytes,
                ..
            } => Some((
                explanation.as_ref().map_or(0, String::len).saturating_add(
                    steps.iter().fold(0usize, |total, step| {
                        total.saturating_add(step.retained_dynamic_bytes())
                    }),
                ),
                *source_bytes,
                *truncated_bytes,
            )),
            _ => None,
        };
        if accounting.is_some_and(|(retained, source, truncated)| {
            u128::from(source) != retained as u128 + u128::from(truncated)
        }) {
            return Err(ConversationProgressiveActivityRejection::InvalidPayload);
        }
        Ok(())
    }

    fn count_matches_observation_count(&self, observation_count: u64) -> bool {
        match self {
            Self::PlanDelta { chunk_count, .. }
            | Self::CommandOutput { chunk_count, .. }
            | Self::ReasoningSummaryTextDelta { chunk_count, .. }
            | Self::ReasoningTextDelta { chunk_count, .. } => *chunk_count == observation_count,
            Self::TerminalInteraction {
                interaction_count, ..
            } => *interaction_count == observation_count,
            Self::McpProgress { update_count, .. }
            | Self::Moderation { update_count, .. }
            | Self::GuardianWarning { update_count, .. } => *update_count == observation_count,
            Self::ReasoningSummaryPartAdded { part_count, .. } => *part_count == observation_count,
            _ => true,
        }
    }

    fn merge_arithmetic_is_valid(&self, newer: &Self) -> bool {
        match (self, newer) {
            (
                Self::AgentMessageDelta {
                    source_bytes,
                    truncated_bytes,
                    ..
                },
                Self::AgentMessageDelta {
                    text: newer_text,
                    source_bytes: newer_source,
                    truncated_bytes: newer_truncated,
                    ..
                },
            ) => {
                source_bytes.checked_add(*newer_source).is_some()
                    && truncated_bytes
                        .checked_add(*newer_truncated)
                        .and_then(|total| total.checked_add(newer_text.len() as u64))
                        .is_some()
            }
            (
                Self::PlanDelta {
                    chunk_count,
                    source_bytes,
                },
                Self::PlanDelta {
                    chunk_count: newer_chunks,
                    source_bytes: newer_source,
                },
            ) => {
                chunk_count.checked_add(*newer_chunks).is_some()
                    && source_bytes.checked_add(*newer_source).is_some()
            }
            (
                Self::CommandOutput {
                    tail,
                    chunk_count,
                    source_bytes,
                    newline_count,
                    truncated_bytes,
                    ..
                },
                Self::CommandOutput {
                    tail: newer_tail,
                    chunk_count: newer_chunks,
                    source_bytes: newer_source,
                    newline_count: newer_newlines,
                    truncated_bytes: newer_truncated,
                    ..
                },
            ) => {
                chunk_count.checked_add(*newer_chunks).is_some()
                    && source_bytes.checked_add(*newer_source).is_some()
                    && newline_count.checked_add(*newer_newlines).is_some()
                    && truncated_bytes
                        .checked_add(*newer_truncated)
                        .and_then(|total| {
                            total.checked_add(tail.len().saturating_add(newer_tail.len()) as u64)
                        })
                        .is_some()
            }
            (
                Self::TerminalInteraction {
                    interaction_count,
                    input_bytes,
                    ..
                },
                Self::TerminalInteraction {
                    interaction_count: newer_count,
                    input_bytes: newer_bytes,
                    ..
                },
            ) => {
                interaction_count.checked_add(*newer_count).is_some()
                    && input_bytes.checked_add(*newer_bytes).is_some()
            }
            (
                Self::McpProgress {
                    update_count,
                    source_bytes,
                    truncated_bytes,
                    ..
                }
                | Self::GuardianWarning {
                    update_count,
                    source_bytes,
                    truncated_bytes,
                    ..
                },
                Self::McpProgress {
                    update_count: newer_updates,
                    source_bytes: newer_source,
                    truncated_bytes: newer_truncated,
                    ..
                }
                | Self::GuardianWarning {
                    update_count: newer_updates,
                    source_bytes: newer_source,
                    truncated_bytes: newer_truncated,
                    ..
                },
            ) => {
                update_count.checked_add(*newer_updates).is_some()
                    && source_bytes.checked_add(*newer_source).is_some()
                    && truncated_bytes.checked_add(*newer_truncated).is_some()
            }
            (
                Self::ReasoningSummaryTextDelta {
                    chunk_count,
                    source_bytes,
                    ..
                }
                | Self::ReasoningTextDelta {
                    chunk_count,
                    source_bytes,
                    ..
                },
                Self::ReasoningSummaryTextDelta {
                    chunk_count: newer_chunks,
                    source_bytes: newer_source,
                    ..
                }
                | Self::ReasoningTextDelta {
                    chunk_count: newer_chunks,
                    source_bytes: newer_source,
                    ..
                },
            ) => {
                chunk_count.checked_add(*newer_chunks).is_some()
                    && source_bytes.checked_add(*newer_source).is_some()
            }
            (
                Self::ReasoningSummaryPartAdded { part_count, .. },
                Self::ReasoningSummaryPartAdded {
                    part_count: newer_parts,
                    ..
                },
            ) => part_count.checked_add(*newer_parts).is_some(),
            (
                Self::Moderation {
                    update_count,
                    metadata_bytes,
                },
                Self::Moderation {
                    update_count: newer_updates,
                    metadata_bytes: newer_bytes,
                },
            ) => {
                update_count.checked_add(*newer_updates).is_some()
                    && metadata_bytes.checked_add(*newer_bytes).is_some()
            }
            _ => true,
        }
    }

    fn retained_dynamic_bytes(&self) -> usize {
        match self {
            Self::AgentMessageDelta { phase, text, .. } => phase
                .as_ref()
                .map_or(0, String::len)
                .saturating_add(text.len()),
            Self::CommandOutput { tail, .. } => tail.len(),
            Self::TerminalInteraction { process_id, .. } => process_id.len(),
            Self::FileChangePatch { changes, .. } => {
                changes.iter().fold(0usize, |total, change| {
                    total.saturating_add(change.retained_dynamic_bytes())
                })
            }
            Self::McpProgress { message, .. } | Self::GuardianWarning { message, .. } => {
                message.len()
            }
            Self::TurnDiff { detail, .. } => detail.len(),
            Self::TurnPlan {
                explanation, steps, ..
            } => explanation.as_ref().map_or(0, String::len).saturating_add(
                steps.iter().fold(0usize, |total, step| {
                    total.saturating_add(step.retained_dynamic_bytes())
                }),
            ),
            _ => 0,
        }
    }

    fn source_bytes(&self) -> u64 {
        match self {
            Self::AgentMessageDelta { source_bytes, .. }
            | Self::PlanDelta { source_bytes, .. }
            | Self::CommandOutput { source_bytes, .. }
            | Self::FileChangePatch { source_bytes, .. }
            | Self::McpProgress { source_bytes, .. }
            | Self::ReasoningSummaryTextDelta { source_bytes, .. }
            | Self::ReasoningTextDelta { source_bytes, .. }
            | Self::TurnDiff { source_bytes, .. }
            | Self::TurnPlan { source_bytes, .. }
            | Self::GuardianWarning { source_bytes, .. } => *source_bytes,
            Self::TerminalInteraction { input_bytes, .. } => *input_bytes,
            Self::Moderation { metadata_bytes, .. }
            | Self::Unknown {
                payload_bytes: metadata_bytes,
            } => *metadata_bytes,
            _ => 0,
        }
    }

    fn truncated_bytes(&self) -> u64 {
        match self {
            Self::AgentMessageDelta {
                truncated_bytes, ..
            }
            | Self::CommandOutput {
                truncated_bytes, ..
            }
            | Self::FileChangePatch {
                truncated_bytes, ..
            }
            | Self::McpProgress {
                truncated_bytes, ..
            }
            | Self::TurnDiff {
                truncated_bytes, ..
            }
            | Self::TurnPlan {
                truncated_bytes, ..
            }
            | Self::GuardianWarning {
                truncated_bytes, ..
            } => *truncated_bytes,
            _ => 0,
        }
    }

    fn coalescing_index(&self) -> Option<i64> {
        match self {
            Self::ReasoningSummaryTextDelta { summary_index, .. }
            | Self::ReasoningSummaryPartAdded { summary_index, .. } => Some(*summary_index),
            Self::ReasoningTextDelta { content_index, .. } => Some(*content_index),
            _ => None,
        }
    }

    fn merge_newer(&mut self, newer: Self) {
        match (self, newer) {
            (
                Self::AgentMessageDelta {
                    phase,
                    text,
                    source_bytes,
                    truncated_bytes,
                },
                Self::AgentMessageDelta {
                    phase: newer_phase,
                    text: newer_text,
                    source_bytes: newer_source,
                    truncated_bytes: newer_truncated,
                },
            ) => {
                if newer_phase.is_some() {
                    *phase = newer_phase;
                }
                *source_bytes = source_bytes.saturating_add(newer_source);
                *truncated_bytes = truncated_bytes.saturating_add(newer_truncated);
                *truncated_bytes = truncated_bytes.saturating_add(append_prefix_bounded(
                    text,
                    &newer_text,
                    MAX_PROGRESSIVE_AGENT_DRAFT_BYTES,
                ));
            }
            (
                Self::PlanDelta {
                    chunk_count,
                    source_bytes,
                },
                Self::PlanDelta {
                    chunk_count: newer_chunks,
                    source_bytes: newer_source,
                },
            ) => {
                *chunk_count = chunk_count.saturating_add(newer_chunks);
                *source_bytes = source_bytes.saturating_add(newer_source);
            }
            (
                Self::CommandOutput {
                    tail,
                    chunk_count,
                    source_bytes,
                    newline_count,
                    ends_with_newline,
                    truncated_bytes,
                },
                Self::CommandOutput {
                    tail: newer_tail,
                    chunk_count: newer_chunks,
                    source_bytes: newer_source,
                    newline_count: newer_newlines,
                    ends_with_newline: newer_ends_with_newline,
                    truncated_bytes: newer_truncated,
                },
            ) => {
                *chunk_count = chunk_count.saturating_add(newer_chunks);
                *source_bytes = source_bytes.saturating_add(newer_source);
                *newline_count = newline_count.saturating_add(newer_newlines);
                if newer_source > 0 {
                    *ends_with_newline = newer_ends_with_newline;
                }
                *truncated_bytes = truncated_bytes.saturating_add(newer_truncated);
                *truncated_bytes = truncated_bytes.saturating_add(append_tail_bounded(
                    tail,
                    &newer_tail,
                    MAX_PROGRESSIVE_COMMAND_DETAIL_BYTES,
                ));
            }
            (
                Self::TerminalInteraction {
                    process_id,
                    interaction_count,
                    input_bytes,
                },
                Self::TerminalInteraction {
                    process_id: newer_process,
                    interaction_count: newer_count,
                    input_bytes: newer_bytes,
                },
            ) => {
                *process_id = newer_process;
                *interaction_count = interaction_count.saturating_add(newer_count);
                *input_bytes = input_bytes.saturating_add(newer_bytes);
            }
            (
                Self::McpProgress {
                    message,
                    update_count,
                    source_bytes,
                    truncated_bytes,
                },
                Self::McpProgress {
                    message: newer_message,
                    update_count: newer_updates,
                    source_bytes: newer_source,
                    truncated_bytes: newer_truncated,
                },
            )
            | (
                Self::GuardianWarning {
                    message,
                    update_count,
                    source_bytes,
                    truncated_bytes,
                },
                Self::GuardianWarning {
                    message: newer_message,
                    update_count: newer_updates,
                    source_bytes: newer_source,
                    truncated_bytes: newer_truncated,
                },
            ) => {
                *message = newer_message;
                *update_count = update_count.saturating_add(newer_updates);
                *source_bytes = source_bytes.saturating_add(newer_source);
                *truncated_bytes = truncated_bytes.saturating_add(newer_truncated);
            }
            (
                Self::ReasoningSummaryTextDelta {
                    chunk_count,
                    source_bytes,
                    ..
                },
                Self::ReasoningSummaryTextDelta {
                    chunk_count: newer_chunks,
                    source_bytes: newer_source,
                    ..
                },
            )
            | (
                Self::ReasoningTextDelta {
                    chunk_count,
                    source_bytes,
                    ..
                },
                Self::ReasoningTextDelta {
                    chunk_count: newer_chunks,
                    source_bytes: newer_source,
                    ..
                },
            ) => {
                *chunk_count = chunk_count.saturating_add(newer_chunks);
                *source_bytes = source_bytes.saturating_add(newer_source);
            }
            (
                Self::ReasoningSummaryPartAdded { part_count, .. },
                Self::ReasoningSummaryPartAdded {
                    part_count: newer_parts,
                    ..
                },
            ) => *part_count = part_count.saturating_add(newer_parts),
            (
                Self::Moderation {
                    update_count,
                    metadata_bytes,
                },
                Self::Moderation {
                    update_count: newer_updates,
                    metadata_bytes: newer_bytes,
                },
            ) => {
                *update_count = update_count.saturating_add(newer_updates);
                *metadata_bytes = metadata_bytes.saturating_add(newer_bytes);
            }
            (current, newer) => *current = newer,
        }
    }

    fn variant_label(&self) -> &'static str {
        match self {
            Self::AgentMessageDelta { .. } => "agent-message-delta",
            Self::PlanDelta { .. } => "plan-delta",
            Self::CommandOutput { .. } => "command-output",
            Self::TerminalInteraction { .. } => "terminal-interaction",
            Self::FileChangePatch { .. } => "file-change-patch",
            Self::McpProgress { .. } => "mcp-progress",
            Self::ReasoningSummaryTextDelta { .. } => "reasoning-summary-text-delta",
            Self::ReasoningSummaryPartAdded { .. } => "reasoning-summary-part-added",
            Self::ReasoningTextDelta { .. } => "reasoning-text-delta",
            Self::TurnDiff { .. } => "turn-diff",
            Self::TurnPlan { .. } => "turn-plan",
            Self::TokenUsage { .. } => "token-usage",
            Self::Moderation { .. } => "moderation",
            Self::GuardianWarning { .. } => "guardian-warning",
            Self::Unknown { .. } => "unknown",
        }
    }
}

impl fmt::Debug for ConversationProgressiveActivityPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConversationProgressiveActivityPayload")
            .field("variant", &self.variant_label())
            .field("source_bytes", &self.source_bytes())
            .field("retained_dynamic_bytes", &self.retained_dynamic_bytes())
            .field("truncated_bytes", &self.truncated_bytes())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ConversationProgressiveFileChange {
    pub path: String,
    pub diff: String,
    pub kind: ConversationProgressiveFileChangeKind,
}

impl ConversationProgressiveFileChange {
    fn validate(&self) -> Result<(), ConversationProgressiveActivityRejection> {
        if self.path.is_empty() || self.path.len() > MAX_PROGRESSIVE_ACTIVITY_PATH_BYTES {
            return Err(ConversationProgressiveActivityRejection::InvalidPayload);
        }
        validate_detail(&self.diff, MAX_PROGRESSIVE_DIFF_DETAIL_BYTES)
    }

    fn retained_dynamic_bytes(&self) -> usize {
        self.path.len().saturating_add(self.diff.len())
    }
}

impl fmt::Debug for ConversationProgressiveFileChange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConversationProgressiveFileChange")
            .field("kind", &self.kind)
            .field("path_bytes", &self.path.len())
            .field("diff_bytes", &self.diff.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationProgressiveFileChangeKind {
    Add,
    Delete,
    Update { move_path_present: bool },
}

#[derive(Clone, PartialEq, Eq)]
pub struct ConversationProgressivePlanStep {
    pub status: ConversationProgressivePlanStepStatus,
    pub text: String,
}

impl ConversationProgressivePlanStep {
    fn retained_dynamic_bytes(&self) -> usize {
        self.text.len()
    }
}

impl fmt::Debug for ConversationProgressivePlanStep {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConversationProgressivePlanStep")
            .field("status", &self.status)
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationProgressivePlanStepStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversationProgressiveTokenUsage {
    pub last: ConversationProgressiveTokenUsageBreakdown,
    pub total: ConversationProgressiveTokenUsageBreakdown,
    pub model_context_window: Option<u64>,
}

impl ConversationProgressiveTokenUsage {
    fn validate(&self) -> Result<(), ConversationProgressiveActivityRejection> {
        if self.model_context_window == Some(0) {
            return Err(ConversationProgressiveActivityRejection::InvalidPayload);
        }
        self.last.validate()?;
        self.total.validate()?;
        if !self.total.includes(&self.last) {
            return Err(ConversationProgressiveActivityRejection::InvalidPayload);
        }
        Ok(())
    }

    pub fn context_pressure_basis_points(&self) -> Option<u16> {
        let window = self.model_context_window.filter(|window| *window > 0)?;
        let basis_points = u128::from(self.last.total_tokens.min(window)).saturating_mul(10_000)
            / u128::from(window);
        u16::try_from(basis_points).ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversationProgressiveTokenUsageBreakdown {
    pub cached_input_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
}

impl ConversationProgressiveTokenUsageBreakdown {
    fn validate(&self) -> Result<(), ConversationProgressiveActivityRejection> {
        if self.cached_input_tokens > self.input_tokens
            || self.reasoning_output_tokens > self.output_tokens
            || u128::from(self.total_tokens)
                != u128::from(self.input_tokens) + u128::from(self.output_tokens)
        {
            return Err(ConversationProgressiveActivityRejection::InvalidPayload);
        }
        Ok(())
    }

    const fn includes(&self, latest: &Self) -> bool {
        self.cached_input_tokens >= latest.cached_input_tokens
            && self.input_tokens >= latest.input_tokens
            && self.output_tokens >= latest.output_tokens
            && self.reasoning_output_tokens >= latest.reasoning_output_tokens
            && self.total_tokens >= latest.total_tokens
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ConversationProgressiveActivityRecord {
    first_sequence: u64,
    last_sequence: u64,
    observation_count: u64,
    observation: ConversationProgressiveActivityObservation,
}

impl ConversationProgressiveActivityRecord {
    pub const fn first_sequence(&self) -> u64 {
        self.first_sequence
    }

    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    pub const fn observation_count(&self) -> u64 {
        self.observation_count
    }

    pub const fn observation(&self) -> &ConversationProgressiveActivityObservation {
        &self.observation
    }
}

impl fmt::Debug for ConversationProgressiveActivityRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConversationProgressiveActivityRecord")
            .field("first_sequence", &self.first_sequence)
            .field("last_sequence", &self.last_sequence)
            .field("observation_count", &self.observation_count)
            .field("observation", &self.observation)
            .finish()
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct ConversationProgressiveActivityBatch {
    thread_id: String,
    turn_id: Option<String>,
    records: Vec<ConversationProgressiveActivityRecord>,
    first_sequence: Option<u64>,
    last_sequence: Option<u64>,
    source_observation_count: u64,
    coalesced_observation_count: u64,
    superseded_publication_count: u64,
    payload_truncation_count: u64,
    dropped_observation_count: u64,
    invalid_observation_count: u64,
    unknown_observation_count: u64,
}

impl ConversationProgressiveActivityBatch {
    pub fn single(
        observation: ConversationProgressiveActivityObservation,
    ) -> Result<Self, ConversationProgressiveActivityRejection> {
        observation.validate()?;
        observation.payload.validate_single_source_accounting()?;
        if !observation.payload.count_matches_observation_count(1) {
            return Err(ConversationProgressiveActivityRejection::InvalidPayload);
        }
        let sequence = observation.sequence;
        let thread_id = observation.thread_id.clone();
        let turn_id = observation.turn_id.clone();
        let payload_truncation_count = u64::from(observation.truncated_bytes() > 0);
        let unknown_observation_count = u64::from(matches!(
            observation.kind,
            ConversationProgressiveActivityKind::Unknown(_)
        ));
        Ok(Self {
            thread_id,
            turn_id,
            records: vec![ConversationProgressiveActivityRecord {
                first_sequence: sequence,
                last_sequence: sequence,
                observation_count: 1,
                observation,
            }],
            first_sequence: Some(sequence),
            last_sequence: Some(sequence),
            source_observation_count: 1,
            payload_truncation_count,
            unknown_observation_count,
            ..Self::default()
        })
    }

    pub fn validate(&self) -> Result<(), ConversationProgressiveActivityRejection> {
        if self.source_observation_count == 0 {
            if !self.thread_id.is_empty()
                || self.turn_id.is_some()
                || !self.records.is_empty()
                || self.first_sequence.is_some()
                || self.last_sequence.is_some()
                || self.coalesced_observation_count > 0
                || self.superseded_publication_count > 0
                || self.payload_truncation_count > 0
                || self.dropped_observation_count > 0
                || self.invalid_observation_count > 0
                || self.unknown_observation_count > 0
            {
                return Err(ConversationProgressiveActivityRejection::InvalidBatch);
            }
            return Ok(());
        }

        validate_identifier("threadId", &self.thread_id)?;
        if let Some(turn_id) = self.turn_id.as_deref() {
            validate_identifier("turnId", turn_id)?;
        }
        let (Some(first_sequence), Some(last_sequence)) = (self.first_sequence, self.last_sequence)
        else {
            return Err(ConversationProgressiveActivityRejection::InvalidBatch);
        };
        if first_sequence > last_sequence
            || u128::from(self.source_observation_count)
                > u128::from(last_sequence)
                    .saturating_sub(u128::from(first_sequence))
                    .saturating_add(1)
        {
            return Err(ConversationProgressiveActivityRejection::InvalidBatch);
        }
        if self.records.len() > MAX_RETAINED_PROGRESSIVE_ACTIVITY_RECORDS
            || self.retained_dynamic_bytes() > MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES
        {
            return Err(ConversationProgressiveActivityRejection::PayloadTooLarge);
        }
        if self.records.is_empty() {
            return (self.dropped_observation_count == self.source_observation_count)
                .then_some(())
                .ok_or(ConversationProgressiveActivityRejection::InvalidBatch);
        }

        let mut retained_observation_count = 0u128;
        let mut previous_last_sequence = None;
        for (index, record) in self.records.iter().enumerate() {
            record.observation.validate()?;
            if record.observation.thread_id != self.thread_id
                || record
                    .observation
                    .turn_id
                    .as_deref()
                    .is_some_and(|turn_id| self.turn_id.as_deref() != Some(turn_id))
                || (self.turn_id.is_none() && record.observation.turn_id.is_some())
                || record.observation.sequence != record.last_sequence
                || record.observation_count == 0
                || !record
                    .observation
                    .payload
                    .count_matches_observation_count(record.observation_count)
                || record.first_sequence > record.last_sequence
                || record.first_sequence < first_sequence
                || record.last_sequence > last_sequence
                || u128::from(record.observation_count)
                    > u128::from(record.last_sequence)
                        .saturating_sub(u128::from(record.first_sequence))
                        .saturating_add(1)
                || previous_last_sequence.is_some_and(|previous| previous >= record.last_sequence)
                || self.records[..index].iter().any(|previous| {
                    previous
                        .observation
                        .same_coalescing_key(&record.observation)
                })
            {
                return Err(ConversationProgressiveActivityRejection::InvalidBatch);
            }
            retained_observation_count += u128::from(record.observation_count);
            previous_last_sequence = Some(record.last_sequence);
        }
        if retained_observation_count + u128::from(self.dropped_observation_count)
            != u128::from(self.source_observation_count)
            || (self.records.last().map(|record| record.last_sequence) != Some(last_sequence)
                && self.dropped_observation_count == 0)
        {
            return Err(ConversationProgressiveActivityRejection::InvalidBatch);
        }
        Ok(())
    }

    pub(crate) fn try_merge_from(
        &mut self,
        newer: Self,
    ) -> Result<(), ConversationProgressiveActivityRejection> {
        self.validate_merge_candidate(&newer)?;
        if self.source_observation_count == 0 {
            *self = newer;
            return Ok(());
        }
        if newer.source_observation_count == 0 {
            return Ok(());
        }
        if self.turn_id.is_none() {
            self.turn_id.clone_from(&newer.turn_id);
        }
        self.first_sequence = min_optional(self.first_sequence, newer.first_sequence);
        self.last_sequence = max_optional(self.last_sequence, newer.last_sequence);
        self.source_observation_count = self
            .source_observation_count
            .saturating_add(newer.source_observation_count);
        self.coalesced_observation_count = self
            .coalesced_observation_count
            .saturating_add(newer.coalesced_observation_count);
        self.superseded_publication_count = self
            .superseded_publication_count
            .saturating_add(newer.superseded_publication_count);
        self.payload_truncation_count = self
            .payload_truncation_count
            .saturating_add(newer.payload_truncation_count);
        self.dropped_observation_count = self
            .dropped_observation_count
            .saturating_add(newer.dropped_observation_count);
        self.invalid_observation_count = self
            .invalid_observation_count
            .saturating_add(newer.invalid_observation_count);
        self.unknown_observation_count = self
            .unknown_observation_count
            .saturating_add(newer.unknown_observation_count);
        for record in newer.records {
            if merge_record(
                &mut self.records,
                record,
                &mut self.coalesced_observation_count,
            ) {
                self.payload_truncation_count = self.payload_truncation_count.saturating_add(1);
            }
        }
        enforce_record_bounds(
            &mut self.records,
            &mut self.dropped_observation_count,
            self.thread_id
                .len()
                .saturating_add(self.turn_id.as_ref().map_or(0, String::len)),
        );
        debug_assert!(self.validate().is_ok());
        Ok(())
    }

    pub(crate) fn validate_merge_candidate(
        &self,
        newer: &Self,
    ) -> Result<(), ConversationProgressiveActivityRejection> {
        self.validate()?;
        newer.validate()?;
        if self.source_observation_count == 0 {
            return Ok(());
        }
        if newer.source_observation_count == 0 {
            return Ok(());
        }
        if newer
            .first_sequence
            .is_none_or(|first| self.last_sequence.is_some_and(|last| first <= last))
        {
            return Err(ConversationProgressiveActivityRejection::NonMonotonicBatch);
        }
        if self.thread_id != newer.thread_id
            || self
                .turn_id
                .as_deref()
                .zip(newer.turn_id.as_deref())
                .is_some_and(|(current, incoming)| current != incoming)
        {
            return Err(ConversationProgressiveActivityRejection::MixedCorrelation);
        }
        self.validate_merge_arithmetic(newer)?;
        Ok(())
    }

    pub(crate) fn validate_publication_merge_candidate(
        &self,
        newer: &Self,
    ) -> Result<(), ConversationProgressiveActivityRejection> {
        self.validate_merge_candidate(newer)?;
        self.superseded_publication_count
            .checked_add(newer.superseded_publication_count)
            .and_then(|total| total.checked_add(1))
            .ok_or(ConversationProgressiveActivityRejection::InvalidBatch)
            .map(|_| ())
    }

    fn validate_merge_arithmetic(
        &self,
        newer: &Self,
    ) -> Result<(), ConversationProgressiveActivityRejection> {
        let matched_records = validate_record_merge_arithmetic(&self.records, &newer.records)?;
        let source_observation_count = self
            .source_observation_count
            .checked_add(newer.source_observation_count)
            .ok_or(ConversationProgressiveActivityRejection::InvalidBatch)?;
        let maximum_dropped_observation_count = self
            .dropped_observation_count
            .checked_add(newer.dropped_observation_count)
            .and_then(|total| checked_record_observation_count(&self.records, total))
            .and_then(|total| checked_record_observation_count(&newer.records, total))
            .ok_or(ConversationProgressiveActivityRejection::InvalidBatch)?;
        if maximum_dropped_observation_count != source_observation_count {
            return Err(ConversationProgressiveActivityRejection::InvalidBatch);
        }
        let checked = self
            .coalesced_observation_count
            .checked_add(newer.coalesced_observation_count)
            .and_then(|total| total.checked_add(matched_records))
            .and_then(|_| {
                self.superseded_publication_count
                    .checked_add(newer.superseded_publication_count)
            })
            .and_then(|_| {
                self.payload_truncation_count
                    .checked_add(newer.payload_truncation_count)
            })
            .and_then(|total| total.checked_add(matched_records))
            .and_then(|_| {
                self.invalid_observation_count
                    .checked_add(newer.invalid_observation_count)
            })
            .and_then(|_| {
                self.unknown_observation_count
                    .checked_add(newer.unknown_observation_count)
            });
        if checked.is_none() {
            return Err(ConversationProgressiveActivityRejection::InvalidBatch);
        }
        Ok(())
    }

    fn validate_projection_arithmetic(
        snapshot: &ConversationProgressiveActivityProjectionSnapshot,
        batch: &Self,
    ) -> Result<(), ConversationProgressiveActivityRejection> {
        let matched_records = validate_record_merge_arithmetic(&snapshot.records, &batch.records)?;
        let source_observation_count = snapshot
            .source_observation_count
            .checked_add(batch.source_observation_count)
            .ok_or(ConversationProgressiveActivityRejection::InvalidBatch)?;
        let maximum_dropped_observation_count = snapshot
            .dropped_observation_count
            .checked_add(batch.dropped_observation_count)
            .and_then(|total| checked_record_observation_count(&snapshot.records, total))
            .and_then(|total| checked_record_observation_count(&batch.records, total))
            .ok_or(ConversationProgressiveActivityRejection::InvalidBatch)?;
        if maximum_dropped_observation_count != source_observation_count {
            return Err(ConversationProgressiveActivityRejection::InvalidBatch);
        }
        let checked = snapshot
            .coalesced_observation_count
            .checked_add(batch.coalesced_observation_count)
            .and_then(|total| total.checked_add(matched_records))
            .and_then(|_| {
                snapshot
                    .superseded_publication_count
                    .checked_add(batch.superseded_publication_count)
            })
            .and_then(|_| {
                snapshot
                    .payload_truncation_count
                    .checked_add(batch.payload_truncation_count)
            })
            .and_then(|total| total.checked_add(matched_records))
            .and_then(|_| {
                snapshot
                    .invalid_observation_count
                    .checked_add(batch.invalid_observation_count)
            })
            .and_then(|_| {
                snapshot
                    .unknown_observation_count
                    .checked_add(batch.unknown_observation_count)
            });
        if checked.is_none() {
            return Err(ConversationProgressiveActivityRejection::InvalidBatch);
        }
        Ok(())
    }

    pub(crate) fn record_superseded_publication(
        &mut self,
    ) -> Result<(), ConversationProgressiveActivityRejection> {
        self.superseded_publication_count = self
            .superseded_publication_count
            .checked_add(1)
            .ok_or(ConversationProgressiveActivityRejection::InvalidBatch)?;
        Ok(())
    }

    pub(crate) fn discard_retained_records(&mut self) {
        let discarded = self.records.iter().fold(0u64, |total, record| {
            total.saturating_add(record.observation_count)
        });
        self.records.clear();
        self.dropped_observation_count = self.dropped_observation_count.saturating_add(discarded);
        debug_assert!(self.validate().is_ok());
    }

    pub fn thread_id(&self) -> Option<&str> {
        (self.source_observation_count > 0).then_some(self.thread_id.as_str())
    }

    pub fn turn_id(&self) -> Option<&str> {
        self.turn_id.as_deref()
    }

    pub fn records(&self) -> &[ConversationProgressiveActivityRecord] {
        &self.records
    }

    pub const fn first_sequence(&self) -> Option<u64> {
        self.first_sequence
    }

    pub const fn last_sequence(&self) -> Option<u64> {
        self.last_sequence
    }

    pub const fn source_observation_count(&self) -> u64 {
        self.source_observation_count
    }

    pub const fn coalesced_observation_count(&self) -> u64 {
        self.coalesced_observation_count
    }

    pub const fn superseded_publication_count(&self) -> u64 {
        self.superseded_publication_count
    }

    pub const fn payload_truncation_count(&self) -> u64 {
        self.payload_truncation_count
    }

    pub const fn dropped_observation_count(&self) -> u64 {
        self.dropped_observation_count
    }

    pub const fn loss_event_count(&self) -> u64 {
        self.payload_truncation_count
            .saturating_add(self.dropped_observation_count)
    }

    pub const fn invalid_observation_count(&self) -> u64 {
        self.invalid_observation_count
    }

    pub const fn unknown_observation_count(&self) -> u64 {
        self.unknown_observation_count
    }

    #[cfg(test)]
    pub(crate) fn set_incomplete_counters_for_test(
        &mut self,
        coalesced: u64,
        payload_truncations: u64,
        invalid: u64,
        unknown: u64,
    ) {
        self.coalesced_observation_count = coalesced;
        self.payload_truncation_count = payload_truncations;
        self.invalid_observation_count = invalid;
        self.unknown_observation_count = unknown;
    }

    pub fn retained_dynamic_bytes(&self) -> usize {
        self.thread_id
            .len()
            .saturating_add(self.turn_id.as_ref().map_or(0, String::len))
            .saturating_add(retained_record_bytes(&self.records))
    }

    pub fn history_incomplete(&self) -> bool {
        self.superseded_publication_count > 0
            || self.payload_truncation_count > 0
            || self.dropped_observation_count > 0
            || self.invalid_observation_count > 0
            || self.unknown_observation_count > 0
    }
}

impl fmt::Debug for ConversationProgressiveActivityBatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConversationProgressiveActivityBatch")
            .field("thread_id_bytes", &self.thread_id.len())
            .field("turn_id_bytes", &self.turn_id.as_ref().map(String::len))
            .field("records", &self.records)
            .field("first_sequence", &self.first_sequence)
            .field("last_sequence", &self.last_sequence)
            .field("source_observation_count", &self.source_observation_count)
            .field(
                "coalesced_observation_count",
                &self.coalesced_observation_count,
            )
            .field(
                "superseded_publication_count",
                &self.superseded_publication_count,
            )
            .field("payload_truncation_count", &self.payload_truncation_count)
            .field("dropped_observation_count", &self.dropped_observation_count)
            .field("invalid_observation_count", &self.invalid_observation_count)
            .field("unknown_observation_count", &self.unknown_observation_count)
            .field("retained_dynamic_bytes", &self.retained_dynamic_bytes())
            .finish()
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct ConversationProgressiveActivityProjectionSnapshot {
    pub records: Vec<ConversationProgressiveActivityRecord>,
    pub last_sequence: Option<u64>,
    pub source_observation_count: u64,
    pub coalesced_observation_count: u64,
    pub superseded_publication_count: u64,
    pub payload_truncation_count: u64,
    pub dropped_observation_count: u64,
    pub invalid_observation_count: u64,
    pub unknown_observation_count: u64,
}

impl ConversationProgressiveActivityProjectionSnapshot {
    pub fn retained_dynamic_bytes(&self) -> usize {
        retained_record_bytes(&self.records)
    }

    pub fn history_incomplete(&self) -> bool {
        self.superseded_publication_count > 0
            || self.payload_truncation_count > 0
            || self.dropped_observation_count > 0
            || self.invalid_observation_count > 0
            || self.unknown_observation_count > 0
    }

    pub const fn loss_event_count(&self) -> u64 {
        self.payload_truncation_count
            .saturating_add(self.dropped_observation_count)
    }
}

impl fmt::Debug for ConversationProgressiveActivityProjectionSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConversationProgressiveActivityProjectionSnapshot")
            .field("records", &self.records)
            .field("last_sequence", &self.last_sequence)
            .field("source_observation_count", &self.source_observation_count)
            .field(
                "coalesced_observation_count",
                &self.coalesced_observation_count,
            )
            .field(
                "superseded_publication_count",
                &self.superseded_publication_count,
            )
            .field("payload_truncation_count", &self.payload_truncation_count)
            .field("dropped_observation_count", &self.dropped_observation_count)
            .field("invalid_observation_count", &self.invalid_observation_count)
            .field("unknown_observation_count", &self.unknown_observation_count)
            .field("retained_dynamic_bytes", &self.retained_dynamic_bytes())
            .finish()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationProgressiveActivityProjection {
    state: Arc<ConversationProgressiveActivityProjectionSnapshot>,
}

impl ConversationProgressiveActivityProjection {
    pub fn apply_batch_correlated(
        &mut self,
        expected_thread_id: Option<&str>,
        expected_turn_id: Option<&str>,
        batch: ConversationProgressiveActivityBatch,
    ) -> Result<(), ConversationProgressiveActivityRejection> {
        if let Err(rejection) = batch.validate() {
            self.record_invalid();
            return Err(rejection);
        }
        if batch.source_observation_count == 0 {
            return Ok(());
        }
        if let Some(last_sequence) = self.state.last_sequence
            && batch
                .first_sequence
                .is_some_and(|first_sequence| first_sequence <= last_sequence)
        {
            self.record_invalid();
            return Err(ConversationProgressiveActivityRejection::StaleSequence { last_sequence });
        }
        if expected_thread_id != Some(batch.thread_id.as_str()) {
            self.record_invalid();
            return Err(ConversationProgressiveActivityRejection::ThreadMismatch);
        }
        if let Some(turn_id) = batch.turn_id.as_deref()
            && expected_turn_id != Some(turn_id)
        {
            self.record_invalid();
            return Err(ConversationProgressiveActivityRejection::TurnMismatch);
        }
        if let Err(rejection) = ConversationProgressiveActivityBatch::validate_projection_arithmetic(
            &self.state,
            &batch,
        ) {
            self.record_invalid();
            return Err(rejection);
        }

        let state = Arc::make_mut(&mut self.state);
        state.last_sequence = max_optional(state.last_sequence, batch.last_sequence);
        state.source_observation_count = state
            .source_observation_count
            .saturating_add(batch.source_observation_count);
        state.coalesced_observation_count = state
            .coalesced_observation_count
            .saturating_add(batch.coalesced_observation_count);
        state.superseded_publication_count = state
            .superseded_publication_count
            .saturating_add(batch.superseded_publication_count);
        state.payload_truncation_count = state
            .payload_truncation_count
            .saturating_add(batch.payload_truncation_count);
        state.dropped_observation_count = state
            .dropped_observation_count
            .saturating_add(batch.dropped_observation_count);
        state.invalid_observation_count = state
            .invalid_observation_count
            .saturating_add(batch.invalid_observation_count);
        state.unknown_observation_count = state
            .unknown_observation_count
            .saturating_add(batch.unknown_observation_count);
        for record in batch.records {
            if merge_record(
                &mut state.records,
                record,
                &mut state.coalesced_observation_count,
            ) {
                state.payload_truncation_count = state.payload_truncation_count.saturating_add(1);
            }
        }
        enforce_record_bounds(&mut state.records, &mut state.dropped_observation_count, 0);
        Ok(())
    }

    pub fn record_invalid(&mut self) {
        let state = Arc::make_mut(&mut self.state);
        state.invalid_observation_count = state.invalid_observation_count.saturating_add(1);
    }

    pub fn snapshot(&self) -> Arc<ConversationProgressiveActivityProjectionSnapshot> {
        Arc::clone(&self.state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationProgressiveActivityRejection {
    MissingIdentity { field: &'static str },
    UnexpectedIdentity { field: &'static str },
    IdentifierTooLong { field: &'static str },
    KindPayloadMismatch,
    InvalidPayload,
    InvalidBatch,
    NonMonotonicBatch,
    MixedCorrelation,
    PayloadTooLarge,
    ThreadMismatch,
    TurnMismatch,
    StaleSequence { last_sequence: u64 },
}

impl ConversationProgressiveActivityRejection {
    pub const fn notice_label(&self) -> &'static str {
        match self {
            Self::MissingIdentity { .. } => "missing progressive activity identity",
            Self::UnexpectedIdentity { .. } => "unexpected progressive activity identity",
            Self::IdentifierTooLong { .. } => "oversized progressive activity identity",
            Self::KindPayloadMismatch => "progressive activity kind/payload mismatch",
            Self::InvalidPayload => "invalid progressive activity payload",
            Self::InvalidBatch => "invalid progressive activity batch",
            Self::NonMonotonicBatch => "non-monotonic progressive activity batch",
            Self::MixedCorrelation => "mixed progressive activity correlation",
            Self::PayloadTooLarge => "oversized progressive activity payload",
            Self::ThreadMismatch => "progressive activity thread mismatch",
            Self::TurnMismatch => "progressive activity turn mismatch",
            Self::StaleSequence { .. } => "stale progressive activity sequence",
        }
    }
}

pub fn bounded_progressive_prefix(value: &str, maximum_bytes: usize) -> (String, u64) {
    let retained = utf8_prefix(value, maximum_bytes);
    (
        retained.to_string(),
        value.len().saturating_sub(retained.len()) as u64,
    )
}

pub fn bounded_progressive_tail(value: &str, maximum_bytes: usize) -> (String, u64) {
    let retained = utf8_tail(value, maximum_bytes);
    (
        retained.to_string(),
        value.len().saturating_sub(retained.len()) as u64,
    )
}

fn merge_record(
    records: &mut Vec<ConversationProgressiveActivityRecord>,
    record: ConversationProgressiveActivityRecord,
    coalesced_observation_count: &mut u64,
) -> bool {
    if let Some(index) = records
        .iter()
        .position(|current| current.observation.same_coalescing_key(&record.observation))
    {
        let mut current = records.remove(index);
        let expected_truncated_bytes = current
            .observation
            .truncated_bytes()
            .saturating_add(record.observation.truncated_bytes());
        current.last_sequence = current.last_sequence.max(record.last_sequence);
        current.observation_count = current
            .observation_count
            .saturating_add(record.observation_count);
        current.observation.merge_newer(record.observation);
        let merge_truncated_detail =
            current.observation.truncated_bytes() > expected_truncated_bytes;
        *coalesced_observation_count = coalesced_observation_count.saturating_add(1);
        records.push(current);
        merge_truncated_detail
    } else {
        records.push(record);
        false
    }
}

fn validate_record_merge_arithmetic(
    current_records: &[ConversationProgressiveActivityRecord],
    newer_records: &[ConversationProgressiveActivityRecord],
) -> Result<u64, ConversationProgressiveActivityRejection> {
    let mut matched_records = 0u64;
    for incoming in newer_records {
        let Some(current) = current_records.iter().find(|current| {
            current
                .observation
                .same_coalescing_key(&incoming.observation)
        }) else {
            continue;
        };
        matched_records = matched_records
            .checked_add(1)
            .ok_or(ConversationProgressiveActivityRejection::InvalidBatch)?;
        if current
            .observation_count
            .checked_add(incoming.observation_count)
            .is_none()
            || !current
                .observation
                .payload
                .merge_arithmetic_is_valid(&incoming.observation.payload)
        {
            return Err(ConversationProgressiveActivityRejection::InvalidPayload);
        }
    }
    Ok(matched_records)
}

fn checked_record_observation_count(
    records: &[ConversationProgressiveActivityRecord],
    initial: u64,
) -> Option<u64> {
    records.iter().try_fold(initial, |total, record| {
        total.checked_add(record.observation_count)
    })
}

fn enforce_record_bounds(
    records: &mut Vec<ConversationProgressiveActivityRecord>,
    dropped_observation_count: &mut u64,
    reserved_dynamic_bytes: usize,
) {
    let mut retained_bytes = retained_record_bytes(records);
    let mut remove_count = 0usize;
    while records.len().saturating_sub(remove_count) > MAX_RETAINED_PROGRESSIVE_ACTIVITY_RECORDS
        || retained_bytes.saturating_add(reserved_dynamic_bytes)
            > MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES
    {
        let removed = &records[remove_count];
        retained_bytes =
            retained_bytes.saturating_sub(removed.observation.retained_dynamic_bytes());
        *dropped_observation_count =
            dropped_observation_count.saturating_add(removed.observation_count);
        remove_count += 1;
    }
    if remove_count > 0 {
        records.drain(..remove_count);
    }
}

fn retained_record_bytes(records: &[ConversationProgressiveActivityRecord]) -> usize {
    records.iter().fold(0usize, |total, record| {
        total.saturating_add(record.observation.retained_dynamic_bytes())
    })
}

fn min_optional(current: Option<u64>, incoming: Option<u64>) -> Option<u64> {
    match (current, incoming) {
        (Some(current), Some(incoming)) => Some(current.min(incoming)),
        (current, incoming) => current.or(incoming),
    }
}

fn max_optional(current: Option<u64>, incoming: Option<u64>) -> Option<u64> {
    match (current, incoming) {
        (Some(current), Some(incoming)) => Some(current.max(incoming)),
        (current, incoming) => current.or(incoming),
    }
}

fn validate_identifier(
    field: &'static str,
    value: &str,
) -> Result<(), ConversationProgressiveActivityRejection> {
    if value.is_empty() {
        return Err(ConversationProgressiveActivityRejection::MissingIdentity { field });
    }
    if value.len() > MAX_PROGRESSIVE_ACTIVITY_IDENTIFIER_BYTES {
        return Err(ConversationProgressiveActivityRejection::IdentifierTooLong { field });
    }
    Ok(())
}

fn validate_detail(
    value: &str,
    maximum_bytes: usize,
) -> Result<(), ConversationProgressiveActivityRejection> {
    if value.len() > maximum_bytes {
        return Err(ConversationProgressiveActivityRejection::PayloadTooLarge);
    }
    Ok(())
}

fn validate_source_accounting(
    retained_bytes: usize,
    source_bytes: u64,
    truncated_bytes: u64,
) -> Result<(), ConversationProgressiveActivityRejection> {
    if retained_bytes as u128 + u128::from(truncated_bytes) > u128::from(source_bytes) {
        return Err(ConversationProgressiveActivityRejection::InvalidPayload);
    }
    Ok(())
}

fn append_prefix_bounded(target: &mut String, incoming: &str, maximum_bytes: usize) -> u64 {
    let retained = utf8_prefix(incoming, maximum_bytes.saturating_sub(target.len()));
    target.push_str(retained);
    incoming.len().saturating_sub(retained.len()) as u64
}

fn append_tail_bounded(target: &mut String, incoming: &str, maximum_bytes: usize) -> u64 {
    let total_bytes = target.len().saturating_add(incoming.len());
    target.push_str(incoming);
    if target.len() <= maximum_bytes {
        return 0;
    }
    *target = utf8_tail(target, maximum_bytes).to_string();
    total_bytes.saturating_sub(target.len()) as u64
}

fn utf8_prefix(value: &str, maximum_bytes: usize) -> &str {
    if value.len() <= maximum_bytes {
        return value;
    }
    let mut end = maximum_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn utf8_tail(value: &str, maximum_bytes: usize) -> &str {
    if value.len() <= maximum_bytes {
        return value;
    }
    let mut start = value.len().saturating_sub(maximum_bytes);
    while start < value.len() && !value.is_char_boundary(start) {
        start += 1;
    }
    &value[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_observation(
        sequence: u64,
        item_id: &str,
        text: &str,
    ) -> ConversationProgressiveActivityObservation {
        ConversationProgressiveActivityObservation {
            sequence,
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            item_id: Some(item_id.to_string()),
            kind: ConversationProgressiveActivityKind::AgentMessageDelta,
            payload: ConversationProgressiveActivityPayload::AgentMessageDelta {
                phase: None,
                text: text.to_string(),
                source_bytes: text.len() as u64,
                truncated_bytes: 0,
            },
        }
    }

    fn usage_observation(
        sequence: u64,
        total_tokens: u64,
    ) -> ConversationProgressiveActivityObservation {
        usage_observation_with_window(sequence, total_tokens, 100)
    }

    fn usage_observation_with_window(
        sequence: u64,
        total_tokens: u64,
        model_context_window: u64,
    ) -> ConversationProgressiveActivityObservation {
        ConversationProgressiveActivityObservation {
            sequence,
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            item_id: None,
            kind: ConversationProgressiveActivityKind::TokenUsage,
            payload: ConversationProgressiveActivityPayload::TokenUsage {
                usage: ConversationProgressiveTokenUsage {
                    last: ConversationProgressiveTokenUsageBreakdown {
                        cached_input_tokens: 0,
                        input_tokens: total_tokens.saturating_sub(1),
                        output_tokens: 1,
                        reasoning_output_tokens: 0,
                        total_tokens,
                    },
                    total: ConversationProgressiveTokenUsageBreakdown {
                        cached_input_tokens: 0,
                        input_tokens: total_tokens.saturating_sub(1),
                        output_tokens: 1,
                        reasoning_output_tokens: 0,
                        total_tokens,
                    },
                    model_context_window: Some(model_context_window),
                },
            },
        }
    }

    fn command_observation(
        sequence: u64,
        delta: &str,
    ) -> ConversationProgressiveActivityObservation {
        ConversationProgressiveActivityObservation {
            sequence,
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            item_id: Some("command-1".to_string()),
            kind: ConversationProgressiveActivityKind::CommandOutput,
            payload: ConversationProgressiveActivityPayload::CommandOutput {
                tail: delta.to_string(),
                chunk_count: 1,
                source_bytes: delta.len() as u64,
                newline_count: delta.bytes().filter(|byte| *byte == b'\n').count() as u64,
                ends_with_newline: delta.ends_with('\n'),
                truncated_bytes: 0,
            },
        }
    }

    fn patch_observation(
        sequence: u64,
        version: u64,
        omitted_change_count: u64,
        truncated_bytes: u64,
    ) -> ConversationProgressiveActivityObservation {
        let path = format!("src/version-{version}.rs");
        let diff = format!("@@ version {version} @@");
        let source_bytes = path
            .len()
            .saturating_add(diff.len())
            .saturating_add(truncated_bytes as usize) as u64;
        ConversationProgressiveActivityObservation {
            sequence,
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            item_id: Some("patch-1".to_string()),
            kind: ConversationProgressiveActivityKind::FileChangePatch,
            payload: ConversationProgressiveActivityPayload::FileChangePatch {
                changes: vec![ConversationProgressiveFileChange {
                    path,
                    diff,
                    kind: ConversationProgressiveFileChangeKind::Update {
                        move_path_present: version.is_multiple_of(2),
                    },
                }],
                omitted_change_count,
                source_bytes,
                truncated_bytes,
            },
        }
    }

    fn diff_observation(
        sequence: u64,
        version: u64,
        truncated_bytes: u64,
    ) -> ConversationProgressiveActivityObservation {
        let detail =
            format!("@@ version {version} @@\n+added-{version}\n-removed-{version}\n context");
        ConversationProgressiveActivityObservation {
            sequence,
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            item_id: None,
            kind: ConversationProgressiveActivityKind::TurnDiff,
            payload: ConversationProgressiveActivityPayload::TurnDiff {
                source_bytes: detail.len() as u64 + truncated_bytes,
                detail,
                line_count: 4,
                addition_count: 1,
                deletion_count: 1,
                hunk_count: 1,
                truncated_bytes,
            },
        }
    }

    fn plan_observation(
        sequence: u64,
        version: u64,
        omitted_step_count: u64,
        truncated_bytes: u64,
    ) -> ConversationProgressiveActivityObservation {
        let explanation = format!("plan-explanation-{version}");
        let steps = vec![
            ConversationProgressivePlanStep {
                status: ConversationProgressivePlanStepStatus::Completed,
                text: format!("completed-step-{version}"),
            },
            ConversationProgressivePlanStep {
                status: ConversationProgressivePlanStepStatus::InProgress,
                text: format!("active-step-{version}"),
            },
        ];
        let retained_bytes = explanation.len().saturating_add(
            steps
                .iter()
                .fold(0usize, |total, step| total.saturating_add(step.text.len())),
        );
        ConversationProgressiveActivityObservation {
            sequence,
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            item_id: None,
            kind: ConversationProgressiveActivityKind::TurnPlan,
            payload: ConversationProgressiveActivityPayload::TurnPlan {
                explanation: Some(explanation),
                steps,
                omitted_step_count,
                source_bytes: retained_bytes as u64 + truncated_bytes,
                truncated_bytes,
            },
        }
    }

    fn apply_observation(
        projection: &mut ConversationProgressiveActivityProjection,
        observation: ConversationProgressiveActivityObservation,
    ) {
        projection
            .apply_batch_correlated(
                Some("thread-1"),
                Some("turn-1"),
                ConversationProgressiveActivityBatch::single(observation).unwrap(),
            )
            .unwrap();
    }

    #[test]
    fn batch_coalesces_append_activity_and_replaces_latest_state() {
        let mut batch =
            ConversationProgressiveActivityBatch::single(agent_observation(0, "agent-1", "hel"))
                .unwrap();
        batch
            .try_merge_from(
                ConversationProgressiveActivityBatch::single(agent_observation(1, "agent-1", "lo"))
                    .unwrap(),
            )
            .unwrap();

        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.records[0].observation_count, 2);
        assert_eq!(batch.coalesced_observation_count, 1);
        assert!(matches!(
            &batch.records[0].observation.payload,
            ConversationProgressiveActivityPayload::AgentMessageDelta { text, .. }
                if text == "hello"
        ));

        batch
            .try_merge_from(
                ConversationProgressiveActivityBatch::single(usage_observation(2, 10)).unwrap(),
            )
            .unwrap();
        batch
            .try_merge_from(
                ConversationProgressiveActivityBatch::single(usage_observation(3, 20)).unwrap(),
            )
            .unwrap();
        assert_eq!(batch.records.len(), 2);
        assert!(matches!(
            &batch.records[1].observation.payload,
            ConversationProgressiveActivityPayload::TokenUsage { usage }
                if usage.total.total_tokens == 20
        ));
    }

    #[test]
    fn high_rate_append_stream_preserves_agent_prefix_command_tail_and_summaries() {
        const AGENT_CHUNK_BYTES: usize = 4 * 1024;
        const COMMAND_CHUNK_BYTES: usize = 1024;
        const COMMAND_CHUNK_COUNT: usize = 130;

        let agent_chunk_count = MAX_PROGRESSIVE_AGENT_DRAFT_BYTES / AGENT_CHUNK_BYTES + 2;
        let agent_a_chunk = "a".repeat(AGENT_CHUNK_BYTES);
        let agent_b_chunk = "b".repeat(AGENT_CHUNK_BYTES);
        let mut projection = ConversationProgressiveActivityProjection::default();
        let mut sequence = 0u64;

        for _ in 0..agent_chunk_count {
            apply_observation(
                &mut projection,
                agent_observation(sequence, "agent-a", &agent_a_chunk),
            );
            sequence += 1;
            apply_observation(
                &mut projection,
                agent_observation(sequence, "agent-b", &agent_b_chunk),
            );
            sequence += 1;
        }

        let old_command_chunk = format!("{}\n", "o".repeat(COMMAND_CHUNK_BYTES - 1));
        let new_command_chunk = format!("{}\n", "n".repeat(COMMAND_CHUNK_BYTES - 1));
        let command_first_sequence = sequence;
        for index in 0..COMMAND_CHUNK_COUNT {
            let chunk = if index < COMMAND_CHUNK_COUNT / 2 {
                &old_command_chunk
            } else {
                &new_command_chunk
            };
            apply_observation(&mut projection, command_observation(sequence, chunk));
            sequence += 1;
        }

        let snapshot = projection.snapshot();
        assert_eq!(snapshot.last_sequence, Some(sequence - 1));
        assert_eq!(snapshot.source_observation_count, sequence);
        assert_eq!(snapshot.records.len(), 3);
        assert_eq!(
            snapshot.coalesced_observation_count,
            sequence - snapshot.records.len() as u64
        );
        assert_eq!(snapshot.payload_truncation_count, 70);
        assert_eq!(snapshot.dropped_observation_count, 0);
        assert!(snapshot.history_incomplete());

        for (item_id, retained_byte) in [("agent-a", b'a'), ("agent-b", b'b')] {
            let record = snapshot
                .records
                .iter()
                .find(|record| record.observation().item_id.as_deref() == Some(item_id))
                .unwrap();
            let ConversationProgressiveActivityPayload::AgentMessageDelta {
                text,
                source_bytes,
                truncated_bytes,
                ..
            } = &record.observation().payload
            else {
                panic!("agent record should retain an agent delta")
            };
            assert_eq!(record.observation_count(), agent_chunk_count as u64);
            assert_eq!(text.len(), MAX_PROGRESSIVE_AGENT_DRAFT_BYTES);
            assert!(text.bytes().all(|byte| byte == retained_byte));
            assert_eq!(
                *source_bytes,
                (agent_chunk_count * AGENT_CHUNK_BYTES) as u64
            );
            assert_eq!(*truncated_bytes, (2 * AGENT_CHUNK_BYTES) as u64);
        }

        let agent_a_record = snapshot
            .records
            .iter()
            .find(|record| record.observation().item_id.as_deref() == Some("agent-a"))
            .unwrap();
        let agent_b_record = snapshot
            .records
            .iter()
            .find(|record| record.observation().item_id.as_deref() == Some("agent-b"))
            .unwrap();
        assert_eq!(agent_a_record.first_sequence(), 0);
        assert_eq!(agent_a_record.last_sequence(), sequence - 132);
        assert_eq!(agent_b_record.first_sequence(), 1);
        assert_eq!(agent_b_record.last_sequence(), sequence - 131);

        let command_record = snapshot
            .records
            .iter()
            .find(|record| record.observation().item_id.as_deref() == Some("command-1"))
            .unwrap();
        let ConversationProgressiveActivityPayload::CommandOutput {
            tail,
            chunk_count,
            source_bytes,
            newline_count,
            ends_with_newline,
            truncated_bytes,
        } = &command_record.observation().payload
        else {
            panic!("command record should retain command output")
        };
        assert_eq!(command_record.first_sequence(), command_first_sequence);
        assert_eq!(command_record.last_sequence(), sequence - 1);
        assert_eq!(
            command_record.observation_count(),
            COMMAND_CHUNK_COUNT as u64
        );
        assert_eq!(*chunk_count, COMMAND_CHUNK_COUNT as u64);
        assert_eq!(
            *source_bytes,
            (COMMAND_CHUNK_COUNT * COMMAND_CHUNK_BYTES) as u64
        );
        assert_eq!(*newline_count, COMMAND_CHUNK_COUNT as u64);
        assert!(*ends_with_newline);
        assert_eq!(
            *truncated_bytes,
            *source_bytes - MAX_PROGRESSIVE_COMMAND_DETAIL_BYTES as u64
        );
        assert_eq!(tail, &new_command_chunk.repeat(64));
        assert_eq!(
            command_record
                .observation()
                .payload
                .command_output_line_count(),
            Some(COMMAND_CHUNK_COUNT as u64)
        );

        // This is the reducer's retained String/identifier accounting, not allocator RSS.
        let tracked_retained_dynamic_bytes = snapshot.retained_dynamic_bytes();
        assert!(tracked_retained_dynamic_bytes <= MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES);
        assert!(snapshot.records.len() <= MAX_RETAINED_PROGRESSIVE_ACTIVITY_RECORDS);
    }

    #[test]
    fn high_rate_latest_state_stream_preserves_newest_patch_diff_plan_and_token_facts() {
        const UPDATES_PER_KIND: u64 = 512;
        const PATCH_TRUNCATED_BYTES: u64 = 123;
        const DIFF_TRUNCATED_BYTES: u64 = 321;
        const PLAN_TRUNCATED_BYTES: u64 = 77;

        let mut projection = ConversationProgressiveActivityProjection::default();
        let mut sequence = 0u64;
        for version in 0..UPDATES_PER_KIND {
            apply_observation(
                &mut projection,
                patch_observation(
                    sequence,
                    version,
                    u64::from(version + 1 == UPDATES_PER_KIND) * 7,
                    u64::from(version + 1 == UPDATES_PER_KIND) * PATCH_TRUNCATED_BYTES,
                ),
            );
            sequence += 1;
        }
        for version in 0..UPDATES_PER_KIND {
            apply_observation(
                &mut projection,
                diff_observation(
                    sequence,
                    version,
                    u64::from(version + 1 == UPDATES_PER_KIND) * DIFF_TRUNCATED_BYTES,
                ),
            );
            sequence += 1;
        }
        for version in 0..UPDATES_PER_KIND {
            apply_observation(
                &mut projection,
                plan_observation(
                    sequence,
                    version,
                    u64::from(version + 1 == UPDATES_PER_KIND) * 9,
                    u64::from(version + 1 == UPDATES_PER_KIND) * PLAN_TRUNCATED_BYTES,
                ),
            );
            sequence += 1;
        }
        for version in 0..UPDATES_PER_KIND {
            apply_observation(&mut projection, usage_observation(sequence, version + 2));
            sequence += 1;
        }

        let latest_version = UPDATES_PER_KIND - 1;
        let snapshot = projection.snapshot();
        assert_eq!(snapshot.last_sequence, Some(sequence - 1));
        assert_eq!(snapshot.source_observation_count, sequence);
        assert_eq!(snapshot.records.len(), 4);
        assert_eq!(
            snapshot.coalesced_observation_count,
            4 * (UPDATES_PER_KIND - 1)
        );
        assert_eq!(snapshot.payload_truncation_count, 3);
        assert_eq!(snapshot.dropped_observation_count, 0);

        let patch_record = snapshot
            .records
            .iter()
            .find(|record| {
                record.observation().kind == ConversationProgressiveActivityKind::FileChangePatch
            })
            .unwrap();
        let ConversationProgressiveActivityPayload::FileChangePatch {
            changes,
            omitted_change_count,
            source_bytes,
            truncated_bytes,
        } = &patch_record.observation().payload
        else {
            panic!("patch record should retain a patch")
        };
        assert_eq!(patch_record.first_sequence(), 0);
        assert_eq!(patch_record.last_sequence(), UPDATES_PER_KIND - 1);
        assert_eq!(patch_record.observation_count(), UPDATES_PER_KIND);
        assert_eq!(*omitted_change_count, 7);
        assert_eq!(*truncated_bytes, PATCH_TRUNCATED_BYTES);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, format!("src/version-{latest_version}.rs"));
        assert_eq!(changes[0].diff, format!("@@ version {latest_version} @@"));
        assert_eq!(
            *source_bytes,
            (changes[0].path.len() + changes[0].diff.len()) as u64 + PATCH_TRUNCATED_BYTES
        );

        let diff_record = snapshot
            .records
            .iter()
            .find(|record| {
                record.observation().kind == ConversationProgressiveActivityKind::TurnDiff
            })
            .unwrap();
        let ConversationProgressiveActivityPayload::TurnDiff {
            detail,
            source_bytes,
            line_count,
            addition_count,
            deletion_count,
            hunk_count,
            truncated_bytes,
        } = &diff_record.observation().payload
        else {
            panic!("diff record should retain a turn diff")
        };
        assert_eq!(diff_record.first_sequence(), UPDATES_PER_KIND);
        assert_eq!(diff_record.last_sequence(), 2 * UPDATES_PER_KIND - 1);
        assert_eq!(diff_record.observation_count(), UPDATES_PER_KIND);
        assert_eq!(
            detail,
            &format!(
                "@@ version {latest_version} @@\n+added-{latest_version}\n-removed-{latest_version}\n context"
            )
        );
        assert_eq!(*source_bytes, detail.len() as u64 + DIFF_TRUNCATED_BYTES);
        assert_eq!(*line_count, 4);
        assert_eq!(*addition_count, 1);
        assert_eq!(*deletion_count, 1);
        assert_eq!(*hunk_count, 1);
        assert_eq!(*truncated_bytes, DIFF_TRUNCATED_BYTES);

        let plan_record = snapshot
            .records
            .iter()
            .find(|record| {
                record.observation().kind == ConversationProgressiveActivityKind::TurnPlan
            })
            .unwrap();
        let ConversationProgressiveActivityPayload::TurnPlan {
            explanation,
            steps,
            omitted_step_count,
            source_bytes,
            truncated_bytes,
        } = &plan_record.observation().payload
        else {
            panic!("plan record should retain a turn plan")
        };
        assert_eq!(plan_record.first_sequence(), 2 * UPDATES_PER_KIND);
        assert_eq!(plan_record.last_sequence(), 3 * UPDATES_PER_KIND - 1);
        assert_eq!(plan_record.observation_count(), UPDATES_PER_KIND);
        assert_eq!(
            explanation.as_deref(),
            Some(format!("plan-explanation-{latest_version}").as_str())
        );
        assert_eq!(steps.len(), 2);
        assert_eq!(
            steps[0].status,
            ConversationProgressivePlanStepStatus::Completed
        );
        assert_eq!(steps[0].text, format!("completed-step-{latest_version}"));
        assert_eq!(
            steps[1].status,
            ConversationProgressivePlanStepStatus::InProgress
        );
        assert_eq!(steps[1].text, format!("active-step-{latest_version}"));
        assert_eq!(*omitted_step_count, 9);
        let retained_plan_bytes = explanation.as_ref().map_or(0, String::len)
            + steps.iter().map(|step| step.text.len()).sum::<usize>();
        assert_eq!(
            *source_bytes,
            retained_plan_bytes as u64 + PLAN_TRUNCATED_BYTES
        );
        assert_eq!(*truncated_bytes, PLAN_TRUNCATED_BYTES);

        let usage_record = snapshot
            .records
            .iter()
            .find(|record| {
                record.observation().kind == ConversationProgressiveActivityKind::TokenUsage
            })
            .unwrap();
        let ConversationProgressiveActivityPayload::TokenUsage { usage } =
            &usage_record.observation().payload
        else {
            panic!("usage record should retain token usage")
        };
        assert_eq!(usage_record.first_sequence(), 3 * UPDATES_PER_KIND);
        assert_eq!(usage_record.last_sequence(), 4 * UPDATES_PER_KIND - 1);
        assert_eq!(usage_record.observation_count(), UPDATES_PER_KIND);
        assert_eq!(usage.total.total_tokens, latest_version + 2);
        assert_eq!(usage.context_pressure_basis_points(), Some(10_000));

        // This verifies the reducer's own retained dynamic-byte budget, not process RSS.
        let tracked_retained_dynamic_bytes = snapshot.retained_dynamic_bytes();
        assert!(tracked_retained_dynamic_bytes <= MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES);
        assert!(snapshot.records.len() <= MAX_RETAINED_PROGRESSIVE_ACTIVITY_RECORDS);
    }

    #[test]
    fn projection_bounds_a_large_multi_item_stream_and_rejects_replay() {
        let mut projection = ConversationProgressiveActivityProjection::default();
        for sequence in 0..100_000u64 {
            projection
                .apply_batch_correlated(
                    Some("thread-1"),
                    Some("turn-1"),
                    ConversationProgressiveActivityBatch::single(agent_observation(
                        sequence,
                        format!("item-{sequence}").as_str(),
                        "x",
                    ))
                    .unwrap(),
                )
                .unwrap();
        }

        let snapshot = projection.snapshot();
        assert_eq!(
            snapshot.records.len(),
            MAX_RETAINED_PROGRESSIVE_ACTIVITY_RECORDS
        );
        // This is domain-accounted retained detail, not a measurement of allocator RSS.
        let tracked_retained_dynamic_bytes = snapshot.retained_dynamic_bytes();
        assert!(tracked_retained_dynamic_bytes <= MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES);
        assert_eq!(snapshot.last_sequence, Some(99_999));
        assert!(snapshot.loss_event_count() > 0);

        assert!(matches!(
            projection.apply_batch_correlated(
                Some("thread-1"),
                Some("turn-1"),
                ConversationProgressiveActivityBatch::single(agent_observation(
                    99_999, "replay", "x",
                ))
                .unwrap(),
            ),
            Err(ConversationProgressiveActivityRejection::StaleSequence { .. })
        ));
    }

    #[test]
    fn projection_enforces_tracked_dynamic_byte_bound_for_large_multi_agent_detail() {
        const AGENT_COUNT: u64 = 12;

        let full_detail = "x".repeat(MAX_PROGRESSIVE_AGENT_DRAFT_BYTES);
        let mut projection = ConversationProgressiveActivityProjection::default();
        for sequence in 0..AGENT_COUNT {
            apply_observation(
                &mut projection,
                agent_observation(sequence, &format!("agent-{sequence}"), &full_detail),
            );
        }

        let snapshot = projection.snapshot();
        assert_eq!(snapshot.last_sequence, Some(AGENT_COUNT - 1));
        assert_eq!(snapshot.source_observation_count, AGENT_COUNT);
        assert_eq!(snapshot.coalesced_observation_count, 0);
        assert_eq!(snapshot.payload_truncation_count, 0);
        assert_eq!(snapshot.dropped_observation_count, 9);
        assert_eq!(snapshot.records.len(), 3);
        assert_eq!(
            snapshot
                .records
                .iter()
                .map(ConversationProgressiveActivityRecord::last_sequence)
                .collect::<Vec<_>>(),
            vec![9, 10, 11]
        );
        assert!(snapshot.records.iter().all(|record| {
            matches!(
                &record.observation().payload,
                ConversationProgressiveActivityPayload::AgentMessageDelta {
                    text,
                    truncated_bytes: 0,
                    ..
                } if text.len() == MAX_PROGRESSIVE_AGENT_DRAFT_BYTES
            )
        }));
        assert_eq!(
            snapshot
                .records
                .iter()
                .map(ConversationProgressiveActivityRecord::observation_count)
                .sum::<u64>()
                + snapshot.dropped_observation_count,
            snapshot.source_observation_count
        );

        // The reducer tracks retained String/identifier bytes; this does not claim allocator RSS.
        let tracked_retained_dynamic_bytes = snapshot.retained_dynamic_bytes();
        assert!(tracked_retained_dynamic_bytes <= MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES);
        assert!(tracked_retained_dynamic_bytes >= 3 * MAX_PROGRESSIVE_AGENT_DRAFT_BYTES);
        assert!(snapshot.records.len() <= MAX_RETAINED_PROGRESSIVE_ACTIVITY_RECORDS);
        assert!(snapshot.history_incomplete());
    }

    #[test]
    fn batch_validation_rejects_mutation_and_non_monotonic_merge_atomically() {
        let mut missing_turn =
            ConversationProgressiveActivityBatch::single(agent_observation(1, "agent-1", "valid"))
                .unwrap();
        missing_turn.records[0].observation.turn_id = None;
        assert!(matches!(
            missing_turn.validate(),
            Err(ConversationProgressiveActivityRejection::MissingIdentity { field: "turnId" })
        ));

        let mut invalid_range =
            ConversationProgressiveActivityBatch::single(agent_observation(1, "agent-1", "valid"))
                .unwrap();
        invalid_range.first_sequence = None;
        assert_eq!(
            invalid_range.validate(),
            Err(ConversationProgressiveActivityRejection::InvalidBatch)
        );

        let mut current =
            ConversationProgressiveActivityBatch::single(agent_observation(10, "agent-1", "newer"))
                .unwrap();
        let before = current.clone();
        assert_eq!(
            current.try_merge_from(
                ConversationProgressiveActivityBatch::single(agent_observation(
                    9, "agent-1", "older",
                ))
                .unwrap(),
            ),
            Err(ConversationProgressiveActivityRejection::NonMonotonicBatch)
        );
        assert_eq!(current, before);
    }

    #[test]
    fn newer_history_only_batch_advances_sequence_without_invalidating_older_detail() {
        let mut batch =
            ConversationProgressiveActivityBatch::single(agent_observation(0, "agent-1", "kept"))
                .unwrap();
        let mut history_only = ConversationProgressiveActivityBatch::single(agent_observation(
            1,
            "agent-2",
            "discarded",
        ))
        .unwrap();
        history_only.discard_retained_records();

        batch.try_merge_from(history_only).unwrap();
        assert_eq!(batch.first_sequence(), Some(0));
        assert_eq!(batch.last_sequence(), Some(1));
        assert_eq!(batch.source_observation_count(), 2);
        assert_eq!(batch.dropped_observation_count(), 1);
        assert_eq!(batch.records().len(), 1);
        assert_eq!(batch.records()[0].last_sequence(), 0);
        batch.validate().unwrap();
    }

    #[test]
    fn merge_rejects_public_source_counter_overflow_atomically() {
        let extreme = |sequence| ConversationProgressiveActivityObservation {
            sequence,
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            item_id: Some("agent-1".to_string()),
            kind: ConversationProgressiveActivityKind::AgentMessageDelta,
            payload: ConversationProgressiveActivityPayload::AgentMessageDelta {
                phase: None,
                text: "x".to_string(),
                source_bytes: u64::MAX,
                truncated_bytes: u64::MAX - 1,
            },
        };
        let mut current = ConversationProgressiveActivityBatch::single(extreme(0)).unwrap();
        let before = current.clone();
        let newer = ConversationProgressiveActivityBatch::single(extreme(1)).unwrap();

        assert_eq!(
            current.try_merge_from(newer),
            Err(ConversationProgressiveActivityRejection::InvalidPayload)
        );
        assert_eq!(current, before);
        current.validate().unwrap();
    }

    #[test]
    fn projection_rejects_cross_publication_payload_overflow_before_merging() {
        let extreme = ConversationProgressiveActivityObservation {
            sequence: 0,
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            item_id: Some("agent-1".to_string()),
            kind: ConversationProgressiveActivityKind::AgentMessageDelta,
            payload: ConversationProgressiveActivityPayload::AgentMessageDelta {
                phase: None,
                text: "x".to_string(),
                source_bytes: u64::MAX,
                truncated_bytes: u64::MAX - 1,
            },
        };
        let mut projection = ConversationProgressiveActivityProjection::default();
        projection
            .apply_batch_correlated(
                Some("thread-1"),
                Some("turn-1"),
                ConversationProgressiveActivityBatch::single(extreme).unwrap(),
            )
            .unwrap();
        let before = projection.snapshot();

        assert_eq!(
            projection.apply_batch_correlated(
                Some("thread-1"),
                Some("turn-1"),
                ConversationProgressiveActivityBatch::single(agent_observation(1, "agent-1", "y",))
                    .unwrap(),
            ),
            Err(ConversationProgressiveActivityRejection::InvalidPayload)
        );

        let after = projection.snapshot();
        assert_eq!(after.records, before.records);
        assert_eq!(
            after.source_observation_count,
            before.source_observation_count
        );
        assert_eq!(
            after.coalesced_observation_count,
            before.coalesced_observation_count
        );
        assert_eq!(
            after.payload_truncation_count,
            before.payload_truncation_count
        );
        assert_eq!(
            after.invalid_observation_count,
            before.invalid_observation_count + 1
        );
    }

    #[test]
    fn public_payload_validation_rejects_inconsistent_bytes_counts_and_token_overflow() {
        let mut inconsistent_bytes = agent_observation(1, "agent-1", "valid");
        let ConversationProgressiveActivityPayload::AgentMessageDelta {
            truncated_bytes, ..
        } = &mut inconsistent_bytes.payload
        else {
            unreachable!()
        };
        *truncated_bytes = 1;
        assert_eq!(
            ConversationProgressiveActivityBatch::single(inconsistent_bytes),
            Err(ConversationProgressiveActivityRejection::InvalidPayload)
        );

        let mut zero_chunk = command_observation(2, "output");
        let ConversationProgressiveActivityPayload::CommandOutput { chunk_count, .. } =
            &mut zero_chunk.payload
        else {
            unreachable!()
        };
        *chunk_count = 0;
        assert_eq!(
            ConversationProgressiveActivityBatch::single(zero_chunk),
            Err(ConversationProgressiveActivityRejection::InvalidPayload)
        );

        let oversized_count = ConversationProgressiveActivityObservation {
            sequence: 3,
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            item_id: Some("plan-1".to_string()),
            kind: ConversationProgressiveActivityKind::PlanDelta,
            payload: ConversationProgressiveActivityPayload::PlanDelta {
                chunk_count: u64::MAX,
                source_bytes: 0,
            },
        };
        assert_eq!(
            ConversationProgressiveActivityBatch::single(oversized_count),
            Err(ConversationProgressiveActivityRejection::InvalidPayload)
        );

        let overflow_usage = ConversationProgressiveActivityObservation {
            sequence: 4,
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            item_id: None,
            kind: ConversationProgressiveActivityKind::TokenUsage,
            payload: ConversationProgressiveActivityPayload::TokenUsage {
                usage: ConversationProgressiveTokenUsage {
                    last: ConversationProgressiveTokenUsageBreakdown {
                        cached_input_tokens: 0,
                        input_tokens: 1,
                        output_tokens: 1,
                        reasoning_output_tokens: 0,
                        total_tokens: 2,
                    },
                    total: ConversationProgressiveTokenUsageBreakdown {
                        cached_input_tokens: 0,
                        input_tokens: u64::MAX,
                        output_tokens: 1,
                        reasoning_output_tokens: 0,
                        total_tokens: u64::MAX,
                    },
                    model_context_window: Some(u64::MAX),
                },
            },
        };
        assert_eq!(
            ConversationProgressiveActivityBatch::single(overflow_usage),
            Err(ConversationProgressiveActivityRejection::InvalidPayload)
        );

        let mut inconsistent_snapshot = usage_observation(5, 100);
        let ConversationProgressiveActivityPayload::TokenUsage { usage } =
            &mut inconsistent_snapshot.payload
        else {
            unreachable!()
        };
        usage.total = ConversationProgressiveTokenUsageBreakdown {
            cached_input_tokens: 0,
            input_tokens: 0,
            output_tokens: 1,
            reasoning_output_tokens: 0,
            total_tokens: 1,
        };
        assert_eq!(
            ConversationProgressiveActivityBatch::single(inconsistent_snapshot),
            Err(ConversationProgressiveActivityRejection::InvalidPayload)
        );

        let mut impossible_diff_summary = diff_observation(6, 1, 0);
        let ConversationProgressiveActivityPayload::TurnDiff { addition_count, .. } =
            &mut impossible_diff_summary.payload
        else {
            unreachable!()
        };
        *addition_count = 5;
        assert_eq!(
            ConversationProgressiveActivityBatch::single(impossible_diff_summary),
            Err(ConversationProgressiveActivityRejection::InvalidPayload)
        );
    }

    #[test]
    fn replacing_message_detail_separates_superseded_from_truncated_bytes() {
        let observation = |sequence, message: &str| ConversationProgressiveActivityObservation {
            sequence,
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            item_id: Some("mcp-1".to_string()),
            kind: ConversationProgressiveActivityKind::McpProgress,
            payload: ConversationProgressiveActivityPayload::McpProgress {
                message: message.to_string(),
                update_count: 1,
                source_bytes: message.len() as u64,
                truncated_bytes: 0,
            },
        };
        let mut batch = ConversationProgressiveActivityBatch::single(observation(0, "older"))
            .expect("first MCP update should be valid");
        batch
            .try_merge_from(
                ConversationProgressiveActivityBatch::single(observation(1, "new"))
                    .expect("second MCP update should be valid"),
            )
            .expect("MCP updates should coalesce");

        let payload = &batch.records()[0].observation().payload;
        let ConversationProgressiveActivityPayload::McpProgress {
            message,
            source_bytes,
            truncated_bytes,
            update_count,
        } = payload
        else {
            unreachable!()
        };
        assert_eq!(message, "new");
        assert_eq!(*update_count, 2);
        assert_eq!(*source_bytes, 8);
        assert_eq!(*truncated_bytes, 0);
        assert_eq!(batch.coalesced_observation_count(), 1);
        batch.validate().unwrap();
    }

    #[test]
    fn projection_marks_detail_trimmed_during_cross_publication_merge() {
        let mut projection = ConversationProgressiveActivityProjection::default();
        let full = "a".repeat(MAX_PROGRESSIVE_AGENT_DRAFT_BYTES);
        projection
            .apply_batch_correlated(
                Some("thread-1"),
                Some("turn-1"),
                ConversationProgressiveActivityBatch::single(agent_observation(
                    0, "agent-1", &full,
                ))
                .unwrap(),
            )
            .unwrap();
        projection
            .apply_batch_correlated(
                Some("thread-1"),
                Some("turn-1"),
                ConversationProgressiveActivityBatch::single(agent_observation(1, "agent-1", "b"))
                    .unwrap(),
            )
            .unwrap();

        let snapshot = projection.snapshot();
        assert_eq!(snapshot.payload_truncation_count, 1);
        assert_eq!(snapshot.dropped_observation_count, 0);
        assert!(snapshot.history_incomplete());
        assert!(snapshot.records[0].observation().truncated_bytes() > 0);
    }

    #[test]
    fn history_only_batch_preserves_sequence_correlation_and_drop_count() {
        let mut batch = ConversationProgressiveActivityBatch::single(agent_observation(
            7,
            "agent-1",
            "discarded",
        ))
        .unwrap();
        batch.discard_retained_records();
        assert!(batch.records().is_empty());
        assert_eq!(batch.dropped_observation_count(), 1);
        batch.validate().unwrap();

        let mut projection = ConversationProgressiveActivityProjection::default();
        projection
            .apply_batch_correlated(Some("thread-1"), Some("turn-1"), batch)
            .unwrap();
        let snapshot = projection.snapshot();
        assert_eq!(snapshot.last_sequence, Some(7));
        assert_eq!(snapshot.source_observation_count, 1);
        assert_eq!(snapshot.dropped_observation_count, 1);
        assert!(snapshot.records.is_empty());
        assert!(snapshot.history_incomplete());
    }

    #[test]
    fn command_line_count_is_exact_across_chunk_boundaries() {
        let mut batch =
            ConversationProgressiveActivityBatch::single(command_observation(0, "a")).unwrap();
        batch
            .try_merge_from(
                ConversationProgressiveActivityBatch::single(command_observation(1, "b")).unwrap(),
            )
            .unwrap();
        assert_eq!(
            batch.records()[0]
                .observation()
                .payload
                .command_output_line_count(),
            Some(1)
        );

        batch
            .try_merge_from(
                ConversationProgressiveActivityBatch::single(command_observation(2, "\nc\n"))
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            batch.records()[0]
                .observation()
                .payload
                .command_output_line_count(),
            Some(2)
        );
    }

    #[test]
    fn token_usage_reports_bounded_context_pressure_basis_points() {
        let observation = usage_observation(0, 25);
        let ConversationProgressiveActivityPayload::TokenUsage { usage } = observation.payload
        else {
            panic!("usage fixture should retain token usage")
        };

        assert_eq!(usage.context_pressure_basis_points(), Some(2_500));
        let after_compaction = ConversationProgressiveTokenUsage {
            total: ConversationProgressiveTokenUsageBreakdown {
                cached_input_tokens: 0,
                input_tokens: 999_999,
                output_tokens: 1,
                reasoning_output_tokens: 0,
                total_tokens: 1_000_000,
            },
            ..usage
        };
        assert_eq!(
            after_compaction.context_pressure_basis_points(),
            Some(2_500)
        );
        let invalid_zero_window = ConversationProgressiveTokenUsage {
            model_context_window: Some(0),
            ..usage
        };
        assert_eq!(invalid_zero_window.context_pressure_basis_points(), None);
    }

    #[test]
    fn token_usage_context_pressure_is_exact_at_int64_maximum_and_boundaries() {
        let maximum_schema_integer = i64::MAX as u64;
        let at_capacity =
            usage_observation_with_window(0, maximum_schema_integer, maximum_schema_integer);
        let ConversationProgressiveActivityPayload::TokenUsage { usage: at_capacity } =
            at_capacity.payload
        else {
            panic!("usage fixture should retain token usage")
        };
        at_capacity.validate().unwrap();
        assert_eq!(at_capacity.context_pressure_basis_points(), Some(10_000));

        let below_capacity =
            usage_observation_with_window(1, maximum_schema_integer - 1, maximum_schema_integer);
        let ConversationProgressiveActivityPayload::TokenUsage {
            usage: below_capacity,
        } = below_capacity.payload
        else {
            panic!("usage fixture should retain token usage")
        };
        below_capacity.validate().unwrap();
        assert_eq!(below_capacity.context_pressure_basis_points(), Some(9_999));

        let above_capacity =
            usage_observation_with_window(2, maximum_schema_integer, maximum_schema_integer - 1);
        let ConversationProgressiveActivityPayload::TokenUsage {
            usage: above_capacity,
        } = above_capacity.payload
        else {
            panic!("usage fixture should retain token usage")
        };
        above_capacity.validate().unwrap();
        assert_eq!(above_capacity.context_pressure_basis_points(), Some(10_000));
    }

    #[test]
    fn projection_shares_snapshots_and_rejects_cross_turn_data() {
        let mut projection = ConversationProgressiveActivityProjection::default();
        projection
            .apply_batch_correlated(
                Some("thread-1"),
                Some("turn-1"),
                ConversationProgressiveActivityBatch::single(agent_observation(
                    0, "item-1", "hello",
                ))
                .unwrap(),
            )
            .unwrap();
        let first = projection.snapshot();
        assert!(Arc::ptr_eq(&first, &projection.snapshot()));

        projection
            .apply_batch_correlated(
                Some("thread-1"),
                Some("turn-1"),
                ConversationProgressiveActivityBatch::single(agent_observation(
                    1, "item-1", " world",
                ))
                .unwrap(),
            )
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &projection.snapshot()));

        let mut stale = agent_observation(2, "item-1", "secret");
        stale.turn_id = Some("turn-other".to_string());
        assert!(matches!(
            projection.apply_batch_correlated(
                Some("thread-1"),
                Some("turn-1"),
                ConversationProgressiveActivityBatch::single(stale).unwrap(),
            ),
            Err(ConversationProgressiveActivityRejection::TurnMismatch)
        ));
    }

    #[test]
    fn debug_output_redacts_retained_detail_and_identity() {
        let secret = "AKRA_PROGRESSIVE_SECRET_CANARY";
        let observation = ConversationProgressiveActivityObservation {
            sequence: 0,
            thread_id: secret.to_string(),
            turn_id: Some(secret.to_string()),
            item_id: Some(secret.to_string()),
            kind: ConversationProgressiveActivityKind::CommandOutput,
            payload: ConversationProgressiveActivityPayload::CommandOutput {
                tail: secret.to_string(),
                chunk_count: 1,
                source_bytes: secret.len() as u64,
                newline_count: 0,
                ends_with_newline: false,
                truncated_bytes: 0,
            },
        };
        let debug = format!(
            "{:?}",
            ConversationProgressiveActivityBatch::single(observation).unwrap()
        );
        assert!(!debug.contains(secret));
        assert!(debug.contains("command-output"));
    }

    #[test]
    fn utf8_bounds_report_exact_omitted_bytes() {
        let value = "한".repeat(10);
        let (prefix, prefix_omitted) = bounded_progressive_prefix(&value, 10);
        let (tail, tail_omitted) = bounded_progressive_tail(&value, 10);
        assert_eq!(prefix.len() as u64 + prefix_omitted, value.len() as u64);
        assert_eq!(tail.len() as u64 + tail_omitted, value.len() as u64);
        assert!(prefix.is_char_boundary(prefix.len()));
        assert!(tail.is_char_boundary(tail.len()));
    }
}
