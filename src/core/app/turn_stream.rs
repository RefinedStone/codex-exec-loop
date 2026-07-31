use crate::domain::conversation::{
    ConversationApprovalRequest, ConversationApprovalRequestIdentity,
    ConversationApprovalResolution, ConversationApprovalReview, ConversationToolActivity,
};
#[cfg(test)]
use crate::domain::conversation_item_lifecycle::ConversationItemLifecycleHydrationRejection;
use crate::domain::conversation_item_lifecycle::{
    ConversationItemLifecycleConsistency, ConversationItemLifecycleObservation,
    ConversationItemLifecycleProjection, ConversationItemLifecycleProjectionSnapshot,
    ConversationItemLifecycleRejection,
};
use crate::domain::conversation_progressive_activity::{
    ConversationProgressiveActivityBatch, ConversationProgressiveActivityProjection,
    ConversationProgressiveActivityProjectionSnapshot, ConversationProgressiveActivityRejection,
};
use crate::domain::conversation_runtime_envelope::{
    ConversationRuntimeConfigurationRequest, ConversationRuntimeEnvelope,
    ConversationRuntimeEnvelopeObservation, ConversationRuntimeEnvelopeObservationRejection,
};
use crate::domain::planning::{PostTurnExecution, TurnSnapshotCapture};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
use crate::domain::turn_terminal::{
    ConversationTurnApplicationDelivery, ConversationTurnError, ConversationTurnTerminalOutcome,
    ConversationTurnTerminalReceipt,
};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::core) struct TurnStreamState {
    revision: u64,
    thread_id: Option<String>,
    title: Option<String>,
    cwd: Option<String>,
    runtime_envelope: Option<ConversationRuntimeEnvelope>,
    item_lifecycle: ConversationItemLifecycleProjection,
    progressive_activity: ConversationProgressiveActivityProjection,
    active_turn_id: Option<String>,
    pending_approval_identity: Option<ConversationApprovalRequestIdentity>,
    status_text: Option<String>,
    terminal: Option<TurnStreamTerminalSnapshot>,
    last_applied_post_turn_evaluation_id: Option<String>,
}

impl TurnStreamState {
    pub fn new() -> Self {
        Self {
            revision: 0,
            thread_id: None,
            title: None,
            cwd: None,
            runtime_envelope: None,
            item_lifecycle: ConversationItemLifecycleProjection::default(),
            progressive_activity: ConversationProgressiveActivityProjection::default(),
            active_turn_id: None,
            pending_approval_identity: None,
            status_text: None,
            terminal: None,
            last_applied_post_turn_evaluation_id: None,
        }
    }

    #[cfg(test)]
    pub fn seed_loaded_thread_identity(
        &mut self,
        thread_id: impl Into<String>,
        title: impl Into<String>,
        cwd: impl Into<String>,
    ) {
        let result = self.seed_loaded_thread(thread_id, title, cwd, Arc::default());
        debug_assert!(result.is_ok());
    }

    #[cfg(test)]
    pub fn seed_loaded_thread(
        &mut self,
        thread_id: impl Into<String>,
        title: impl Into<String>,
        cwd: impl Into<String>,
        item_lifecycle: Arc<ConversationItemLifecycleProjectionSnapshot>,
    ) -> Result<(), ConversationItemLifecycleHydrationRejection> {
        let thread_id = thread_id.into();
        let item_lifecycle = ConversationItemLifecycleProjection::from_snapshot_for_thread(
            &thread_id,
            item_lifecycle,
        )?;
        self.seed_loaded_thread_projection(thread_id, title, cwd, item_lifecycle);
        Ok(())
    }

    pub fn seed_loaded_thread_projection(
        &mut self,
        thread_id: impl Into<String>,
        title: impl Into<String>,
        cwd: impl Into<String>,
        item_lifecycle: ConversationItemLifecycleProjection,
    ) {
        self.thread_id = Some(thread_id.into());
        self.title = Some(title.into());
        self.cwd = Some(cwd.into());
        self.runtime_envelope = None;
        self.item_lifecycle = item_lifecycle;
        self.progressive_activity = ConversationProgressiveActivityProjection::default();
        self.active_turn_id = None;
        self.pending_approval_identity = None;
        self.status_text = None;
        self.terminal = None;
        self.last_applied_post_turn_evaluation_id = None;
    }

    pub fn begin_submission(&mut self) {
        self.active_turn_id = None;
        self.pending_approval_identity = None;
        self.status_text = Some("starting turn".to_string());
        self.terminal = None;
        self.last_applied_post_turn_evaluation_id = None;
    }

    pub fn matches_thread(&self, thread_id: &str) -> bool {
        self.thread_id.as_deref() == Some(thread_id)
    }

    pub fn matches_conversation(&self, workspace_directory: &str, thread_id: &str) -> bool {
        self.thread_id.as_deref() == Some(thread_id)
            && self.cwd.as_deref() == Some(workspace_directory)
    }

    pub fn matches_active_turn(&self, thread_id: &str, turn_id: &str) -> bool {
        self.thread_id.as_deref() == Some(thread_id)
            && self.active_turn_id.as_deref() == Some(turn_id)
    }

    pub const fn has_active_turn(&self) -> bool {
        self.active_turn_id.is_some()
    }

    pub fn matches_pending_approval(
        &self,
        request_identity: &ConversationApprovalRequestIdentity,
    ) -> bool {
        self.pending_approval_identity.as_ref() == Some(request_identity)
    }

    pub fn apply_session_rename(
        &mut self,
        thread_id: &str,
        title: &str,
    ) -> Option<TurnStreamSnapshot> {
        if self.thread_id.as_deref() != Some(thread_id) || self.title.as_deref() == Some(title) {
            return None;
        }
        self.title = Some(title.to_string());
        Some(self.snapshot(TurnStreamUpdate::SessionRenamed {
            thread_id: thread_id.to_string(),
            title: title.to_string(),
        }))
    }

    pub fn apply_stream_event(&mut self, event: TurnStreamEvent) -> TurnStreamSnapshot {
        let update = match event {
            TurnStreamEvent::AttachmentObserved { profile } => {
                TurnStreamUpdate::AttachmentObserved { profile }
            }
            TurnStreamEvent::ThreadPrepared {
                thread_id,
                title,
                cwd,
                runtime_envelope,
            } => {
                let thread_changed = self.thread_id.as_deref() != Some(thread_id.as_str());
                self.thread_id = Some(thread_id.clone());
                self.title = Some(title.clone());
                self.cwd = Some(cwd.clone());
                self.runtime_envelope = Some(*runtime_envelope);
                if thread_changed {
                    self.item_lifecycle = ConversationItemLifecycleProjection::default();
                }
                self.progressive_activity = ConversationProgressiveActivityProjection::default();
                self.active_turn_id = None;
                self.pending_approval_identity = None;
                self.terminal = None;
                self.last_applied_post_turn_evaluation_id = None;
                self.status_text = Some("thread started".to_string());
                TurnStreamUpdate::ThreadPrepared {
                    thread_id,
                    title,
                    cwd,
                    status_text: "thread started".to_string(),
                }
            }
            TurnStreamEvent::TurnStarted {
                turn_id,
                runtime_request,
            } => {
                if let Some(expected_turn_id) = self.active_turn_id.as_ref() {
                    let rejection = if expected_turn_id == &turn_id {
                        TurnStreamStartRejection::Duplicate
                    } else {
                        TurnStreamStartRejection::TurnMismatch {
                            expected_turn_id: expected_turn_id.clone(),
                        }
                    };
                    TurnStreamUpdate::TurnStartedIgnored { turn_id, rejection }
                } else {
                    self.active_turn_id = Some(turn_id.clone());
                    self.pending_approval_identity = None;
                    self.progressive_activity =
                        ConversationProgressiveActivityProjection::default();
                    self.runtime_envelope
                        .get_or_insert_with(ConversationRuntimeEnvelope::unobserved)
                        .record_turn_request(*runtime_request);
                    self.terminal = None;
                    self.last_applied_post_turn_evaluation_id = None;
                    self.status_text = Some("turn started".to_string());
                    TurnStreamUpdate::TurnStarted {
                        turn_id,
                        status_text: "turn started".to_string(),
                    }
                }
            }
            TurnStreamEvent::RuntimeEnvelopeObserved { observation } => {
                self.runtime_envelope_observed_update(*observation)
            }
            TurnStreamEvent::ItemLifecycleObserved { observation } => {
                self.item_lifecycle_observed_update(*observation)
            }
            TurnStreamEvent::ProgressiveActivityObserved { batch } => {
                self.progressive_activity_observed_update(*batch)
            }
            TurnStreamEvent::StatusUpdated { text } => {
                self.status_text = Some(text.clone());
                TurnStreamUpdate::StatusUpdated { text }
            }
            TurnStreamEvent::AgentMessageCompleted {
                item_id,
                phase,
                text,
            } => TurnStreamUpdate::AgentMessageCompleted {
                item_id,
                phase,
                text,
            },
            TurnStreamEvent::ToolActivity { activity } => {
                TurnStreamUpdate::ToolActivity { activity }
            }
            TurnStreamEvent::ApprovalReviewUpdated { review } => {
                TurnStreamUpdate::ApprovalReviewUpdated { review }
            }
            TurnStreamEvent::ApprovalRequested { request } => {
                self.pending_approval_identity = Some(request.identity());
                self.status_text = Some("approval required".to_string());
                TurnStreamUpdate::ApprovalRequested { request }
            }
            TurnStreamEvent::ApprovalResolved {
                request_identity,
                resolution,
            } => {
                if self.pending_approval_identity.as_ref() == Some(&request_identity) {
                    self.pending_approval_identity = None;
                    TurnStreamUpdate::ApprovalResolved {
                        request_identity,
                        resolution,
                    }
                } else {
                    TurnStreamUpdate::ApprovalResolutionIgnored {
                        request_identity,
                        resolution,
                    }
                }
            }
            TurnStreamEvent::TurnInterruptRequestFailed { message } => {
                self.status_text = Some(message.clone());
                TurnStreamUpdate::TurnInterruptRequestFailed { message }
            }
            TurnStreamEvent::TurnRetrying {
                thread_id,
                turn_id,
                error,
            } => self.turn_retrying_update(thread_id, turn_id, error),
            TurnStreamEvent::TurnTerminal {
                receipt,
                execution_snapshot_capture,
            } => self.turn_terminal_update(receipt, execution_snapshot_capture),
            TurnStreamEvent::Failed { message } => {
                if self.terminal.is_some() {
                    TurnStreamUpdate::RuntimeFailureIgnored { message }
                } else {
                    self.active_turn_id = None;
                    self.pending_approval_identity = None;
                    self.status_text = Some("turn failed".to_string());
                    self.terminal = Some(TurnStreamTerminalSnapshot::Failed {
                        message: message.clone(),
                    });
                    TurnStreamUpdate::Failed {
                        message,
                        status_text: "turn failed".to_string(),
                    }
                }
            }
        };

        self.snapshot(update)
    }

