use std::sync::Arc;

pub const MAX_CONVERSATION_ITEM_IDENTIFIER_BYTES: usize = 4 * 1024;
pub const MAX_CONVERSATION_ITEM_KIND_LABEL_BYTES: usize = 4 * 1024;
pub const MAX_CONVERSATION_ITEM_OUTCOME_LABEL_BYTES: usize = 4 * 1024;
pub const MAX_CONVERSATION_ITEM_SUMMARY_BYTES: usize = 4 * 1024;
pub const MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS: usize = 256;
pub const MAX_RETAINED_CONVERSATION_ITEM_DYNAMIC_BYTES: usize =
    MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS
        * (MAX_CONVERSATION_ITEM_IDENTIFIER_BYTES * 3
            + MAX_CONVERSATION_ITEM_KIND_LABEL_BYTES
            + MAX_CONVERSATION_ITEM_OUTCOME_LABEL_BYTES
            + MAX_CONVERSATION_ITEM_SUMMARY_BYTES);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationItemLifecycleObservation {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub kind: ConversationItemKind,
    pub phase: ConversationItemLifecyclePhase,
    pub source: ConversationItemLifecycleSource,
    pub observed_at_ms: Option<i64>,
    pub outcome: ConversationItemOutcome,
    pub summary: String,
}

impl ConversationItemLifecycleObservation {
    pub fn validate(&self) -> Result<(), ConversationItemLifecycleRejection> {
        validate_observation(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationItemKind {
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
    EnteredReviewMode,
    ExitedReviewMode,
    ContextCompaction,
    Unknown(String),
}

impl ConversationItemKind {
    pub const fn stable_wire_label(&self) -> Option<&'static str> {
        match self {
            Self::UserMessage => Some("userMessage"),
            Self::HookPrompt => Some("hookPrompt"),
            Self::AgentMessage => Some("agentMessage"),
            Self::Plan => Some("plan"),
            Self::Reasoning => Some("reasoning"),
            Self::CommandExecution => Some("commandExecution"),
            Self::FileChange => Some("fileChange"),
            Self::McpToolCall => Some("mcpToolCall"),
            Self::DynamicToolCall => Some("dynamicToolCall"),
            Self::CollaborationAgentToolCall => Some("collabAgentToolCall"),
            Self::SubAgentActivity => Some("subAgentActivity"),
            Self::WebSearch => Some("webSearch"),
            Self::ImageView => Some("imageView"),
            Self::Sleep => Some("sleep"),
            Self::ImageGeneration => Some("imageGeneration"),
            Self::EnteredReviewMode => Some("enteredReviewMode"),
            Self::ExitedReviewMode => Some("exitedReviewMode"),
            Self::ContextCompaction => Some("contextCompaction"),
            Self::Unknown(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationItemLifecyclePhase {
    Started,
    Completed,
    SnapshotObserved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationItemLifecycleSource {
    Live,
    Snapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationItemOutcome {
    NotReported,
    InProgress,
    Completed,
    Failed,
    Declined,
    Interrupted,
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationItemLifecycleConsistency {
    Accepted,
    SnapshotObserved,
    DuplicateStart,
    DuplicateCompletion,
    CompletionWithoutStart,
    StartAfterCompletion,
    KindMismatch,
    TimestampRegression,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationItemLifecycleRecord {
    pub sequence: u64,
    pub observation: ConversationItemLifecycleObservation,
    pub consistency: ConversationItemLifecycleConsistency,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationItemLifecycleProjectionSnapshot {
    pub records: Vec<ConversationItemLifecycleRecord>,
    pub truncated_record_count: u64,
    pub invalid_record_count: u64,
    pub unknown_kind_record_count: u64,
}

impl ConversationItemLifecycleProjectionSnapshot {
    pub fn is_complete(&self) -> bool {
        self.truncated_record_count == 0
            && self.invalid_record_count == 0
            && self.unknown_kind_record_count == 0
    }

    pub fn retained_dynamic_bytes(&self) -> usize {
        self.records.iter().fold(0usize, |total, record| {
            let observation = &record.observation;
            let kind_bytes = match &observation.kind {
                ConversationItemKind::Unknown(label) => label.len(),
                _ => 0,
            };
            let outcome_bytes = match &observation.outcome {
                ConversationItemOutcome::Unknown(label) => label.len(),
                _ => 0,
            };
            total
                .saturating_add(observation.thread_id.len())
                .saturating_add(observation.turn_id.len())
                .saturating_add(observation.item_id.len())
                .saturating_add(kind_bytes)
                .saturating_add(outcome_bytes)
                .saturating_add(observation.summary.len())
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationItemLifecycleProjection {
    state: Arc<ConversationItemLifecycleProjectionSnapshot>,
    next_sequence: u64,
}

impl ConversationItemLifecycleProjection {
    pub fn from_snapshot_for_thread(
        expected_thread_id: &str,
        snapshot: Arc<ConversationItemLifecycleProjectionSnapshot>,
    ) -> Result<Self, ConversationItemLifecycleHydrationRejection> {
        if snapshot.records.len() > MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS {
            return Err(ConversationItemLifecycleHydrationRejection::TooManyRecords);
        }
        if snapshot.truncated_record_count != 0
            && snapshot.records.len() != MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS
        {
            return Err(ConversationItemLifecycleHydrationRejection::TruncationHistoryMismatch);
        }

        let mut retained_unknown_kind_count = 0u64;
        for (index, record) in snapshot.records.iter().enumerate() {
            record.observation.validate().map_err(|rejection| {
                ConversationItemLifecycleHydrationRejection::InvalidRecord { index, rejection }
            })?;
            if record.observation.thread_id != expected_thread_id {
                return Err(ConversationItemLifecycleHydrationRejection::ThreadMismatch { index });
            }
            let expected_sequence = snapshot
                .truncated_record_count
                .checked_add(index as u64)
                .ok_or(ConversationItemLifecycleHydrationRejection::SequenceExhausted)?;
            if record.sequence != expected_sequence {
                return Err(
                    ConversationItemLifecycleHydrationRejection::SequenceHistoryMismatch { index },
                );
            }
            if matches!(record.observation.kind, ConversationItemKind::Unknown(_)) {
                retained_unknown_kind_count = retained_unknown_kind_count.saturating_add(1);
            }
        }
        if retained_unknown_kind_count > snapshot.unknown_kind_record_count {
            return Err(ConversationItemLifecycleHydrationRejection::UnknownKindCounterRegression);
        }
        if snapshot.retained_dynamic_bytes() > MAX_RETAINED_CONVERSATION_ITEM_DYNAMIC_BYTES {
            return Err(ConversationItemLifecycleHydrationRejection::DynamicBytesExceeded);
        }

        let next_sequence = snapshot
            .truncated_record_count
            .checked_add(snapshot.records.len() as u64)
            .ok_or(ConversationItemLifecycleHydrationRejection::SequenceExhausted)?;
        Ok(Self {
            state: snapshot,
            next_sequence,
        })
    }

    pub fn apply_correlated(
        &mut self,
        expected_thread_id: Option<&str>,
        expected_turn_id: Option<&str>,
        observation: ConversationItemLifecycleObservation,
    ) -> Result<ConversationItemLifecycleConsistency, ConversationItemLifecycleRejection> {
        if expected_thread_id != Some(observation.thread_id.as_str()) {
            self.record_invalid_observation();
            return Err(ConversationItemLifecycleRejection::ThreadMismatch {
                expected_thread_id: expected_thread_id.map(str::to_string),
            });
        }
        if expected_turn_id != Some(observation.turn_id.as_str()) {
            self.record_invalid_observation();
            return Err(ConversationItemLifecycleRejection::TurnMismatch {
                expected_turn_id: expected_turn_id.map(str::to_string),
            });
        }
        self.apply(observation)
    }

    pub fn apply(
        &mut self,
        observation: ConversationItemLifecycleObservation,
    ) -> Result<ConversationItemLifecycleConsistency, ConversationItemLifecycleRejection> {
        observation
            .validate()
            .inspect_err(|_| self.record_invalid_observation())?;

        let consistency = self.classify_consistency(&observation);
        let unknown_kind = matches!(observation.kind, ConversationItemKind::Unknown(_));
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        let state = Arc::make_mut(&mut self.state);
        if unknown_kind {
            state.unknown_kind_record_count = state.unknown_kind_record_count.saturating_add(1);
        }
        if state.records.len() == MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS {
            state.records.remove(0);
            state.truncated_record_count = state.truncated_record_count.saturating_add(1);
        }
        state.records.push(ConversationItemLifecycleRecord {
            sequence,
            observation,
            consistency,
        });
        debug_assert!(
            state.retained_dynamic_bytes() <= MAX_RETAINED_CONVERSATION_ITEM_DYNAMIC_BYTES
        );
        Ok(consistency)
    }

    pub fn record_invalid_observation(&mut self) {
        let state = Arc::make_mut(&mut self.state);
        state.invalid_record_count = state.invalid_record_count.saturating_add(1);
    }

    pub fn snapshot(&self) -> Arc<ConversationItemLifecycleProjectionSnapshot> {
        Arc::clone(&self.state)
    }

    fn classify_consistency(
        &self,
        observation: &ConversationItemLifecycleObservation,
    ) -> ConversationItemLifecycleConsistency {
        let prior = self.state.records.iter().rev().filter(|record| {
            record.observation.thread_id == observation.thread_id
                && record.observation.turn_id == observation.turn_id
                && record.observation.item_id == observation.item_id
        });
        let prior = prior.collect::<Vec<_>>();

        if prior
            .iter()
            .any(|record| record.observation.kind != observation.kind)
        {
            return ConversationItemLifecycleConsistency::KindMismatch;
        }
        if timestamp_regressed(prior.as_slice(), observation) {
            return ConversationItemLifecycleConsistency::TimestampRegression;
        }

        match observation.phase {
            ConversationItemLifecyclePhase::SnapshotObserved => {
                ConversationItemLifecycleConsistency::SnapshotObserved
            }
            ConversationItemLifecyclePhase::Started => {
                if prior.iter().any(|record| {
                    record.observation.phase == ConversationItemLifecyclePhase::Started
                }) {
                    ConversationItemLifecycleConsistency::DuplicateStart
                } else if prior.iter().any(|record| {
                    record.observation.phase == ConversationItemLifecyclePhase::Completed
                }) {
                    ConversationItemLifecycleConsistency::StartAfterCompletion
                } else {
                    ConversationItemLifecycleConsistency::Accepted
                }
            }
            ConversationItemLifecyclePhase::Completed => {
                if prior.iter().any(|record| {
                    record.observation.phase == ConversationItemLifecyclePhase::Completed
                }) {
                    ConversationItemLifecycleConsistency::DuplicateCompletion
                } else if prior.iter().any(|record| {
                    record.observation.phase == ConversationItemLifecyclePhase::Started
                }) {
                    ConversationItemLifecycleConsistency::Accepted
                } else {
                    ConversationItemLifecycleConsistency::CompletionWithoutStart
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationItemLifecycleRejection {
    MissingIdentity { field: &'static str },
    IdentifierTooLong { field: &'static str },
    SummaryTooLong,
    InvalidOutcomeLabel,
    MissingLiveTimestamp,
    UnexpectedSnapshotTimestamp,
    SourcePhaseMismatch,
    ThreadMismatch { expected_thread_id: Option<String> },
    TurnMismatch { expected_turn_id: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationItemLifecycleHydrationRejection {
    TooManyRecords,
    InvalidRecord {
        index: usize,
        rejection: ConversationItemLifecycleRejection,
    },
    ThreadMismatch {
        index: usize,
    },
    SequenceHistoryMismatch {
        index: usize,
    },
    TruncationHistoryMismatch,
    UnknownKindCounterRegression,
    DynamicBytesExceeded,
    SequenceExhausted,
}

impl ConversationItemLifecycleRejection {
    pub const fn notice_label(&self) -> &'static str {
        match self {
            Self::MissingIdentity { .. } => "missing item lifecycle identity",
            Self::IdentifierTooLong { .. } => "oversized item lifecycle identity",
            Self::SummaryTooLong => "oversized item lifecycle summary",
            Self::InvalidOutcomeLabel => "invalid item lifecycle outcome",
            Self::MissingLiveTimestamp => "missing live item lifecycle timestamp",
            Self::UnexpectedSnapshotTimestamp => "unexpected snapshot item lifecycle timestamp",
            Self::SourcePhaseMismatch => "item lifecycle source and phase mismatch",
            Self::ThreadMismatch { .. } => "item lifecycle thread mismatch",
            Self::TurnMismatch { .. } => "item lifecycle turn mismatch",
        }
    }
}

fn validate_observation(
    observation: &ConversationItemLifecycleObservation,
) -> Result<(), ConversationItemLifecycleRejection> {
    validate_identifier("threadId", &observation.thread_id)?;
    validate_identifier("turnId", &observation.turn_id)?;
    validate_identifier("item.id", &observation.item_id)?;
    if let ConversationItemKind::Unknown(label) = &observation.kind {
        if label.is_empty() {
            return Err(ConversationItemLifecycleRejection::MissingIdentity { field: "item.type" });
        }
        if label.len() > MAX_CONVERSATION_ITEM_KIND_LABEL_BYTES {
            return Err(ConversationItemLifecycleRejection::IdentifierTooLong {
                field: "item.type",
            });
        }
    }
    if observation.summary.len() > MAX_CONVERSATION_ITEM_SUMMARY_BYTES {
        return Err(ConversationItemLifecycleRejection::SummaryTooLong);
    }
    if let ConversationItemOutcome::Unknown(label) = &observation.outcome
        && label.len() > MAX_CONVERSATION_ITEM_OUTCOME_LABEL_BYTES
    {
        return Err(ConversationItemLifecycleRejection::InvalidOutcomeLabel);
    }
    match (
        observation.source,
        observation.phase,
        observation.observed_at_ms,
    ) {
        (
            ConversationItemLifecycleSource::Live,
            ConversationItemLifecyclePhase::SnapshotObserved,
            _,
        )
        | (
            ConversationItemLifecycleSource::Snapshot,
            ConversationItemLifecyclePhase::Started | ConversationItemLifecyclePhase::Completed,
            _,
        ) => {
            return Err(ConversationItemLifecycleRejection::SourcePhaseMismatch);
        }
        (ConversationItemLifecycleSource::Live, _, None) => {
            return Err(ConversationItemLifecycleRejection::MissingLiveTimestamp);
        }
        (ConversationItemLifecycleSource::Snapshot, _, Some(_)) => {
            return Err(ConversationItemLifecycleRejection::UnexpectedSnapshotTimestamp);
        }
        _ => {}
    }
    Ok(())
}

fn validate_identifier(
    field: &'static str,
    value: &str,
) -> Result<(), ConversationItemLifecycleRejection> {
    if value.is_empty() {
        return Err(ConversationItemLifecycleRejection::MissingIdentity { field });
    }
    if value.len() > MAX_CONVERSATION_ITEM_IDENTIFIER_BYTES {
        return Err(ConversationItemLifecycleRejection::IdentifierTooLong { field });
    }
    Ok(())
}

fn timestamp_regressed(
    prior: &[&ConversationItemLifecycleRecord],
    observation: &ConversationItemLifecycleObservation,
) -> bool {
    let Some(observed_at_ms) = observation.observed_at_ms else {
        return false;
    };
    prior.iter().any(|record| {
        record
            .observation
            .observed_at_ms
            .is_some_and(|prior_timestamp| prior_timestamp > observed_at_ms)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(
        item_id: &str,
        phase: ConversationItemLifecyclePhase,
        timestamp: i64,
    ) -> ConversationItemLifecycleObservation {
        ConversationItemLifecycleObservation {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            item_id: item_id.to_string(),
            kind: ConversationItemKind::CommandExecution,
            phase,
            source: ConversationItemLifecycleSource::Live,
            observed_at_ms: Some(timestamp),
            outcome: ConversationItemOutcome::NotReported,
            summary: "command [redacted]".to_string(),
        }
    }

    #[test]
    fn lifecycle_projection_classifies_order_duplicates_and_regressions() {
        let mut projection = ConversationItemLifecycleProjection::default();

        assert_eq!(
            projection
                .apply(observation(
                    "item-1",
                    ConversationItemLifecyclePhase::Started,
                    10,
                ))
                .unwrap(),
            ConversationItemLifecycleConsistency::Accepted
        );
        assert_eq!(
            projection
                .apply(observation(
                    "item-1",
                    ConversationItemLifecyclePhase::Started,
                    11,
                ))
                .unwrap(),
            ConversationItemLifecycleConsistency::DuplicateStart
        );
        assert_eq!(
            projection
                .apply(observation(
                    "item-1",
                    ConversationItemLifecyclePhase::Completed,
                    12,
                ))
                .unwrap(),
            ConversationItemLifecycleConsistency::Accepted
        );
        assert_eq!(
            projection
                .apply(observation(
                    "item-1",
                    ConversationItemLifecyclePhase::Completed,
                    13,
                ))
                .unwrap(),
            ConversationItemLifecycleConsistency::DuplicateCompletion
        );
        assert_eq!(
            projection
                .apply(observation(
                    "item-1",
                    ConversationItemLifecyclePhase::Completed,
                    9,
                ))
                .unwrap(),
            ConversationItemLifecycleConsistency::TimestampRegression
        );
    }

    #[test]
    fn lifecycle_projection_is_memory_bounded_for_large_completed_stream() {
        let mut projection = ConversationItemLifecycleProjection::default();
        let total = 100_000usize;

        for index in 0..total {
            projection
                .apply(observation(
                    format!("item-{index}").as_str(),
                    ConversationItemLifecyclePhase::Completed,
                    index as i64,
                ))
                .unwrap();
        }

        let snapshot = projection.snapshot();
        assert_eq!(
            snapshot.records.len(),
            MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS
        );
        assert_eq!(
            snapshot.truncated_record_count,
            (total - MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS) as u64
        );
        assert_eq!(snapshot.records.first().unwrap().sequence, 99_744);
        assert_eq!(snapshot.records.last().unwrap().sequence, 99_999);
        assert!(!snapshot.is_complete());
    }

    #[test]
    fn lifecycle_projection_bounds_worst_case_retained_dynamic_bytes() {
        let mut projection = ConversationItemLifecycleProjection::default();
        let maximum_identifier = "i".repeat(MAX_CONVERSATION_ITEM_IDENTIFIER_BYTES);
        let maximum_kind = "k".repeat(MAX_CONVERSATION_ITEM_KIND_LABEL_BYTES);
        let maximum_outcome = "o".repeat(MAX_CONVERSATION_ITEM_OUTCOME_LABEL_BYTES);
        let maximum_summary = "s".repeat(MAX_CONVERSATION_ITEM_SUMMARY_BYTES);

        for index in 0..MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS * 2 {
            let mut item_id = index.to_string();
            item_id.push_str(
                &maximum_identifier[item_id.len()..MAX_CONVERSATION_ITEM_IDENTIFIER_BYTES],
            );
            projection
                .apply(ConversationItemLifecycleObservation {
                    thread_id: maximum_identifier.clone(),
                    turn_id: maximum_identifier.clone(),
                    item_id,
                    kind: ConversationItemKind::Unknown(maximum_kind.clone()),
                    phase: ConversationItemLifecyclePhase::Completed,
                    source: ConversationItemLifecycleSource::Live,
                    observed_at_ms: Some(index as i64),
                    outcome: ConversationItemOutcome::Unknown(maximum_outcome.clone()),
                    summary: maximum_summary.clone(),
                })
                .unwrap();
        }

        let snapshot = projection.snapshot();
        assert_eq!(
            snapshot.records.len(),
            MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS
        );
        assert_eq!(
            snapshot.retained_dynamic_bytes(),
            MAX_RETAINED_CONVERSATION_ITEM_DYNAMIC_BYTES
        );
    }

    #[test]
    fn lifecycle_projection_rejects_identity_mismatch_without_retaining_payload() {
        let mut projection = ConversationItemLifecycleProjection::default();
        let rejection = projection
            .apply_correlated(
                Some("thread-other"),
                Some("turn-1"),
                observation("item-1", ConversationItemLifecyclePhase::Started, 10),
            )
            .unwrap_err();

        assert!(matches!(
            rejection,
            ConversationItemLifecycleRejection::ThreadMismatch { .. }
        ));
        let snapshot = projection.snapshot();
        assert!(snapshot.records.is_empty());
        assert_eq!(snapshot.invalid_record_count, 1);
    }

    #[test]
    fn lifecycle_projection_distinguishes_unpaired_snapshot_and_kind_mismatch() {
        let mut live = ConversationItemLifecycleProjection::default();
        assert_eq!(
            live.apply(observation(
                "item-unpaired",
                ConversationItemLifecyclePhase::Completed,
                10,
            ))
            .unwrap(),
            ConversationItemLifecycleConsistency::CompletionWithoutStart
        );

        let mut snapshot = ConversationItemLifecycleProjection::default();
        let mut snapshot_observation = observation(
            "item-snapshot",
            ConversationItemLifecyclePhase::SnapshotObserved,
            10,
        );
        snapshot_observation.source = ConversationItemLifecycleSource::Snapshot;
        snapshot_observation.observed_at_ms = None;
        assert_eq!(
            snapshot.apply(snapshot_observation).unwrap(),
            ConversationItemLifecycleConsistency::SnapshotObserved
        );
        assert_eq!(
            snapshot
                .apply(observation(
                    "item-snapshot",
                    ConversationItemLifecyclePhase::Completed,
                    11,
                ))
                .unwrap(),
            ConversationItemLifecycleConsistency::CompletionWithoutStart
        );

        let mut mismatched = ConversationItemLifecycleProjection::default();
        mismatched
            .apply(observation(
                "item-kind",
                ConversationItemLifecyclePhase::Started,
                10,
            ))
            .unwrap();
        let mut completion =
            observation("item-kind", ConversationItemLifecyclePhase::Completed, 11);
        completion.kind = ConversationItemKind::FileChange;
        assert_eq!(
            mismatched.apply(completion).unwrap(),
            ConversationItemLifecycleConsistency::KindMismatch
        );
    }

    #[test]
    fn unknown_kind_marks_projection_incomplete_without_retaining_raw_payload() {
        let mut projection = ConversationItemLifecycleProjection::default();
        let mut unknown = observation(
            "item-unknown",
            ConversationItemLifecyclePhase::Completed,
            10,
        );
        unknown.kind = ConversationItemKind::Unknown("futureKind".to_string());
        unknown.summary = "unclassified item kind".to_string();

        projection.apply(unknown).unwrap();

        let snapshot = projection.snapshot();
        assert_eq!(snapshot.unknown_kind_record_count, 1);
        assert!(!snapshot.is_complete());
    }

    #[test]
    fn lifecycle_observation_enforces_source_timestamp_and_unknown_outcome_bounds() {
        let mut missing_live_timestamp =
            observation("item-live", ConversationItemLifecyclePhase::Started, 10);
        missing_live_timestamp.observed_at_ms = None;
        assert_eq!(
            missing_live_timestamp.validate(),
            Err(ConversationItemLifecycleRejection::MissingLiveTimestamp)
        );

        let mut timestamped_snapshot = observation(
            "item-snapshot",
            ConversationItemLifecyclePhase::SnapshotObserved,
            10,
        );
        timestamped_snapshot.source = ConversationItemLifecycleSource::Snapshot;
        assert_eq!(
            timestamped_snapshot.validate(),
            Err(ConversationItemLifecycleRejection::UnexpectedSnapshotTimestamp)
        );

        let mut live_snapshot_phase = observation(
            "item-live-snapshot-phase",
            ConversationItemLifecyclePhase::SnapshotObserved,
            10,
        );
        assert_eq!(
            live_snapshot_phase.validate(),
            Err(ConversationItemLifecycleRejection::SourcePhaseMismatch)
        );
        live_snapshot_phase.source = ConversationItemLifecycleSource::Snapshot;
        live_snapshot_phase.observed_at_ms = None;
        live_snapshot_phase.phase = ConversationItemLifecyclePhase::Completed;
        assert_eq!(
            live_snapshot_phase.validate(),
            Err(ConversationItemLifecycleRejection::SourcePhaseMismatch)
        );

        let mut oversized_outcome = observation(
            "item-outcome",
            ConversationItemLifecyclePhase::Completed,
            10,
        );
        oversized_outcome.outcome = ConversationItemOutcome::Unknown(
            "o".repeat(MAX_CONVERSATION_ITEM_OUTCOME_LABEL_BYTES + 1),
        );
        assert_eq!(
            oversized_outcome.validate(),
            Err(ConversationItemLifecycleRejection::InvalidOutcomeLabel)
        );
    }

    #[test]
    fn loaded_projection_reuses_valid_snapshot_and_restores_next_sequence() {
        let mut source = ConversationItemLifecycleProjection::default();
        for index in 0..MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS + 1 {
            source
                .apply(observation(
                    format!("item-{index}").as_str(),
                    ConversationItemLifecyclePhase::Completed,
                    index as i64,
                ))
                .unwrap();
        }
        let snapshot = source.snapshot();

        let mut loaded = ConversationItemLifecycleProjection::from_snapshot_for_thread(
            "thread-1",
            Arc::clone(&snapshot),
        )
        .unwrap();
        let hydrated = loaded.snapshot();

        assert!(Arc::ptr_eq(&hydrated, &snapshot));
        assert_eq!(hydrated.records.first().unwrap().sequence, 1);
        assert_eq!(hydrated.truncated_record_count, 1);

        loaded
            .apply(observation(
                "item-next",
                ConversationItemLifecyclePhase::Started,
                1_000,
            ))
            .unwrap();
        let appended = loaded.snapshot();
        assert_eq!(appended.records.last().unwrap().sequence, 257);
        assert_eq!(snapshot.records.last().unwrap().sequence, 256);
    }

    #[test]
    fn lifecycle_snapshot_is_shared_until_mutation_and_then_cows() {
        let mut projection = ConversationItemLifecycleProjection::default();
        projection
            .apply(observation(
                "item-1",
                ConversationItemLifecyclePhase::Started,
                1,
            ))
            .unwrap();
        let first = projection.snapshot();
        let unchanged = projection.snapshot();
        assert!(Arc::ptr_eq(&first, &unchanged));

        projection
            .apply(observation(
                "item-2",
                ConversationItemLifecyclePhase::Started,
                2,
            ))
            .unwrap();
        let second = projection.snapshot();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(first.records.len(), 1);
        assert_eq!(second.records.len(), 2);
    }

    #[test]
    fn loaded_projection_rejects_cross_thread_and_impossible_sequence_history() {
        let mut projection = ConversationItemLifecycleProjection::default();
        projection
            .apply(observation(
                "item-1",
                ConversationItemLifecyclePhase::Started,
                1,
            ))
            .unwrap();
        let snapshot = projection.snapshot();
        assert_eq!(
            ConversationItemLifecycleProjection::from_snapshot_for_thread(
                "thread-other",
                Arc::clone(&snapshot),
            )
            .unwrap_err(),
            ConversationItemLifecycleHydrationRejection::ThreadMismatch { index: 0 }
        );

        let mut non_monotonic = snapshot.as_ref().clone();
        non_monotonic.records.push(non_monotonic.records[0].clone());
        assert_eq!(
            ConversationItemLifecycleProjection::from_snapshot_for_thread(
                "thread-1",
                Arc::new(non_monotonic),
            )
            .unwrap_err(),
            ConversationItemLifecycleHydrationRejection::SequenceHistoryMismatch { index: 1 }
        );

        let empty_but_truncated = ConversationItemLifecycleProjectionSnapshot {
            truncated_record_count: 1,
            ..Default::default()
        };
        assert_eq!(
            ConversationItemLifecycleProjection::from_snapshot_for_thread(
                "thread-1",
                Arc::new(empty_but_truncated),
            )
            .unwrap_err(),
            ConversationItemLifecycleHydrationRejection::TruncationHistoryMismatch
        );
    }
}