    #[cfg(test)]
    pub fn apply_turn_completed(
        &mut self,
        turn_id: String,
        changed_planning_file_paths: Vec<String>,
        execution_snapshot_capture: TurnSnapshotCapture,
    ) -> TurnStreamSnapshot {
        let thread_id = self.thread_id.clone().unwrap_or_default();
        let receipt = ConversationTurnTerminalReceipt::completed(
            thread_id,
            turn_id,
            changed_planning_file_paths,
        )
        .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);
        let update = if self.thread_id.is_none() && self.active_turn_id.is_none() {
            self.apply_terminal_receipt(receipt, Some(execution_snapshot_capture))
        } else {
            self.turn_terminal_update(receipt, Some(execution_snapshot_capture))
        };
        self.snapshot(update)
    }

    pub fn apply_runtime_notice(&mut self, notice: String) -> TurnStreamSnapshot {
        self.snapshot(TurnStreamUpdate::RuntimeNotice { notice })
    }

    pub fn can_start_post_turn_evaluation(&self, thread_id: &str, completed_turn_id: &str) -> bool {
        self.thread_id.as_deref() == Some(thread_id)
            && self.active_turn_id.is_none()
            && self.last_applied_post_turn_evaluation_id.as_deref() != Some(completed_turn_id)
            && matches!(
                &self.terminal,
                Some(TurnStreamTerminalSnapshot::Turn { receipt })
                    if receipt.turn_id == completed_turn_id
                        && receipt.is_completed_and_confirmed()
            )
    }

    pub fn has_unapplied_confirmed_terminal(&self) -> bool {
        matches!(
            &self.terminal,
            Some(TurnStreamTerminalSnapshot::Turn { receipt })
                if receipt.is_completed_and_confirmed()
                    && self.last_applied_post_turn_evaluation_id.as_deref()
                        != Some(receipt.turn_id.as_str())
        )
    }

    pub fn accept_post_turn_evaluation_completion(
        &mut self,
        execution: &PostTurnExecution,
    ) -> bool {
        if !self.settle_post_turn_terminal(&execution.thread_id, &execution.completed_turn_id) {
            return false;
        }
        true
    }

    pub fn settle_post_turn_terminal(&mut self, thread_id: &str, completed_turn_id: &str) -> bool {
        if !self.can_start_post_turn_evaluation(thread_id, completed_turn_id) {
            return false;
        }
        self.last_applied_post_turn_evaluation_id = Some(completed_turn_id.to_string());
        true
    }

    fn turn_retrying_update(
        &mut self,
        thread_id: String,
        turn_id: String,
        error: ConversationTurnError,
    ) -> TurnStreamUpdate {
        let correlation_failure = self.correlation_failure(&thread_id, &turn_id);
        if correlation_failure.is_none() && self.terminal.is_none() {
            self.status_text = Some("turn retrying".to_string());
        }
        TurnStreamUpdate::TurnRetrying {
            thread_id,
            turn_id,
            error,
            correlation_failure,
            status_text: "turn retrying".to_string(),
        }
    }

    fn runtime_envelope_observed_update(
        &mut self,
        observation: ConversationRuntimeEnvelopeObservation,
    ) -> TurnStreamUpdate {
        let rejection = match self.runtime_envelope.as_mut() {
            Some(envelope) => envelope
                .apply_correlated_observation(
                    self.thread_id.as_deref(),
                    self.active_turn_id.as_deref(),
                    &observation,
                )
                .err()
                .map(TurnStreamRuntimeEnvelopeRejection::from),
            None => Some(TurnStreamRuntimeEnvelopeRejection::EnvelopeNotPrepared),
        };
        TurnStreamUpdate::RuntimeEnvelopeObserved {
            observation: Box::new(observation),
            rejection,
        }
    }

    fn item_lifecycle_observed_update(
        &mut self,
        observation: ConversationItemLifecycleObservation,
    ) -> TurnStreamUpdate {
        let result = self.item_lifecycle.apply_correlated(
            self.thread_id.as_deref(),
            self.active_turn_id.as_deref(),
            observation.clone(),
        );
        TurnStreamUpdate::ItemLifecycleObserved {
            observation: Box::new(observation),
            consistency: result.as_ref().ok().copied(),
            rejection: result.err(),
        }
    }

    fn progressive_activity_observed_update(
        &mut self,
        batch: ConversationProgressiveActivityBatch,
    ) -> TurnStreamUpdate {
        let activity = TurnStreamProgressiveActivityUpdate::from_batch(&batch);
        let rejection = self
            .progressive_activity
            .apply_batch_correlated(
                self.thread_id.as_deref(),
                self.active_turn_id.as_deref(),
                batch,
            )
            .err();
        TurnStreamUpdate::ProgressiveActivityObserved {
            activity,
            rejection,
        }
    }

    fn turn_terminal_update(
        &mut self,
        receipt: ConversationTurnTerminalReceipt,
        execution_snapshot_capture: Option<TurnSnapshotCapture>,
    ) -> TurnStreamUpdate {
        if self.terminal.is_some() {
            return TurnStreamUpdate::TurnTerminalIgnored {
                receipt: Box::new(receipt),
                reason: TurnStreamTerminalRejection::TerminalAlreadyApplied,
            };
        }
        if let Some(reason) = self.correlation_failure(&receipt.thread_id, &receipt.turn_id) {
            return TurnStreamUpdate::TurnTerminalIgnored {
                receipt: Box::new(receipt),
                reason,
            };
        }
        self.apply_terminal_receipt(receipt, execution_snapshot_capture)
    }

    fn apply_terminal_receipt(
        &mut self,
        receipt: ConversationTurnTerminalReceipt,
        execution_snapshot_capture: Option<TurnSnapshotCapture>,
    ) -> TurnStreamUpdate {
        self.active_turn_id = None;
        self.pending_approval_identity = None;
        let status_text = terminal_status_text(&receipt).to_string();
        self.status_text = Some(status_text.clone());
        self.terminal = Some(TurnStreamTerminalSnapshot::Turn {
            receipt: Box::new(receipt.clone()),
        });
        if receipt.is_completed_and_confirmed() {
            TurnStreamUpdate::TurnCompleted {
                turn_id: receipt.turn_id.clone(),
                changed_planning_file_paths: receipt
                    .observations
                    .changed_planning_file_paths
                    .clone(),
                execution_snapshot_capture,
                status_text,
            }
        } else {
            TurnStreamUpdate::TurnTerminal {
                receipt: Box::new(receipt),
                execution_snapshot_capture,
                status_text,
            }
        }
    }

    fn correlation_failure(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Option<TurnStreamTerminalRejection> {
        if self.thread_id.as_deref() != Some(thread_id) {
            return Some(TurnStreamTerminalRejection::ThreadMismatch {
                expected_thread_id: self.thread_id.clone(),
            });
        }
        if self.active_turn_id.as_deref() != Some(turn_id) {
            return Some(TurnStreamTerminalRejection::TurnMismatch {
                expected_turn_id: self.active_turn_id.clone(),
            });
        }
        None
    }

    fn snapshot(&mut self, update: TurnStreamUpdate) -> TurnStreamSnapshot {
        self.revision += 1;
        TurnStreamSnapshot {
            revision: self.revision,
            thread_id: self.thread_id.clone(),
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            runtime_envelope: self.runtime_envelope.clone().map(Box::new),
            item_lifecycle: self.item_lifecycle.snapshot(),
            progressive_activity: self.progressive_activity.snapshot(),
            active_turn_id: self.active_turn_id.clone(),
            status_text: self.status_text.clone(),
            terminal: self.terminal.clone(),
            update,
        }
    }
}

impl Default for TurnStreamState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
pub(crate) struct TurnStreamTestHarness(TurnStreamState);

#[cfg(test)]
impl TurnStreamTestHarness {
    pub(crate) fn new() -> Self {
        Self(TurnStreamState::new())
    }

    pub(crate) fn seed_loaded_thread_identity(
        &mut self,
        thread_id: impl Into<String>,
        title: impl Into<String>,
        cwd: impl Into<String>,
    ) {
        self.0.seed_loaded_thread_identity(thread_id, title, cwd);
    }

    pub(crate) fn apply_session_rename(
        &mut self,
        thread_id: &str,
        title: &str,
    ) -> Option<TurnStreamSnapshot> {
        self.0.apply_session_rename(thread_id, title)
    }

    pub(crate) fn apply_stream_event(&mut self, event: TurnStreamEvent) -> TurnStreamSnapshot {
        self.0.apply_stream_event(event)
    }

    pub(crate) fn apply_turn_completed(
        &mut self,
        turn_id: String,
        changed_paths: Vec<String>,
        execution_snapshot_capture: TurnSnapshotCapture,
    ) -> TurnStreamSnapshot {
        self.0
            .apply_turn_completed(turn_id, changed_paths, execution_snapshot_capture)
    }

    pub(crate) fn apply_runtime_notice(&mut self, notice: String) -> TurnStreamSnapshot {
        self.0.apply_runtime_notice(notice)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnStreamSnapshot {
    pub revision: u64,
    pub thread_id: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub runtime_envelope: Option<Box<ConversationRuntimeEnvelope>>,
    pub item_lifecycle: Arc<ConversationItemLifecycleProjectionSnapshot>,
    pub progressive_activity: Arc<ConversationProgressiveActivityProjectionSnapshot>,
    pub active_turn_id: Option<String>,
    pub status_text: Option<String>,
    pub terminal: Option<TurnStreamTerminalSnapshot>,
    pub update: TurnStreamUpdate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnStreamProgressiveActivityUpdate {
    pub first_sequence: Option<u64>,
    pub last_sequence: Option<u64>,
    pub source_observation_count: u64,
    pub superseded_publication_count: u64,
    pub payload_truncation_count: u64,
    pub dropped_observation_count: u64,
    pub invalid_observation_count: u64,
    pub unknown_observation_count: u64,
}

impl TurnStreamProgressiveActivityUpdate {
    fn from_batch(batch: &ConversationProgressiveActivityBatch) -> Self {
        Self {
            first_sequence: batch.first_sequence(),
            last_sequence: batch.last_sequence(),
            source_observation_count: batch.source_observation_count(),
            superseded_publication_count: batch.superseded_publication_count(),
            payload_truncation_count: batch.payload_truncation_count(),
            dropped_observation_count: batch.dropped_observation_count(),
            invalid_observation_count: batch.invalid_observation_count(),
            unknown_observation_count: batch.unknown_observation_count(),
        }
    }

    pub const fn history_incomplete(&self) -> bool {
        self.superseded_publication_count > 0
            || self.payload_truncation_count > 0
            || self.dropped_observation_count > 0
            || self.invalid_observation_count > 0
            || self.unknown_observation_count > 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStreamEvent {
    AttachmentObserved {
        profile: TerminalBridgeAttachmentProfile,
    },
    ThreadPrepared {
        thread_id: String,
        title: String,
        cwd: String,
        runtime_envelope: Box<ConversationRuntimeEnvelope>,
    },
    TurnStarted {
        turn_id: String,
        runtime_request: Box<ConversationRuntimeConfigurationRequest>,
    },
    RuntimeEnvelopeObserved {
        observation: Box<ConversationRuntimeEnvelopeObservation>,
    },
    ItemLifecycleObserved {
        observation: Box<ConversationItemLifecycleObservation>,
    },
    ProgressiveActivityObserved {
        batch: Box<ConversationProgressiveActivityBatch>,
    },
    StatusUpdated {
        text: String,
    },
    AgentMessageCompleted {
        item_id: String,
        phase: Option<String>,
        text: String,
    },
    ToolActivity {
        activity: ConversationToolActivity,
    },
    ApprovalReviewUpdated {
        review: ConversationApprovalReview,
    },
    ApprovalRequested {
        request: ConversationApprovalRequest,
    },
    ApprovalResolved {
        request_identity: ConversationApprovalRequestIdentity,
        resolution: ConversationApprovalResolution,
    },
    TurnInterruptRequestFailed {
        message: String,
    },
    TurnRetrying {
        thread_id: String,
        turn_id: String,
        error: ConversationTurnError,
    },
    TurnTerminal {
        receipt: ConversationTurnTerminalReceipt,
        execution_snapshot_capture: Option<TurnSnapshotCapture>,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStreamTerminalSnapshot {
    Turn {
        receipt: Box<ConversationTurnTerminalReceipt>,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStreamUpdate {
    AttachmentObserved {
        profile: TerminalBridgeAttachmentProfile,
    },
    SessionRenamed {
        thread_id: String,
        title: String,
    },
    ThreadPrepared {
        thread_id: String,
        title: String,
        cwd: String,
        status_text: String,
    },
    TurnStarted {
        turn_id: String,
        status_text: String,
    },
    TurnStartedIgnored {
        turn_id: String,
        rejection: TurnStreamStartRejection,
    },
    RuntimeEnvelopeObserved {
        observation: Box<ConversationRuntimeEnvelopeObservation>,
        rejection: Option<TurnStreamRuntimeEnvelopeRejection>,
    },
    ItemLifecycleObserved {
        observation: Box<ConversationItemLifecycleObservation>,
        consistency: Option<ConversationItemLifecycleConsistency>,
        rejection: Option<ConversationItemLifecycleRejection>,
    },
    ProgressiveActivityObserved {
        activity: TurnStreamProgressiveActivityUpdate,
        rejection: Option<ConversationProgressiveActivityRejection>,
    },
    StatusUpdated {
        text: String,
    },
    AgentMessageCompleted {
        item_id: String,
        phase: Option<String>,
        text: String,
    },
    ToolActivity {
        activity: ConversationToolActivity,
    },
    ApprovalReviewUpdated {
        review: ConversationApprovalReview,
    },
    ApprovalRequested {
        request: ConversationApprovalRequest,
    },
    ApprovalResolved {
        request_identity: ConversationApprovalRequestIdentity,
        resolution: ConversationApprovalResolution,
    },
    ApprovalResolutionIgnored {
        request_identity: ConversationApprovalRequestIdentity,
        resolution: ConversationApprovalResolution,
    },
    TurnInterruptRequestFailed {
        message: String,
    },
    TurnRetrying {
        thread_id: String,
        turn_id: String,
        error: ConversationTurnError,
        correlation_failure: Option<TurnStreamTerminalRejection>,
        status_text: String,
    },
    TurnCompleted {
        turn_id: String,
        changed_planning_file_paths: Vec<String>,
        execution_snapshot_capture: Option<TurnSnapshotCapture>,
        status_text: String,
    },
    TurnTerminal {
        receipt: Box<ConversationTurnTerminalReceipt>,
        execution_snapshot_capture: Option<TurnSnapshotCapture>,
        status_text: String,
    },
    TurnTerminalIgnored {
        receipt: Box<ConversationTurnTerminalReceipt>,
        reason: TurnStreamTerminalRejection,
    },
    Failed {
        message: String,
        status_text: String,
    },
    RuntimeFailureIgnored {
        message: String,
    },
    RuntimeNotice {
        notice: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStreamStartRejection {
    Duplicate,
    TurnMismatch { expected_turn_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStreamTerminalRejection {
    ThreadMismatch { expected_thread_id: Option<String> },
    TurnMismatch { expected_turn_id: Option<String> },
    TerminalAlreadyApplied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStreamRuntimeEnvelopeRejection {
    EnvelopeNotPrepared,
    ThreadMismatch { expected_thread_id: Option<String> },
    TurnMismatch { expected_turn_id: Option<String> },
}

impl TurnStreamRuntimeEnvelopeRejection {
    pub const fn notice_label(&self) -> &'static str {
        match self {
            Self::EnvelopeNotPrepared => "envelope not prepared",
            Self::ThreadMismatch { .. } => "thread mismatch",
            Self::TurnMismatch { .. } => "turn mismatch",
        }
    }
}

impl From<ConversationRuntimeEnvelopeObservationRejection> for TurnStreamRuntimeEnvelopeRejection {
    fn from(rejection: ConversationRuntimeEnvelopeObservationRejection) -> Self {
        match rejection {
            ConversationRuntimeEnvelopeObservationRejection::Thread { expected_thread_id } => {
                Self::ThreadMismatch { expected_thread_id }
            }
            ConversationRuntimeEnvelopeObservationRejection::Turn { expected_turn_id } => {
                Self::TurnMismatch { expected_turn_id }
            }
        }
    }
}

fn terminal_status_text(receipt: &ConversationTurnTerminalReceipt) -> &'static str {
    if receipt.application_delivery != ConversationTurnApplicationDelivery::Confirmed {
        return "turn recovery pending";
    }
    match &receipt.outcome {
        ConversationTurnTerminalOutcome::Completed => "turn completed",
        ConversationTurnTerminalOutcome::Interrupted => "turn interrupted",
        ConversationTurnTerminalOutcome::Failed { .. } => "turn failed",
        ConversationTurnTerminalOutcome::Unknown { .. } => "turn outcome unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversation::{
        ConversationApprovalRequest, ConversationApprovalRequestKind,
        ConversationApprovalResolution, ConversationApprovalReview,
        ConversationApprovalReviewStatus, ConversationToolActivity, ConversationToolActivityKind,
    };
    use crate::domain::conversation_progressive_activity::{
        ConversationProgressiveActivityKind, ConversationProgressiveActivityObservation,
        ConversationProgressiveActivityPayload,
    };
    use crate::domain::conversation_runtime_envelope::{
        ConversationRuntimeConfigurationObservation, ConversationRuntimeLaunchEnvironment,
        ConversationRuntimeModelReroute, ConversationRuntimeModelRerouteReason,
        ConversationRuntimeObservationGap, ConversationRuntimeObservedValue,
        ConversationRuntimeRequestedValue, ConversationRuntimeThreadSource,
        ConversationRuntimeThreadStatus,
    };
    use crate::domain::planning::{ExecutionSnapshot, TurnSnapshotCapture};
    use crate::domain::turn_terminal::{
        ConversationTurnApplicationDeliveryFailure, ConversationTurnTerminalUncertainty,
    };

    fn prepared_turn_state() -> TurnStreamState {
        let mut state = TurnStreamState::new();
        state.apply_stream_event(TurnStreamEvent::ThreadPrepared {
            thread_id: "thread-1".to_string(),
            title: "Core stream".to_string(),
            cwd: "/tmp/workspace".to_string(),
            runtime_envelope: Box::default(),
        });
        state.apply_stream_event(TurnStreamEvent::TurnStarted {
            turn_id: "turn-1".to_string(),
            runtime_request: Box::default(),
        });
        state
    }

    fn item_lifecycle_observation(
        thread_id: &str,
        turn_id: &str,
        item_id: &str,
        phase: crate::domain::conversation_item_lifecycle::ConversationItemLifecyclePhase,
    ) -> ConversationItemLifecycleObservation {
        ConversationItemLifecycleObservation {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            item_id: item_id.to_string(),
            kind:
                crate::domain::conversation_item_lifecycle::ConversationItemKind::CommandExecution,
            phase,
            source:
                crate::domain::conversation_item_lifecycle::ConversationItemLifecycleSource::Live,
            observed_at_ms: Some(10),
            outcome:
                crate::domain::conversation_item_lifecycle::ConversationItemOutcome::InProgress,
            summary: "command bytes=10; status=inProgress".to_string(),
            command_actions: Default::default(),
        }
    }

    fn progressive_agent_batch(
        sequence: u64,
        thread_id: &str,
        turn_id: &str,
        item_id: &str,
        text: &str,
    ) -> ConversationProgressiveActivityBatch {
        ConversationProgressiveActivityBatch::single(ConversationProgressiveActivityObservation {
            sequence,
            thread_id: thread_id.to_string(),
            turn_id: Some(turn_id.to_string()),
            item_id: Some(item_id.to_string()),
            kind: ConversationProgressiveActivityKind::AgentMessageDelta,
            payload: ConversationProgressiveActivityPayload::AgentMessageDelta {
                phase: Some("output".to_string()),
                text: text.to_string(),
                source_bytes: text.len() as u64,
                truncated_bytes: 0,
            },
        })
        .expect("progressive agent batch should be valid")
    }

    fn runtime_envelope_with_applied_model(model: &str) -> ConversationRuntimeEnvelope {
        ConversationRuntimeEnvelope::prepared(
            ConversationRuntimeConfigurationRequest::default(),
            crate::domain::conversation_runtime_envelope::ConversationRuntimeConfigurationObservation {
                model: ConversationRuntimeObservedValue::Observed(model.to_string()),
                source: ConversationRuntimeObservedValue::Observed(
                    ConversationRuntimeThreadSource::AppServer,
                ),
                ..Default::default()
            },
            ConversationRuntimeLaunchEnvironment::unknown(),
            ConversationRuntimeObservedValue::Observed(ConversationRuntimeThreadStatus::Idle),
        )
    }

    fn prepared_runtime_envelope_state(
        applied_model: &str,
        requested_model: &str,
    ) -> TurnStreamState {
        let mut state = TurnStreamState::new();
        state.apply_stream_event(TurnStreamEvent::ThreadPrepared {
            thread_id: "thread-envelope".to_string(),
            title: "Envelope".to_string(),
            cwd: "/repo".to_string(),
            runtime_envelope: Box::new(runtime_envelope_with_applied_model(applied_model)),
        });
        state.apply_stream_event(TurnStreamEvent::TurnStarted {
            turn_id: "turn-envelope".to_string(),
            runtime_request: Box::new(ConversationRuntimeConfigurationRequest {
                model: ConversationRuntimeRequestedValue::Value(requested_model.to_string()),
                ..Default::default()
            }),
        });
        state
    }

    fn confirmed_receipt(
        outcome: ConversationTurnTerminalOutcome,
    ) -> ConversationTurnTerminalReceipt {
        ConversationTurnTerminalReceipt::new("thread-1", "turn-1", outcome)
            .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed)
    }

    #[test]
    fn thread_prepared_updates_stream_identity() {
        let mut state = TurnStreamState::new();

        let snapshot = state.apply_stream_event(TurnStreamEvent::ThreadPrepared {
            thread_id: "thread-1".to_string(),
            title: "Core stream".to_string(),
            cwd: "/tmp/workspace".to_string(),
            runtime_envelope: Box::default(),
        });

        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(snapshot.title.as_deref(), Some("Core stream"));
        assert_eq!(snapshot.cwd.as_deref(), Some("/tmp/workspace"));
        assert_eq!(snapshot.status_text.as_deref(), Some("thread started"));
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Core stream".to_string(),
                cwd: "/tmp/workspace".to_string(),
                status_text: "thread started".to_string(),
            }
        );
    }

    #[test]
    fn loaded_thread_identity_is_seeded_without_emitting_a_stream_revision() {
        let mut state = TurnStreamState::new();

        state.seed_loaded_thread_identity("thread-loaded", "Loaded thread", "/tmp/loaded");
        let snapshot = state.apply_runtime_notice("reattached".to_string());

        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.thread_id.as_deref(), Some("thread-loaded"));
        assert_eq!(snapshot.title.as_deref(), Some("Loaded thread"));
        assert_eq!(snapshot.cwd.as_deref(), Some("/tmp/loaded"));
        assert_eq!(snapshot.status_text, None);
    }

    #[test]
    fn turn_start_is_exact_once_and_conflicting_identity_is_fail_closed_input() {
        let mut state = TurnStreamState::new();
        state.apply_stream_event(TurnStreamEvent::ThreadPrepared {
            thread_id: "thread-1".to_string(),
            title: "Core stream".to_string(),
            cwd: "/tmp/workspace".to_string(),
            runtime_envelope: Box::default(),
        });
        let accepted = state.apply_stream_event(TurnStreamEvent::TurnStarted {
            turn_id: "turn-1".to_string(),
            runtime_request: Box::default(),
        });
        assert!(matches!(
            accepted.update,
            TurnStreamUpdate::TurnStarted { ref turn_id, .. } if turn_id == "turn-1"
        ));

        let duplicate = state.apply_stream_event(TurnStreamEvent::TurnStarted {
            turn_id: "turn-1".to_string(),
            runtime_request: Box::default(),
        });
        assert!(matches!(
            duplicate.update,
            TurnStreamUpdate::TurnStartedIgnored {
                ref turn_id,
                rejection: TurnStreamStartRejection::Duplicate,
            } if turn_id == "turn-1"
        ));
        assert_eq!(duplicate.active_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(duplicate.status_text.as_deref(), Some("turn started"));

        let conflicting = state.apply_stream_event(TurnStreamEvent::TurnStarted {
            turn_id: "turn-forged".to_string(),
            runtime_request: Box::default(),
        });
        assert!(matches!(
            conflicting.update,
            TurnStreamUpdate::TurnStartedIgnored {
                ref turn_id,
                rejection: TurnStreamStartRejection::TurnMismatch {
                    ref expected_turn_id,
                },
            } if turn_id == "turn-forged" && expected_turn_id == "turn-1"
        ));
        assert_eq!(conflicting.active_turn_id.as_deref(), Some("turn-1"));
    }

    #[test]
    fn session_rename_updates_only_matching_stream_identity() {
        let mut state = TurnStreamState::new();
        state.seed_loaded_thread_identity("thread-loaded", "Loaded thread", "/tmp/loaded");

        assert!(
            state
                .apply_session_rename("thread-other", "Other title")
                .is_none()
        );
        let snapshot = state
            .apply_session_rename("thread-loaded", "Renamed thread")
            .expect("matching rename should publish a stream snapshot");
        assert!(
            state
                .apply_session_rename("thread-loaded", "Renamed thread")
                .is_none()
        );

        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.title.as_deref(), Some("Renamed thread"));
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::SessionRenamed {
                thread_id: "thread-loaded".to_string(),
                title: "Renamed thread".to_string(),
            }
        );
    }

    #[test]
    fn turn_started_records_active_turn() {
        let mut state = TurnStreamState::new();

        let snapshot = state.apply_stream_event(TurnStreamEvent::TurnStarted {
            turn_id: "turn-1".to_string(),
            runtime_request: Box::default(),
        });

        assert_eq!(snapshot.active_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(snapshot.status_text.as_deref(), Some("turn started"));
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::TurnStarted {
                turn_id: "turn-1".to_string(),
                status_text: "turn started".to_string(),
            }
        );
    }

    #[test]
    fn turn_request_never_overwrites_applied_thread_envelope() {
        let state = prepared_runtime_envelope_state("applied-model", "requested-model");
        let snapshot = state
            .runtime_envelope
            .expect("envelope should remain prepared");

        assert_eq!(
            snapshot.applied.model,
            ConversationRuntimeObservedValue::Observed("applied-model".to_string())
        );
        assert_eq!(
            snapshot
                .turn_request
                .as_ref()
                .and_then(|request| request.model.as_value())
                .map(String::as_str),
            Some("requested-model")
        );
        assert_eq!(snapshot.observation_sequence, 0);
    }

    #[test]
    fn correlated_settings_and_status_apply_in_fifo_order() {
        let mut state = prepared_runtime_envelope_state("applied-model", "requested-model");
        let settings = crate::domain::conversation_runtime_envelope::ConversationRuntimeConfigurationObservation {
            model: ConversationRuntimeObservedValue::Observed("settings-model".to_string()),
            source: ConversationRuntimeObservedValue::Missing,
            ..Default::default()
        };

        let settings_snapshot =
            state.apply_stream_event(TurnStreamEvent::RuntimeEnvelopeObserved {
                observation: Box::new(ConversationRuntimeEnvelopeObservation::SettingsUpdated {
                    thread_id: "thread-envelope".to_string(),
                    settings: Box::new(settings),
                }),
            });
        assert!(matches!(
            settings_snapshot.update,
            TurnStreamUpdate::RuntimeEnvelopeObserved {
                rejection: None,
                ..
            }
        ));
        let status_snapshot = state.apply_stream_event(TurnStreamEvent::RuntimeEnvelopeObserved {
            observation: Box::new(
                ConversationRuntimeEnvelopeObservation::ThreadStatusChanged {
                    thread_id: "thread-envelope".to_string(),
                    status: ConversationRuntimeObservedValue::Observed(
                        ConversationRuntimeThreadStatus::Active {
                            waiting_on_approval: true,
                            waiting_on_user_input: false,
                            unknown_flags: Vec::new(),
                            unknown_flags_truncated: false,
                        },
                    ),
                },
            ),
        });
        let envelope = status_snapshot
            .runtime_envelope
            .expect("accepted observations should retain envelope");
        assert_eq!(
            envelope.applied.model,
            ConversationRuntimeObservedValue::Observed("settings-model".to_string())
        );
        assert!(matches!(
            envelope.thread_status,
            ConversationRuntimeObservedValue::Observed(ConversationRuntimeThreadStatus::Active {
                waiting_on_approval: true,
                ..
            })
        ));
        assert_eq!(envelope.observation_sequence, 2);
    }

    #[test]
    fn reroute_accepts_correlated_authority_and_rejects_only_stale_turn_identity() {
        let mut state = prepared_runtime_envelope_state("thread-default", "requested-model");
        let reroute = ConversationRuntimeEnvelopeObservation::ModelRerouted {
            thread_id: "thread-envelope".to_string(),
            turn_id: "turn-envelope".to_string(),
            reroute: ConversationRuntimeModelReroute {
                from_model: "requested-model".to_string(),
                to_model: "rerouted-model".to_string(),
                reason: ConversationRuntimeModelRerouteReason::HighRiskCyberActivity,
            },
        };

        let accepted = state.apply_stream_event(TurnStreamEvent::RuntimeEnvelopeObserved {
            observation: Box::new(reroute.clone()),
        });
        assert!(matches!(
            accepted.update,
            TurnStreamUpdate::RuntimeEnvelopeObserved {
                rejection: None,
                ..
            }
        ));
        assert_eq!(
            accepted
                .runtime_envelope
                .as_ref()
                .map(|envelope| &envelope.applied.model),
            Some(&ConversationRuntimeObservedValue::Observed(
                "rerouted-model".to_string()
            ))
        );

        let duplicate = state.apply_stream_event(TurnStreamEvent::RuntimeEnvelopeObserved {
            observation: Box::new(reroute),
        });
        assert!(matches!(
            duplicate.update,
            TurnStreamUpdate::RuntimeEnvelopeObserved {
                rejection: None,
                ..
            }
        ));
        assert_eq!(
            duplicate
                .runtime_envelope
                .as_ref()
                .map(|envelope| envelope.observation_sequence),
            Some(2)
        );

        let stale_turn = state.apply_stream_event(TurnStreamEvent::RuntimeEnvelopeObserved {
            observation: Box::new(ConversationRuntimeEnvelopeObservation::ModelRerouted {
                thread_id: "thread-envelope".to_string(),
                turn_id: "turn-stale".to_string(),
                reroute: ConversationRuntimeModelReroute {
                    from_model: "rerouted-model".to_string(),
                    to_model: "stale-model".to_string(),
                    reason: ConversationRuntimeModelRerouteReason::Unknown("future".to_string()),
                },
            }),
        });
        assert!(matches!(
            stale_turn.update,
            TurnStreamUpdate::RuntimeEnvelopeObserved {
                rejection: Some(TurnStreamRuntimeEnvelopeRejection::TurnMismatch { .. }),
                ..
            }
        ));
    }

    #[test]
    fn reroute_from_turn_model_survives_a_later_thread_settings_model() {
        let mut state = prepared_runtime_envelope_state("turn-model", "turn-model");
        state.apply_stream_event(TurnStreamEvent::RuntimeEnvelopeObserved {
            observation: Box::new(ConversationRuntimeEnvelopeObservation::SettingsUpdated {
                thread_id: "thread-envelope".to_string(),
                settings: Box::new(ConversationRuntimeConfigurationObservation {
                    model: ConversationRuntimeObservedValue::Observed(
                        "later-settings-model".to_string(),
                    ),
                    ..ConversationRuntimeConfigurationObservation::default()
                }),
            }),
        });

        let snapshot = state.apply_stream_event(TurnStreamEvent::RuntimeEnvelopeObserved {
            observation: Box::new(ConversationRuntimeEnvelopeObservation::ModelRerouted {
                thread_id: "thread-envelope".to_string(),
                turn_id: "turn-envelope".to_string(),
                reroute: ConversationRuntimeModelReroute {
                    from_model: "turn-model".to_string(),
                    to_model: "rerouted-model".to_string(),
                    reason: ConversationRuntimeModelRerouteReason::HighRiskCyberActivity,
                },
            }),
        });

        assert!(matches!(
            snapshot.update,
            TurnStreamUpdate::RuntimeEnvelopeObserved {
                rejection: None,
                ..
            }
        ));
        let envelope = snapshot
            .runtime_envelope
            .expect("correlated reroute should remain authoritative");
        assert_eq!(
            envelope.applied.model,
            ConversationRuntimeObservedValue::Observed("rerouted-model".to_string())
        );
        assert_eq!(
            envelope
                .last_model_reroute
                .as_ref()
                .map(|reroute| reroute.from_model.as_str()),
            Some("turn-model")
        );
    }

    #[test]
    fn new_turn_resets_reroute_provenance_and_rejects_prior_turn_observations() {
        let mut state = prepared_runtime_envelope_state("thread-default", "turn-one-model");
        state.apply_stream_event(TurnStreamEvent::RuntimeEnvelopeObserved {
            observation: Box::new(ConversationRuntimeEnvelopeObservation::ModelRerouted {
                thread_id: "thread-envelope".to_string(),
                turn_id: "turn-envelope".to_string(),
                reroute: ConversationRuntimeModelReroute {
                    from_model: "turn-one-model".to_string(),
                    to_model: "turn-one-rerouted".to_string(),
                    reason: ConversationRuntimeModelRerouteReason::HighRiskCyberActivity,
                },
            }),
        });

        state.begin_submission();
        assert!(
            state
                .runtime_envelope
                .as_ref()
                .and_then(|envelope| envelope.last_model_reroute.as_ref())
                .is_some()
        );
        let started = state.apply_stream_event(TurnStreamEvent::TurnStarted {
            turn_id: "turn-two".to_string(),
            runtime_request: Box::new(ConversationRuntimeConfigurationRequest {
                model: ConversationRuntimeRequestedValue::Value("turn-two-model".to_string()),
                ..Default::default()
            }),
        });
        assert!(matches!(
            started.update,
            TurnStreamUpdate::TurnStarted { ref turn_id, .. } if turn_id == "turn-two"
        ));
        assert_eq!(
            started
                .runtime_envelope
                .as_ref()
                .and_then(|envelope| envelope.last_model_reroute.as_ref()),
            None
        );

        let stale = state.apply_stream_event(TurnStreamEvent::RuntimeEnvelopeObserved {
            observation: Box::new(ConversationRuntimeEnvelopeObservation::ModelRerouted {
                thread_id: "thread-envelope".to_string(),
                turn_id: "turn-envelope".to_string(),
                reroute: ConversationRuntimeModelReroute {
                    from_model: "turn-one-rerouted".to_string(),
                    to_model: "stale-model".to_string(),
                    reason: ConversationRuntimeModelRerouteReason::Unknown("stale".to_string()),
                },
            }),
        });
        assert!(matches!(
            stale.update,
            TurnStreamUpdate::RuntimeEnvelopeObserved {
                rejection: Some(TurnStreamRuntimeEnvelopeRejection::TurnMismatch { .. }),
                ..
            }
        ));

        let current = state.apply_stream_event(TurnStreamEvent::RuntimeEnvelopeObserved {
            observation: Box::new(ConversationRuntimeEnvelopeObservation::ModelRerouted {
                thread_id: "thread-envelope".to_string(),
                turn_id: "turn-two".to_string(),
                reroute: ConversationRuntimeModelReroute {
                    from_model: "turn-two-model".to_string(),
                    to_model: "turn-two-rerouted".to_string(),
                    reason: ConversationRuntimeModelRerouteReason::HighRiskCyberActivity,
                },
            }),
        });
        assert!(matches!(
            current.update,
            TurnStreamUpdate::RuntimeEnvelopeObserved {
                rejection: None,
                ..
            }
        ));
        assert_eq!(
            current
                .runtime_envelope
                .as_ref()
                .map(|envelope| &envelope.applied.model),
            Some(&ConversationRuntimeObservedValue::Observed(
                "turn-two-rerouted".to_string()
            ))
        );
    }

    #[test]
    fn thread_scoped_observation_rejects_wrong_thread_without_mutation() {
        let mut state = prepared_runtime_envelope_state("applied-model", "requested-model");

        let snapshot = state.apply_stream_event(TurnStreamEvent::RuntimeEnvelopeObserved {
            observation: Box::new(
                ConversationRuntimeEnvelopeObservation::ThreadStatusChanged {
                    thread_id: "thread-other".to_string(),
                    status: ConversationRuntimeObservedValue::Observed(
                        ConversationRuntimeThreadStatus::SystemError,
                    ),
                },
            ),
        });

        assert!(matches!(
            snapshot.update,
            TurnStreamUpdate::RuntimeEnvelopeObserved {
                rejection: Some(TurnStreamRuntimeEnvelopeRejection::ThreadMismatch { .. }),
                ..
            }
        ));
        assert_eq!(
            snapshot
                .runtime_envelope
                .as_ref()
                .map(|envelope| envelope.observation_sequence),
            Some(0)
        );
    }

    #[test]
    fn correlated_projection_gap_marks_applied_truth_unavailable() {
        let mut state = prepared_runtime_envelope_state("applied-model", "requested-model");

        let snapshot = state.apply_stream_event(TurnStreamEvent::RuntimeEnvelopeObserved {
            observation: Box::new(ConversationRuntimeEnvelopeObservation::ProjectionGap {
                thread_id: "thread-envelope".to_string(),
                gap: ConversationRuntimeObservationGap::settings(),
            }),
        });

        assert!(matches!(
            snapshot.update,
            TurnStreamUpdate::RuntimeEnvelopeObserved {
                rejection: None,
                ..
            }
        ));
        let envelope = snapshot
            .runtime_envelope
            .expect("gap should remain in the core snapshot");
        assert_eq!(
            envelope.applied.model,
            ConversationRuntimeObservedValue::UnavailableAfterObservationGap
        );
        assert_eq!(
            envelope.projection_gap,
            Some(ConversationRuntimeObservationGap::settings())
        );
    }

    #[test]
    fn progressive_activity_projects_batch_update_and_reuses_snapshot_arc() {
        let mut state = prepared_turn_state();
        let batch = progressive_agent_batch(1, "thread-1", "turn-1", "item-1", "hello");

        let observed = state.apply_stream_event(TurnStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(batch.clone()),
        });

        assert!(matches!(
            &observed.update,
            TurnStreamUpdate::ProgressiveActivityObserved {
                activity,
                rejection: None,
            } if activity.first_sequence == batch.first_sequence()
                && activity.last_sequence == batch.last_sequence()
                && activity.source_observation_count == batch.source_observation_count()
        ));
        assert_eq!(observed.progressive_activity.last_sequence, Some(1));
        assert_eq!(observed.progressive_activity.source_observation_count, 1);
        assert_eq!(observed.progressive_activity.records.len(), 1);

        let status = state.apply_stream_event(TurnStreamEvent::StatusUpdated {
            text: "working".to_string(),
        });
        assert!(Arc::ptr_eq(
            &observed.progressive_activity,
            &status.progressive_activity
        ));

        let appended = state.apply_stream_event(TurnStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(progressive_agent_batch(
                2, "thread-1", "turn-1", "item-1", " world",
            )),
        });
        assert!(!Arc::ptr_eq(
            &observed.progressive_activity,
            &appended.progressive_activity
        ));
        assert_eq!(observed.progressive_activity.source_observation_count, 1);
        assert_eq!(appended.progressive_activity.source_observation_count, 2);
        assert_eq!(appended.progressive_activity.records.len(), 1);
        assert_eq!(
            appended.progressive_activity.records[0].observation_count(),
            2
        );
    }

    #[test]
    fn progressive_activity_resets_for_each_new_turn_and_thread() {
        let mut state = prepared_turn_state();
        state.apply_stream_event(TurnStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(progressive_agent_batch(
                1,
                "thread-1",
                "turn-1",
                "item-1",
                "first turn",
            )),
        });

        state.begin_submission();
        assert_eq!(state.progressive_activity.snapshot().records.len(), 1);
        let next_turn = state.apply_stream_event(TurnStreamEvent::TurnStarted {
            turn_id: "turn-2".to_string(),
            runtime_request: Box::default(),
        });
        assert!(matches!(
            next_turn.update,
            TurnStreamUpdate::TurnStarted { ref turn_id, .. } if turn_id == "turn-2"
        ));
        assert!(next_turn.progressive_activity.records.is_empty());
        assert_eq!(next_turn.progressive_activity.last_sequence, None);
        assert_eq!(next_turn.progressive_activity.source_observation_count, 0);

        state.apply_stream_event(TurnStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(progressive_agent_batch(
                1,
                "thread-1",
                "turn-2",
                "item-2",
                "second turn",
            )),
        });
        let next_thread = state.apply_stream_event(TurnStreamEvent::ThreadPrepared {
            thread_id: "thread-2".to_string(),
            title: "Next thread".to_string(),
            cwd: "/tmp/next-workspace".to_string(),
            runtime_envelope: Box::default(),
        });
        assert!(next_thread.progressive_activity.records.is_empty());
        assert_eq!(next_thread.progressive_activity.last_sequence, None);
        assert_eq!(next_thread.progressive_activity.source_observation_count, 0);
    }

    #[test]
    fn progressive_activity_rejects_stale_and_uncorrelated_batches() {
        let mut state = prepared_turn_state();
        state.apply_stream_event(TurnStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(progressive_agent_batch(
                7, "thread-1", "turn-1", "item-1", "accepted",
            )),
        });

        let stale = state.apply_stream_event(TurnStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(progressive_agent_batch(
                7, "thread-1", "turn-1", "item-1", "replayed",
            )),
        });
        assert!(matches!(
            stale.update,
            TurnStreamUpdate::ProgressiveActivityObserved {
                rejection: Some(ConversationProgressiveActivityRejection::StaleSequence {
                    last_sequence: 7
                }),
                ..
            }
        ));

        let wrong_thread = state.apply_stream_event(TurnStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(progressive_agent_batch(
                8,
                "thread-other",
                "turn-1",
                "item-2",
                "wrong thread",
            )),
        });
        assert!(matches!(
            wrong_thread.update,
            TurnStreamUpdate::ProgressiveActivityObserved {
                rejection: Some(ConversationProgressiveActivityRejection::ThreadMismatch),
                ..
            }
        ));

        let wrong_turn = state.apply_stream_event(TurnStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(progressive_agent_batch(
                9,
                "thread-1",
                "turn-other",
                "item-3",
                "wrong turn",
            )),
        });
        assert!(matches!(
            wrong_turn.update,
            TurnStreamUpdate::ProgressiveActivityObserved {
                rejection: Some(ConversationProgressiveActivityRejection::TurnMismatch),
                ..
            }
        ));
        assert_eq!(wrong_turn.progressive_activity.records.len(), 1);
        assert_eq!(wrong_turn.progressive_activity.last_sequence, Some(7));
        assert_eq!(wrong_turn.progressive_activity.invalid_observation_count, 3);
    }

    #[test]
    fn progressive_activity_preserves_incomplete_history_counters() {
        let mut state = prepared_turn_state();
        let mut batch = progressive_agent_batch(1, "thread-1", "turn-1", "item-1", "bounded");
        batch.record_superseded_publication().unwrap();
        batch.set_incomplete_counters_for_test(2, 3, 4, 5);

        let snapshot = state.apply_stream_event(TurnStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(batch),
        });

        assert!(snapshot.progressive_activity.history_incomplete());
        assert_eq!(
            snapshot.progressive_activity.superseded_publication_count,
            1
        );
        assert_eq!(snapshot.progressive_activity.coalesced_observation_count, 2);
        assert_eq!(snapshot.progressive_activity.loss_event_count(), 3);
        assert_eq!(snapshot.progressive_activity.invalid_observation_count, 4);
        assert_eq!(snapshot.progressive_activity.unknown_observation_count, 5);
    }

    #[test]
    fn progressive_history_only_batch_advances_core_sequence_and_retains_gap() {
        let mut state = prepared_turn_state();
        let mut batch = progressive_agent_batch(4, "thread-1", "turn-1", "item-1", "discarded");
        batch.discard_retained_records();

        let snapshot = state.apply_stream_event(TurnStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(batch),
        });

        assert!(matches!(
            snapshot.update,
            TurnStreamUpdate::ProgressiveActivityObserved {
                rejection: None,
                ..
            }
        ));
        assert_eq!(snapshot.progressive_activity.last_sequence, Some(4));
        assert_eq!(snapshot.progressive_activity.source_observation_count, 1);
        assert_eq!(snapshot.progressive_activity.dropped_observation_count, 1);
        assert!(snapshot.progressive_activity.records.is_empty());
        assert!(snapshot.progressive_activity.history_incomplete());
    }

    #[test]
    fn completed_message_projects_snapshot_update() {
        let mut state = TurnStreamState::new();

        let snapshot = state.apply_stream_event(TurnStreamEvent::AgentMessageCompleted {
            item_id: "item-1".to_string(),
            phase: None,
            text: "final answer".to_string(),
        });

        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::AgentMessageCompleted {
                item_id: "item-1".to_string(),
                phase: None,
                text: "final answer".to_string(),
            }
        );
    }

    #[test]
    fn item_lifecycle_event_requires_core_thread_and_turn_correlation() {
        let mut state = prepared_turn_state();

        let accepted = state.apply_stream_event(TurnStreamEvent::ItemLifecycleObserved {
            observation: Box::new(item_lifecycle_observation(
                "thread-1",
                "turn-1",
                "item-1",
                crate::domain::conversation_item_lifecycle::ConversationItemLifecyclePhase::Started,
            )),
        });
        assert!(matches!(
            accepted.update,
            TurnStreamUpdate::ItemLifecycleObserved {
                consistency: Some(ConversationItemLifecycleConsistency::Accepted),
                rejection: None,
                ..
            }
        ));
        assert_eq!(accepted.item_lifecycle.records.len(), 1);

        let rejected = state.apply_stream_event(TurnStreamEvent::ItemLifecycleObserved {
            observation: Box::new(item_lifecycle_observation(
                "thread-other",
                "turn-1",
                "item-2",
                crate::domain::conversation_item_lifecycle::ConversationItemLifecyclePhase::Completed,
            )),
        });
        assert!(matches!(
            rejected.update,
            TurnStreamUpdate::ItemLifecycleObserved {
                consistency: None,
                rejection: Some(ConversationItemLifecycleRejection::ThreadMismatch { .. }),
                ..
            }
        ));
        assert_eq!(rejected.item_lifecycle.records.len(), 1);
        assert_eq!(rejected.item_lifecycle.invalid_record_count, 1);
    }

    #[test]
    fn non_lifecycle_events_reuse_the_lifecycle_snapshot_arc() {
        let mut state = prepared_turn_state();
        let observed = state.apply_stream_event(TurnStreamEvent::ItemLifecycleObserved {
            observation: Box::new(item_lifecycle_observation(
                "thread-1",
                "turn-1",
                "item-1",
                crate::domain::conversation_item_lifecycle::ConversationItemLifecyclePhase::Started,
            )),
        });

        let status = state.apply_stream_event(TurnStreamEvent::StatusUpdated {
            text: "working".to_string(),
        });

        assert!(Arc::ptr_eq(
            &observed.item_lifecycle,
            &status.item_lifecycle
        ));
    }

    #[test]
    fn loaded_lifecycle_survives_same_thread_prepare_and_appends_live_records() {
        let mut loaded_projection = ConversationItemLifecycleProjection::default();
        let mut loaded_observation = item_lifecycle_observation(
            "thread-1",
            "turn-loaded",
            "item-loaded",
            crate::domain::conversation_item_lifecycle::ConversationItemLifecyclePhase::SnapshotObserved,
        );
        loaded_observation.source =
            crate::domain::conversation_item_lifecycle::ConversationItemLifecycleSource::Snapshot;
        loaded_observation.observed_at_ms = None;
        loaded_observation.outcome =
            crate::domain::conversation_item_lifecycle::ConversationItemOutcome::Completed;
        loaded_projection.apply(loaded_observation).unwrap();

        let mut state = TurnStreamState::new();
        state
            .seed_loaded_thread(
                "thread-1",
                "Loaded thread",
                "/tmp/workspace",
                loaded_projection.snapshot(),
            )
            .unwrap();
        let prepared = state.apply_stream_event(TurnStreamEvent::ThreadPrepared {
            thread_id: "thread-1".to_string(),
            title: "Loaded thread".to_string(),
            cwd: "/tmp/workspace".to_string(),
            runtime_envelope: Box::default(),
        });
        assert_eq!(prepared.item_lifecycle.records.len(), 1);

        state.apply_stream_event(TurnStreamEvent::TurnStarted {
            turn_id: "turn-live".to_string(),
            runtime_request: Box::default(),
        });
        let live = state.apply_stream_event(TurnStreamEvent::ItemLifecycleObserved {
            observation: Box::new(item_lifecycle_observation(
                "thread-1",
                "turn-live",
                "item-live",
                crate::domain::conversation_item_lifecycle::ConversationItemLifecyclePhase::Started,
            )),
        });

        assert_eq!(live.item_lifecycle.records.len(), 2);
        assert_eq!(
            live.item_lifecycle.records[0].observation.source,
            crate::domain::conversation_item_lifecycle::ConversationItemLifecycleSource::Snapshot
        );
        assert_eq!(
            live.item_lifecycle.records[1].observation.source,
            crate::domain::conversation_item_lifecycle::ConversationItemLifecycleSource::Live
        );
    }

    #[test]
    fn different_thread_prepare_clears_hydrated_lifecycle() {
        let mut loaded_projection = ConversationItemLifecycleProjection::default();
        let mut loaded_observation = item_lifecycle_observation(
            "thread-loaded",
            "turn-loaded",
            "item-loaded",
            crate::domain::conversation_item_lifecycle::ConversationItemLifecyclePhase::SnapshotObserved,
        );
        loaded_observation.source =
            crate::domain::conversation_item_lifecycle::ConversationItemLifecycleSource::Snapshot;
        loaded_observation.observed_at_ms = None;
        loaded_projection.apply(loaded_observation).unwrap();
        let mut state = TurnStreamState::new();
        state
            .seed_loaded_thread(
                "thread-loaded",
                "Loaded thread",
                "/tmp/workspace",
                loaded_projection.snapshot(),
            )
            .unwrap();

        let prepared = state.apply_stream_event(TurnStreamEvent::ThreadPrepared {
            thread_id: "thread-new".to_string(),
            title: "New thread".to_string(),
            cwd: "/tmp/workspace".to_string(),
            runtime_envelope: Box::default(),
        });

        assert!(prepared.item_lifecycle.records.is_empty());
    }

    #[test]
    fn default_state_projects_attachment_status_and_runtime_notice_updates() {
        let mut state = TurnStreamState::default();
        let profile = TerminalBridgeAttachmentProfile::default();

        let attachment = state.apply_stream_event(TurnStreamEvent::AttachmentObserved { profile });
        assert_eq!(attachment.revision, 1);
        assert_eq!(
            attachment.update,
            TurnStreamUpdate::AttachmentObserved { profile }
        );

        let status = state.apply_stream_event(TurnStreamEvent::StatusUpdated {
            text: "running tools".to_string(),
        });
        assert_eq!(status.status_text.as_deref(), Some("running tools"));
        assert_eq!(
            status.update,
            TurnStreamUpdate::StatusUpdated {
                text: "running tools".to_string(),
            }
        );

        let notice = state.apply_runtime_notice("runtime reattached".to_string());
        assert_eq!(notice.status_text.as_deref(), Some("running tools"));
        assert_eq!(
            notice.update,
            TurnStreamUpdate::RuntimeNotice {
                notice: "runtime reattached".to_string(),
            }
        );
    }

    #[test]
    fn tool_activity_projects_domain_activity() {
        let mut state = TurnStreamState::new();
        let activity = ConversationToolActivity {
            kind: ConversationToolActivityKind::FileChange,
            text: "edited src/lib.rs".to_string(),
            display_label: None,
            file_change_count: 1,
        };

        let snapshot = state.apply_stream_event(TurnStreamEvent::ToolActivity {
            activity: activity.clone(),
        });

        assert_eq!(snapshot.update, TurnStreamUpdate::ToolActivity { activity });
    }

    #[test]
    fn approval_review_projects_domain_review() {
        let mut state = TurnStreamState::new();
        let review = ConversationApprovalReview {
            target_item_id: "review-1".to_string(),
            status: ConversationApprovalReviewStatus::InProgress,
            risk_level: Some("medium".to_string()),
            rationale: Some("approve command".to_string()),
        };

        let snapshot = state.apply_stream_event(TurnStreamEvent::ApprovalReviewUpdated {
            review: review.clone(),
        });

        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::ApprovalReviewUpdated { review }
        );
    }

    #[test]
    fn pending_approval_identity_ignores_same_id_old_server_resolution_and_clears_exact_request() {
        let mut state = TurnStreamState::new();
        let request = ConversationApprovalRequest {
            approval_id: "approval-core".to_string(),
            server_request_id: "server-current".to_string(),
            method: "item/fileChange/requestApproval".to_string(),
            kind: ConversationApprovalRequestKind::FileChange,
            summary: "File changes requested.".to_string(),
            details: vec!["Reason: update tests".to_string()],
        };
        let request_identity = request.identity();

        let requested = state.apply_stream_event(TurnStreamEvent::ApprovalRequested {
            request: request.clone(),
        });
        assert_eq!(
            requested.update,
            TurnStreamUpdate::ApprovalRequested { request }
        );
        assert_eq!(requested.status_text.as_deref(), Some("approval required"));
        assert!(state.matches_pending_approval(&request_identity));

        let stale_identity = ConversationApprovalRequestIdentity {
            approval_id: "approval-core".to_string(),
            server_request_id: "server-old".to_string(),
        };
        let stale = state.apply_stream_event(TurnStreamEvent::ApprovalResolved {
            request_identity: stale_identity.clone(),
            resolution: ConversationApprovalResolution::Declined,
        });
        assert!(matches!(
            stale.update,
            TurnStreamUpdate::ApprovalResolutionIgnored {
                request_identity: ignored,
                resolution: ConversationApprovalResolution::Declined,
            } if ignored == stale_identity
        ));
        assert!(state.matches_pending_approval(&request_identity));

        let resolved = state.apply_stream_event(TurnStreamEvent::ApprovalResolved {
            request_identity: request_identity.clone(),
            resolution: ConversationApprovalResolution::Declined,
        });
        assert!(matches!(
            resolved.update,
            TurnStreamUpdate::ApprovalResolved {
                request_identity: resolved_identity,
                resolution: ConversationApprovalResolution::Declined,
            } if resolved_identity == request_identity
        ));
        assert!(!state.matches_pending_approval(&request_identity));

        let duplicate = state.apply_stream_event(TurnStreamEvent::ApprovalResolved {
            request_identity: request_identity.clone(),
            resolution: ConversationApprovalResolution::Declined,
        });
        assert!(matches!(
            duplicate.update,
            TurnStreamUpdate::ApprovalResolutionIgnored {
                request_identity: ignored,
                resolution: ConversationApprovalResolution::Declined,
            } if ignored == request_identity
        ));
    }

    #[test]
    fn confirmed_completed_receipt_projects_completed_update() {
        let mut state = prepared_turn_state();
        let receipt = ConversationTurnTerminalReceipt::completed(
            "thread-1",
            "turn-1",
            vec!["docs/plan.md".to_string()],
        )
        .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);

        let snapshot = state.apply_stream_event(TurnStreamEvent::TurnTerminal {
            receipt: receipt.clone(),
            execution_snapshot_capture: None,
        });

        assert_eq!(snapshot.active_turn_id, None);
        assert_eq!(snapshot.status_text.as_deref(), Some("turn completed"));
        assert_eq!(
            snapshot.terminal,
            Some(TurnStreamTerminalSnapshot::Turn {
                receipt: Box::new(receipt)
            })
        );
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::TurnCompleted {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: vec!["docs/plan.md".to_string()],
                execution_snapshot_capture: None,
                status_text: "turn completed".to_string(),
            }
        );
        assert!(
            state.has_unapplied_confirmed_terminal(),
            "manual submission must remain closed until post-turn applies this terminal"
        );
    }

    #[test]
    fn turn_completed_records_terminal_snapshot_with_execution_capture() {
        let mut state = TurnStreamState::new();
        let execution_snapshot_capture =
            TurnSnapshotCapture::ready("/tmp/workspace", ExecutionSnapshot::default());

        let snapshot = state.apply_turn_completed(
            "turn-1".to_string(),
            vec!["docs/plan.md".to_string()],
            execution_snapshot_capture.clone(),
        );

        assert_eq!(snapshot.active_turn_id, None);
        assert_eq!(snapshot.status_text.as_deref(), Some("turn completed"));
        assert_eq!(
            snapshot.terminal,
            Some(TurnStreamTerminalSnapshot::Turn {
                receipt: Box::new(
                    ConversationTurnTerminalReceipt::completed(
                        "",
                        "turn-1",
                        vec!["docs/plan.md".to_string()]
                    )
                    .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed)
                )
            })
        );
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::TurnCompleted {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: vec!["docs/plan.md".to_string()],
                execution_snapshot_capture: Some(execution_snapshot_capture),
                status_text: "turn completed".to_string(),
            }
        );
    }

    #[test]
    fn non_completed_and_unconfirmed_receipts_stay_non_completed_updates() {
        let failed_error = ConversationTurnError::new("model failed", None::<&str>, None);
        let receipts = vec![
            confirmed_receipt(ConversationTurnTerminalOutcome::Interrupted),
            confirmed_receipt(ConversationTurnTerminalOutcome::Failed {
                error: failed_error.clone(),
            }),
            confirmed_receipt(ConversationTurnTerminalOutcome::Unknown {
                reason: ConversationTurnTerminalUncertainty::NonRetryErrorGraceExpired,
                observed_error: Some(failed_error),
            }),
            ConversationTurnTerminalReceipt::completed("thread-1", "turn-1", Vec::new())
                .with_application_delivery(ConversationTurnApplicationDelivery::Unconfirmed(
                    ConversationTurnApplicationDeliveryFailure::Disconnected,
                )),
        ];

        for receipt in receipts {
            let mut state = prepared_turn_state();
            let snapshot = state.apply_stream_event(TurnStreamEvent::TurnTerminal {
                receipt: receipt.clone(),
                execution_snapshot_capture: None,
            });

            assert!(matches!(
                snapshot.update,
                TurnStreamUpdate::TurnTerminal {
                    receipt: projected,
                    ..
                } if projected.as_ref() == &receipt
            ));
            assert_eq!(
                snapshot.terminal,
                Some(TurnStreamTerminalSnapshot::Turn {
                    receipt: Box::new(receipt)
                })
            );
        }
    }

    #[test]
    fn terminal_receipt_must_match_active_thread_and_turn() {
        let mut state = prepared_turn_state();
        let receipt =
            ConversationTurnTerminalReceipt::completed("thread-other", "turn-1", Vec::new())
                .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);

        let snapshot = state.apply_stream_event(TurnStreamEvent::TurnTerminal {
            receipt: receipt.clone(),
            execution_snapshot_capture: None,
        });

        assert_eq!(snapshot.active_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(snapshot.terminal, None);
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::TurnTerminalIgnored {
                receipt: Box::new(receipt),
                reason: TurnStreamTerminalRejection::ThreadMismatch {
                    expected_thread_id: Some("thread-1".to_string())
                }
            }
        );
    }

    #[test]
    fn terminal_receipt_for_another_turn_keeps_the_active_turn_running() {
        let mut state = prepared_turn_state();
        let receipt =
            ConversationTurnTerminalReceipt::completed("thread-1", "turn-other", Vec::new())
                .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);

        let snapshot = state.apply_stream_event(TurnStreamEvent::TurnTerminal {
            receipt: receipt.clone(),
            execution_snapshot_capture: None,
        });

        assert_eq!(snapshot.active_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(snapshot.terminal, None);
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::TurnTerminalIgnored {
                receipt: Box::new(receipt),
                reason: TurnStreamTerminalRejection::TurnMismatch {
                    expected_turn_id: Some("turn-1".to_string())
                }
            }
        );
    }

    #[test]
    fn first_terminal_receipt_cannot_be_promoted_by_later_observations() {
        let mut state = prepared_turn_state();
        let interrupted = confirmed_receipt(ConversationTurnTerminalOutcome::Interrupted);
        state.apply_stream_event(TurnStreamEvent::TurnTerminal {
            receipt: interrupted.clone(),
            execution_snapshot_capture: None,
        });
        let later_completed = ConversationTurnTerminalReceipt::completed(
            "thread-1",
            "turn-1",
            vec!["docs/plan.md".to_string()],
        )
        .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);

        let duplicate = state.apply_stream_event(TurnStreamEvent::TurnTerminal {
            receipt: later_completed.clone(),
            execution_snapshot_capture: None,
        });

        assert_eq!(
            duplicate.terminal,
            Some(TurnStreamTerminalSnapshot::Turn {
                receipt: Box::new(interrupted)
            })
        );
        assert_eq!(
            duplicate.update,
            TurnStreamUpdate::TurnTerminalIgnored {
                receipt: Box::new(later_completed),
                reason: TurnStreamTerminalRejection::TerminalAlreadyApplied
            }
        );
    }

    #[test]
    fn retry_update_records_correlation_without_ending_the_turn() {
        let mut state = prepared_turn_state();
        let error = ConversationTurnError::new("server overloaded", None::<&str>, None);

        let snapshot = state.apply_stream_event(TurnStreamEvent::TurnRetrying {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            error: error.clone(),
        });

        assert_eq!(snapshot.active_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(snapshot.terminal, None);
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::TurnRetrying {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                error,
                correlation_failure: None,
                status_text: "turn retrying".to_string()
            }
        );
    }

    #[test]
    fn failed_records_terminal_snapshot() {
        let mut state = TurnStreamState::new();
        state.apply_stream_event(TurnStreamEvent::TurnStarted {
            turn_id: "turn-1".to_string(),
            runtime_request: Box::default(),
        });

        let snapshot = state.apply_stream_event(TurnStreamEvent::Failed {
            message: "transport closed".to_string(),
        });

        assert_eq!(snapshot.active_turn_id, None);
        assert_eq!(snapshot.status_text.as_deref(), Some("turn failed"));
        assert_eq!(
            snapshot.terminal,
            Some(TurnStreamTerminalSnapshot::Failed {
                message: "transport closed".to_string()
            })
        );
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::Failed {
                message: "transport closed".to_string(),
                status_text: "turn failed".to_string(),
            }
        );
    }
}
