use std::collections::VecDeque;
use std::fmt;
use std::sync::mpsc::{RecvError, RecvTimeoutError, SendError, TryRecvError, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::domain::conversation_progressive_activity::MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES;

pub use crate::domain::conversation_stream::ConversationStreamEvent;

// Keep only a short control burst between the app-server producer and the
// application reducer. Progressive activity is coalesced only within adjacent
// queue segments, so a control fact remains an exact ordering boundary while
// output pressure can neither consume control admission nor block producers.
pub const CONVERSATION_STREAM_CHANNEL_CAPACITY: usize = 8;

pub struct ConversationStreamSender {
    shared: Arc<ConversationStreamMailbox>,
}

pub struct ConversationStreamReceiver {
    shared: Arc<ConversationStreamMailbox>,
}

struct ConversationStreamMailbox {
    state: Mutex<ConversationStreamMailboxState>,
    changed: Condvar,
}

struct ConversationStreamMailboxState {
    pending_events: VecDeque<ConversationStreamEvent>,
    control_event_count: usize,
    progressive_dynamic_bytes: usize,
    sender_count: usize,
    receiver_alive: bool,
}

impl ConversationStreamMailboxState {
    fn new() -> Self {
        Self {
            pending_events: VecDeque::with_capacity(CONVERSATION_STREAM_CHANNEL_CAPACITY * 2 + 1),
            control_event_count: 0,
            progressive_dynamic_bytes: 0,
            sender_count: 1,
            receiver_alive: true,
        }
    }

    fn publish_control(&mut self, event: ConversationStreamEvent) {
        self.pending_events.push_back(event);
        self.control_event_count = self.control_event_count.saturating_add(1);
    }

    fn publish_progressive(
        &mut self,
        event: ConversationStreamEvent,
    ) -> Option<ConversationStreamEvent> {
        if progressive_event_is_empty(&event) {
            return None;
        }
        if self.pending_events.back().is_some_and(is_progressive_event) {
            let pending = self
                .pending_events
                .back()
                .expect("progressive queue tail should remain available");
            if !progressive_events_can_merge(pending, &event) {
                return Some(event);
            }
            let pending = self
                .pending_events
                .back_mut()
                .expect("progressive queue tail should remain available");
            self.progressive_dynamic_bytes = self
                .progressive_dynamic_bytes
                .saturating_sub(progressive_event_dynamic_bytes(pending));
            merge_progressive_events(pending, event);
            self.progressive_dynamic_bytes = self
                .progressive_dynamic_bytes
                .saturating_add(progressive_event_dynamic_bytes(pending));
        } else {
            self.progressive_dynamic_bytes = self
                .progressive_dynamic_bytes
                .saturating_add(progressive_event_dynamic_bytes(&event));
            self.pending_events.push_back(event);
        }
        self.enforce_progressive_memory_bound();
        None
    }

    fn pop_next(&mut self) -> Option<ConversationStreamEvent> {
        let event = self.pending_events.pop_front()?;
        if is_progressive_event(&event) {
            self.progressive_dynamic_bytes = self
                .progressive_dynamic_bytes
                .saturating_sub(progressive_event_dynamic_bytes(&event));
        } else {
            self.control_event_count = self.control_event_count.saturating_sub(1);
        }
        Some(event)
    }

    fn enforce_progressive_memory_bound(&mut self) {
        if self.progressive_dynamic_bytes <= MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES {
            return;
        }

        for event in &mut self.pending_events {
            let before = progressive_event_dynamic_bytes(event);
            if before == 0 {
                continue;
            }
            discard_progressive_event_records(event);
            let after = progressive_event_dynamic_bytes(event);
            self.progressive_dynamic_bytes = self
                .progressive_dynamic_bytes
                .saturating_sub(before)
                .saturating_add(after);
            if self.progressive_dynamic_bytes <= MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES {
                break;
            }
        }
    }

    fn is_disconnected_for_receiver(&self) -> bool {
        self.sender_count == 0 && self.pending_events.is_empty()
    }
}

impl ConversationStreamSender {
    // Keep std::mpsc source compatibility: disconnect returns ownership of the
    // original event even though the closed event enum makes that error large.
    #[allow(clippy::result_large_err)]
    pub fn send(
        &self,
        event: ConversationStreamEvent,
    ) -> Result<(), SendError<ConversationStreamEvent>> {
        let mut state = self.shared.lock_state();
        if !state.receiver_alive {
            return Err(SendError(event));
        }
        if is_progressive_event(&event) {
            if let Some(event) = state.publish_progressive(event) {
                return Err(SendError(event));
            }
            drop(state);
            self.shared.changed.notify_one();
            return Ok(());
        }

        while state.control_event_count == CONVERSATION_STREAM_CHANNEL_CAPACITY {
            state = self.shared.wait_for_change(state);
            if !state.receiver_alive {
                return Err(SendError(event));
            }
        }
        state.publish_control(event);
        drop(state);
        self.shared.changed.notify_one();
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    pub fn try_send(
        &self,
        event: ConversationStreamEvent,
    ) -> Result<(), TrySendError<ConversationStreamEvent>> {
        let mut state = self.shared.lock_state();
        if !state.receiver_alive {
            return Err(TrySendError::Disconnected(event));
        }
        if is_progressive_event(&event) {
            if let Some(event) = state.publish_progressive(event) {
                return Err(TrySendError::Full(event));
            }
            drop(state);
            self.shared.changed.notify_one();
            return Ok(());
        }
        if state.control_event_count == CONVERSATION_STREAM_CHANNEL_CAPACITY {
            return Err(TrySendError::Full(event));
        }
        state.publish_control(event);
        drop(state);
        self.shared.changed.notify_one();
        Ok(())
    }
}

impl Clone for ConversationStreamSender {
    fn clone(&self) -> Self {
        let mut state = self.shared.lock_state();
        state.sender_count = state.sender_count.saturating_add(1);
        drop(state);
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl Drop for ConversationStreamSender {
    fn drop(&mut self) {
        let mut state = self.shared.lock_state();
        state.sender_count = state.sender_count.saturating_sub(1);
        let disconnected = state.sender_count == 0;
        drop(state);
        if disconnected {
            self.shared.changed.notify_all();
        }
    }
}

impl fmt::Debug for ConversationStreamSender {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConversationStreamSender")
            .finish_non_exhaustive()
    }
}

impl ConversationStreamReceiver {
    pub fn recv(&self) -> Result<ConversationStreamEvent, RecvError> {
        let mut state = self.shared.lock_state();
        loop {
            if let Some(event) = state.pop_next() {
                drop(state);
                self.shared.changed.notify_one();
                return Ok(event);
            }
            if state.is_disconnected_for_receiver() {
                return Err(RecvError);
            }
            state = self.shared.wait_for_change(state);
        }
    }

    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<ConversationStreamEvent, RecvTimeoutError> {
        let started_at = Instant::now();
        let mut state = self.shared.lock_state();
        loop {
            if let Some(event) = state.pop_next() {
                drop(state);
                self.shared.changed.notify_one();
                return Ok(event);
            }
            if state.is_disconnected_for_receiver() {
                return Err(RecvTimeoutError::Disconnected);
            }
            let remaining = timeout.saturating_sub(started_at.elapsed());
            if remaining.is_zero() {
                return Err(RecvTimeoutError::Timeout);
            }
            let (next_state, wait_result) = self.shared.wait_for_change_timeout(state, remaining);
            state = next_state;
            if wait_result.timed_out() && state.pending_events.is_empty() {
                if state.sender_count == 0 {
                    return Err(RecvTimeoutError::Disconnected);
                }
                return Err(RecvTimeoutError::Timeout);
            }
        }
    }

    pub fn try_recv(&self) -> Result<ConversationStreamEvent, TryRecvError> {
        let mut state = self.shared.lock_state();
        if let Some(event) = state.pop_next() {
            drop(state);
            self.shared.changed.notify_one();
            return Ok(event);
        }
        if state.is_disconnected_for_receiver() {
            Err(TryRecvError::Disconnected)
        } else {
            Err(TryRecvError::Empty)
        }
    }

    pub fn iter(&self) -> ConversationStreamIter<'_> {
        ConversationStreamIter { receiver: self }
    }

    pub fn try_iter(&self) -> ConversationStreamTryIter<'_> {
        ConversationStreamTryIter { receiver: self }
    }
}

impl Drop for ConversationStreamReceiver {
    fn drop(&mut self) {
        let mut state = self.shared.lock_state();
        state.receiver_alive = false;
        state.pending_events.clear();
        state.control_event_count = 0;
        state.progressive_dynamic_bytes = 0;
        drop(state);
        self.shared.changed.notify_all();
    }
}

impl fmt::Debug for ConversationStreamReceiver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConversationStreamReceiver")
            .finish_non_exhaustive()
    }
}

pub struct ConversationStreamIter<'a> {
    receiver: &'a ConversationStreamReceiver,
}

impl Iterator for ConversationStreamIter<'_> {
    type Item = ConversationStreamEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

pub struct ConversationStreamTryIter<'a> {
    receiver: &'a ConversationStreamReceiver,
}

impl Iterator for ConversationStreamTryIter<'_> {
    type Item = ConversationStreamEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.try_recv().ok()
    }
}

impl<'a> IntoIterator for &'a ConversationStreamReceiver {
    type Item = ConversationStreamEvent;
    type IntoIter = ConversationStreamIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct ConversationStreamIntoIter {
    receiver: ConversationStreamReceiver,
}

impl Iterator for ConversationStreamIntoIter {
    type Item = ConversationStreamEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

impl IntoIterator for ConversationStreamReceiver {
    type Item = ConversationStreamEvent;
    type IntoIter = ConversationStreamIntoIter;

    fn into_iter(self) -> Self::IntoIter {
        ConversationStreamIntoIter { receiver: self }
    }
}

impl ConversationStreamMailbox {
    fn lock_state(&self) -> MutexGuard<'_, ConversationStreamMailboxState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wait_for_change<'a>(
        &self,
        state: MutexGuard<'a, ConversationStreamMailboxState>,
    ) -> MutexGuard<'a, ConversationStreamMailboxState> {
        self.changed
            .wait(state)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wait_for_change_timeout<'a>(
        &self,
        state: MutexGuard<'a, ConversationStreamMailboxState>,
        timeout: Duration,
    ) -> (
        MutexGuard<'a, ConversationStreamMailboxState>,
        std::sync::WaitTimeoutResult,
    ) {
        self.changed
            .wait_timeout(state, timeout)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

// P0-C2 has exactly one coalescible stream variant. Keeping classification and
// merge here prevents application mailbox policy from spreading into adapters.
fn is_progressive_event(event: &ConversationStreamEvent) -> bool {
    matches!(
        event,
        ConversationStreamEvent::ProgressiveActivityObserved { .. }
    )
}

fn progressive_event_is_empty(event: &ConversationStreamEvent) -> bool {
    matches!(
        event,
        ConversationStreamEvent::ProgressiveActivityObserved { batch }
            if batch.source_observation_count() == 0
    )
}

fn merge_progressive_events(
    pending: &mut ConversationStreamEvent,
    incoming: ConversationStreamEvent,
) {
    let (
        ConversationStreamEvent::ProgressiveActivityObserved {
            batch: pending_batch,
        },
        ConversationStreamEvent::ProgressiveActivityObserved {
            batch: incoming_batch,
        },
    ) = (pending, incoming)
    else {
        unreachable!("progressive mailbox segment must contain only progressive activity")
    };
    if pending_batch.try_merge_from(*incoming_batch).is_ok()
        && pending_batch.record_superseded_publication().is_ok()
    {
    } else {
        debug_assert!(
            false,
            "preflight-approved progressive merge must remain valid"
        );
    }
}

fn progressive_events_can_merge(
    pending: &ConversationStreamEvent,
    incoming: &ConversationStreamEvent,
) -> bool {
    let (
        ConversationStreamEvent::ProgressiveActivityObserved {
            batch: pending_batch,
        },
        ConversationStreamEvent::ProgressiveActivityObserved {
            batch: incoming_batch,
        },
    ) = (pending, incoming)
    else {
        return false;
    };
    if pending_batch.source_observation_count() == 0
        || incoming_batch.source_observation_count() == 0
    {
        return false;
    }
    pending_batch
        .validate_publication_merge_candidate(incoming_batch)
        .is_ok()
}

fn progressive_event_dynamic_bytes(event: &ConversationStreamEvent) -> usize {
    match event {
        ConversationStreamEvent::ProgressiveActivityObserved { batch } => {
            batch.retained_dynamic_bytes()
        }
        _ => 0,
    }
}

fn discard_progressive_event_records(event: &mut ConversationStreamEvent) {
    let ConversationStreamEvent::ProgressiveActivityObserved { batch } = event else {
        return;
    };
    batch.discard_retained_records();
}

pub fn conversation_stream_channel() -> (ConversationStreamSender, ConversationStreamReceiver) {
    let shared = Arc::new(ConversationStreamMailbox {
        state: Mutex::new(ConversationStreamMailboxState::new()),
        changed: Condvar::new(),
    });
    (
        ConversationStreamSender {
            shared: Arc::clone(&shared),
        },
        ConversationStreamReceiver { shared },
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationRuntimeEnvelopeProjection {
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub runtime_envelope:
        Option<crate::domain::conversation_runtime_envelope::ConversationRuntimeEnvelope>,
    pub last_rejection: Option<ConversationRuntimeEnvelopeProjectionRejection>,
}

impl ConversationRuntimeEnvelopeProjection {
    pub fn apply_event(&mut self, event: &ConversationStreamEvent) {
        match event {
            ConversationStreamEvent::ThreadPrepared {
                thread_id,
                runtime_envelope,
                ..
            } => {
                self.thread_id = Some(thread_id.clone());
                self.turn_id = None;
                self.runtime_envelope = Some((**runtime_envelope).clone());
            }
            ConversationStreamEvent::TurnStarted {
                turn_id,
                runtime_request,
            } => {
                self.turn_id = Some(turn_id.clone());
                if let Some(envelope) = self.runtime_envelope.as_mut() {
                    envelope.record_turn_request((**runtime_request).clone());
                } else {
                    self.last_rejection =
                        Some(ConversationRuntimeEnvelopeProjectionRejection::EnvelopeNotPrepared);
                }
            }
            ConversationStreamEvent::RuntimeEnvelopeObserved { observation } => {
                let Some(envelope) = self.runtime_envelope.as_mut() else {
                    self.last_rejection =
                        Some(ConversationRuntimeEnvelopeProjectionRejection::EnvelopeNotPrepared);
                    return;
                };
                if let Err(rejection) = envelope.apply_correlated_observation(
                    self.thread_id.as_deref(),
                    self.turn_id.as_deref(),
                    observation,
                ) {
                    self.last_rejection = Some(
                        ConversationRuntimeEnvelopeProjectionRejection::Observation(rejection),
                    );
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationRuntimeEnvelopeProjectionRejection {
    EnvelopeNotPrepared,
    Observation(
        crate::domain::conversation_runtime_envelope::ConversationRuntimeEnvelopeObservationRejection,
    ),
}

#[cfg(test)]
pub(crate) fn confirmed_test_terminal_receipt()
-> crate::domain::turn_terminal::ConversationTurnTerminalReceipt {
    use crate::domain::turn_terminal::{
        ConversationTurnApplicationDelivery, ConversationTurnTerminalReceipt,
    };

    ConversationTurnTerminalReceipt::completed("test-thread", "test-turn", Vec::new())
        .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed)
}

#[cfg(test)]
pub(crate) fn emit_confirmed_test_terminal_receipt(
    event_sender: &ConversationStreamSender,
    thread_id: &str,
    cwd: &str,
) -> anyhow::Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
    use crate::domain::conversation_runtime_envelope::{
        ConversationRuntimeConfigurationRequest, ConversationRuntimeEnvelope,
    };
    use crate::domain::turn_terminal::{
        ConversationTurnApplicationDelivery, ConversationTurnTerminalReceipt,
    };

    let turn_id = "test-turn";
    event_sender
        .send(ConversationStreamEvent::ThreadPrepared {
            thread_id: thread_id.to_string(),
            title: "Test thread".to_string(),
            cwd: cwd.to_string(),
            runtime_envelope: Box::new(ConversationRuntimeEnvelope::unobserved()),
        })
        .map_err(|_| anyhow::anyhow!("test stream event receiver disconnected"))?;
    event_sender
        .send(ConversationStreamEvent::TurnStarted {
            turn_id: turn_id.to_string(),
            runtime_request: Box::new(ConversationRuntimeConfigurationRequest::default()),
        })
        .map_err(|_| anyhow::anyhow!("test stream event receiver disconnected"))?;
    let receipt = ConversationTurnTerminalReceipt::completed(thread_id, turn_id, Vec::new())
        .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);
    event_sender
        .send(ConversationStreamEvent::TurnTerminal {
            receipt: receipt.clone(),
        })
        .map_err(|_| anyhow::anyhow!("test stream event receiver disconnected"))?;
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::Arc;
    use std::sync::mpsc::{RecvTimeoutError, SendError, TryRecvError, TrySendError, sync_channel};
    use std::thread;
    use std::time::Duration;

    use super::{
        CONVERSATION_STREAM_CHANNEL_CAPACITY, ConversationRuntimeEnvelopeProjection,
        ConversationRuntimeEnvelopeProjectionRejection, ConversationStreamEvent,
        conversation_stream_channel, emit_confirmed_test_terminal_receipt,
    };
    use crate::domain::conversation::{
        ConversationApprovalRequest, ConversationApprovalRequestKind,
    };
    use crate::domain::conversation_item_lifecycle::{
        ConversationItemKind, ConversationItemLifecycleObservation, ConversationItemLifecyclePhase,
        ConversationItemLifecycleSource, ConversationItemOutcome,
    };
    use crate::domain::conversation_progressive_activity::{
        ConversationProgressiveActivityBatch, ConversationProgressiveActivityKind,
        ConversationProgressiveActivityObservation, ConversationProgressiveActivityPayload,
        MAX_PROGRESSIVE_AGENT_DRAFT_BYTES, MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES,
    };
    use crate::domain::conversation_runtime_envelope::{
        ConversationRuntimeConfigurationRequest, ConversationRuntimeEnvelope,
    };
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
    use crate::domain::turn_terminal::{
        ConversationTurnApplicationDelivery, ConversationTurnError, ConversationTurnTerminalReceipt,
    };

    fn status_event(sequence: usize) -> ConversationStreamEvent {
        ConversationStreamEvent::StatusUpdated {
            text: sequence.to_string(),
        }
    }

    fn progressive_event(sequence: u64) -> ConversationStreamEvent {
        progressive_event_for("thread-mailbox", "turn-mailbox", sequence)
    }

    fn progressive_event_for(
        thread_id: &str,
        turn_id: &str,
        sequence: u64,
    ) -> ConversationStreamEvent {
        let batch = ConversationProgressiveActivityBatch::single(
            ConversationProgressiveActivityObservation {
                sequence,
                thread_id: thread_id.to_string(),
                turn_id: Some(turn_id.to_string()),
                item_id: Some("item-plan".to_string()),
                kind: ConversationProgressiveActivityKind::PlanDelta,
                payload: ConversationProgressiveActivityPayload::PlanDelta {
                    chunk_count: 1,
                    source_bytes: 1,
                },
            },
        )
        .expect("test progressive activity should be valid");
        ConversationStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(batch),
        }
    }

    fn progressive_agent_event(sequence: u64, text: &str) -> ConversationStreamEvent {
        let batch = ConversationProgressiveActivityBatch::single(
            ConversationProgressiveActivityObservation {
                sequence,
                thread_id: "thread-mailbox".to_string(),
                turn_id: Some("turn-mailbox".to_string()),
                item_id: Some(format!("item-agent-{sequence}")),
                kind: ConversationProgressiveActivityKind::AgentMessageDelta,
                payload: ConversationProgressiveActivityPayload::AgentMessageDelta {
                    phase: None,
                    text: text.to_string(),
                    source_bytes: text.len() as u64,
                    truncated_bytes: 0,
                },
            },
        )
        .expect("test progressive agent activity should be valid");
        ConversationStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(batch),
        }
    }

    fn progressive_agent_event_with_accounting(
        sequence: u64,
        text: &str,
        source_bytes: u64,
        truncated_bytes: u64,
    ) -> ConversationStreamEvent {
        let batch = ConversationProgressiveActivityBatch::single(
            ConversationProgressiveActivityObservation {
                sequence,
                thread_id: "thread-mailbox".to_string(),
                turn_id: Some("turn-mailbox".to_string()),
                item_id: Some("item-agent-accounting".to_string()),
                kind: ConversationProgressiveActivityKind::AgentMessageDelta,
                payload: ConversationProgressiveActivityPayload::AgentMessageDelta {
                    phase: None,
                    text: text.to_string(),
                    source_bytes,
                    truncated_bytes,
                },
            },
        )
        .expect("accounting boundary fixture should be publicly valid");
        ConversationStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(batch),
        }
    }

    fn empty_progressive_event() -> ConversationStreamEvent {
        ConversationStreamEvent::ProgressiveActivityObserved {
            batch: Box::default(),
        }
    }

    fn guardian_progressive_event(sequence: u64) -> ConversationStreamEvent {
        let message = "guardian warning".to_string();
        let batch = ConversationProgressiveActivityBatch::single(
            ConversationProgressiveActivityObservation {
                sequence,
                thread_id: "thread-mailbox".to_string(),
                turn_id: None,
                item_id: None,
                kind: ConversationProgressiveActivityKind::GuardianWarning,
                payload: ConversationProgressiveActivityPayload::GuardianWarning {
                    source_bytes: message.len() as u64,
                    message,
                    update_count: 1,
                    truncated_bytes: 0,
                },
            },
        )
        .expect("test guardian activity should be valid");
        ConversationStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(batch),
        }
    }

    fn approval_request() -> ConversationApprovalRequest {
        ConversationApprovalRequest {
            approval_id: "approval-1".to_string(),
            server_request_id: "request-1".to_string(),
            method: "item/commandExecution/requestApproval".to_string(),
            kind: ConversationApprovalRequestKind::CommandExecution,
            summary: "approve command".to_string(),
            details: vec!["cargo test".to_string()],
        }
    }

    fn item_lifecycle_event() -> ConversationStreamEvent {
        ConversationStreamEvent::ItemLifecycleObserved {
            observation: Box::new(ConversationItemLifecycleObservation {
                thread_id: "thread-mailbox".to_string(),
                turn_id: "turn-mailbox".to_string(),
                item_id: "item-plan".to_string(),
                kind: ConversationItemKind::Plan,
                phase: ConversationItemLifecyclePhase::Started,
                source: ConversationItemLifecycleSource::Live,
                observed_at_ms: Some(1),
                outcome: ConversationItemOutcome::InProgress,
                summary: "plan started".to_string(),
            }),
        }
    }

    fn retry_event() -> ConversationStreamEvent {
        ConversationStreamEvent::TurnRetrying {
            thread_id: "thread-mailbox".to_string(),
            turn_id: "turn-mailbox".to_string(),
            error: ConversationTurnError::new("retrying", None::<&str>, None),
        }
    }

    fn terminal_event() -> ConversationStreamEvent {
        ConversationStreamEvent::TurnTerminal {
            receipt: ConversationTurnTerminalReceipt::completed(
                "thread-mailbox",
                "turn-mailbox",
                Vec::new(),
            )
            .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed),
        }
    }

    #[test]
    fn codex_attachment_helpers_build_expected_profiles() {
        assert_eq!(
            ConversationStreamEvent::codex_app_server_launch_attachment(),
            ConversationStreamEvent::AttachmentObserved {
                profile: TerminalBridgeAttachmentProfile::codex_app_server_launch(),
            }
        );
        assert_eq!(
            ConversationStreamEvent::codex_app_server_reattach_attachment(),
            ConversationStreamEvent::AttachmentObserved {
                profile: TerminalBridgeAttachmentProfile::codex_app_server_reattach(),
            }
        );
    }

    #[test]
    fn conversation_stream_channel_applies_fifo_backpressure_and_disconnects() {
        let (sender, receiver) = conversation_stream_channel();
        for sequence in 0..CONVERSATION_STREAM_CHANNEL_CAPACITY - 1 {
            sender
                .try_send(status_event(sequence))
                .expect("events within the fixed capacity should be admitted");
        }
        let approval = ConversationApprovalRequest {
            approval_id: "approval-1".to_string(),
            server_request_id: "request-1".to_string(),
            method: "item/commandExecution/requestApproval".to_string(),
            kind: ConversationApprovalRequestKind::CommandExecution,
            summary: "approve command".to_string(),
            details: vec!["cargo test".to_string()],
        };
        sender
            .try_send(ConversationStreamEvent::ApprovalRequested {
                request: approval.clone(),
            })
            .expect("approval control event should retain FIFO admission");

        assert!(matches!(
            sender.try_send(ConversationStreamEvent::StatusUpdated {
                text: "overflow".to_string(),
            }),
            Err(TrySendError::Full(_))
        ));
        sender
            .try_send(progressive_event(0))
            .expect("progressive activity must bypass a full control FIFO");
        for sequence in 0..CONVERSATION_STREAM_CHANNEL_CAPACITY - 1 {
            assert_eq!(
                receiver
                    .recv()
                    .expect("queued event should remain available"),
                status_event(sequence)
            );
        }
        assert_eq!(
            receiver
                .recv()
                .expect("approval control event should remain queued"),
            ConversationStreamEvent::ApprovalRequested { request: approval }
        );
        assert!(matches!(
            receiver
                .recv()
                .expect("progressive activity should remain available after controls"),
            ConversationStreamEvent::ProgressiveActivityObserved { .. }
        ));

        drop(receiver);
        assert!(
            sender
                .send(ConversationStreamEvent::StatusUpdated {
                    text: "disconnected".to_string(),
                })
                .is_err()
        );
    }

    #[test]
    fn adjacent_progressive_burst_keeps_one_segment_and_merges_one_hundred_thousand_publications() {
        const PUBLICATION_COUNT: u64 = 100_000;
        let (sender, receiver) = conversation_stream_channel();
        for sequence in 0..PUBLICATION_COUNT {
            sender
                .try_send(progressive_event(sequence))
                .expect("progressive pressure must never report control FIFO full");
        }

        {
            let state = sender.shared.lock_state();
            assert_eq!(state.control_event_count, 0);
            assert_eq!(state.pending_events.len(), 1);
            assert!(
                state.progressive_dynamic_bytes <= MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES
            );
        }
        let ConversationStreamEvent::ProgressiveActivityObserved { batch } = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("merged progressive batch should remain available")
        else {
            panic!("progressive segment returned a control event")
        };
        assert_eq!(batch.records().len(), 1);
        assert_eq!(batch.first_sequence(), Some(0));
        assert_eq!(batch.last_sequence(), Some(PUBLICATION_COUNT - 1));
        assert_eq!(batch.source_observation_count(), PUBLICATION_COUNT);
        assert_eq!(batch.coalesced_observation_count(), PUBLICATION_COUNT - 1);
        assert_eq!(batch.superseded_publication_count(), PUBLICATION_COUNT - 1);
        assert_eq!(batch.records()[0].observation_count(), PUBLICATION_COUNT);
        let ConversationProgressiveActivityPayload::PlanDelta {
            chunk_count,
            source_bytes,
        } = &batch.records()[0].observation().payload
        else {
            panic!("merged record should retain its plan-delta payload")
        };
        assert_eq!(*chunk_count, PUBLICATION_COUNT);
        assert_eq!(*source_bytes, PUBLICATION_COUNT);
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn adjacent_guardian_and_turn_activity_share_monotonic_thread_correlation() {
        let (sender, receiver) = conversation_stream_channel();
        sender
            .try_send(guardian_progressive_event(0))
            .expect("thread-scoped guardian activity should be admitted");
        sender
            .try_send(progressive_event(1))
            .expect("turn-scoped activity should merge without an invented guardian turn id");

        let ConversationStreamEvent::ProgressiveActivityObserved { batch } = receiver
            .recv()
            .expect("adjacent correlated activity should remain available")
        else {
            panic!("adjacent correlated activity returned a control event")
        };
        assert_eq!(batch.thread_id(), Some("thread-mailbox"));
        assert_eq!(batch.turn_id(), Some("turn-mailbox"));
        assert_eq!(batch.first_sequence(), Some(0));
        assert_eq!(batch.last_sequence(), Some(1));
        assert_eq!(batch.source_observation_count(), 2);
        assert_eq!(batch.records().len(), 2);
        assert_eq!(
            batch.records()[0].observation().kind,
            ConversationProgressiveActivityKind::GuardianWarning
        );
        assert_eq!(batch.records()[0].observation().turn_id, None);
        assert_eq!(
            batch.records()[1].observation().kind,
            ConversationProgressiveActivityKind::PlanDelta
        );
        assert_eq!(
            batch.records()[1].observation().turn_id.as_deref(),
            Some("turn-mailbox")
        );
        assert_eq!(batch.superseded_publication_count(), 1);
    }

    #[test]
    fn progressive_pressure_preserves_control_admission_fifo_and_preterminal_delivery() {
        let (sender, receiver) = conversation_stream_channel();
        sender
            .try_send(status_event(0))
            .expect("leading control event should be admitted");
        for sequence in 0..10_000 {
            sender
                .try_send(progressive_event(sequence))
                .expect("progressive pressure should use only its coalescible segment");
        }
        let controls = vec![
            ConversationStreamEvent::ApprovalRequested {
                request: approval_request(),
            },
            item_lifecycle_event(),
            retry_event(),
            ConversationStreamEvent::Failed {
                message: "terminal candidate error".to_string(),
            },
            terminal_event(),
        ];
        for event in &controls {
            sender
                .try_send(event.clone())
                .expect("progressive pressure must not consume control FIFO capacity");
        }

        assert_eq!(
            receiver.recv().expect("leading control should arrive"),
            status_event(0)
        );
        let ConversationStreamEvent::ProgressiveActivityObserved { batch } = receiver
            .recv()
            .expect("latest progress should precede terminal")
        else {
            panic!("pending progressive activity was reordered behind later controls")
        };
        assert_eq!(batch.source_observation_count(), 10_000);
        for expected in controls {
            assert_eq!(
                receiver.recv().expect("control event should remain queued"),
                expected
            );
        }
    }

    #[test]
    fn item_control_boundary_preserves_exact_progress_control_progress_order() {
        let (sender, receiver) = conversation_stream_channel();
        sender
            .try_send(progressive_event(0))
            .expect("initial progress should occupy its queue segment");
        let item_boundary = item_lifecycle_event();
        sender
            .try_send(item_boundary.clone())
            .expect("item lifecycle should form an exact ordering boundary");
        sender
            .try_send(progressive_event(1))
            .expect("later progress should occupy a distinct queue segment");

        let ConversationStreamEvent::ProgressiveActivityObserved { batch } = receiver
            .recv()
            .expect("leading progress should arrive before its item boundary")
        else {
            panic!("leading progress was reordered across its item boundary")
        };
        assert_eq!(batch.first_sequence(), Some(0));
        assert_eq!(
            receiver.recv().expect("item boundary should arrive second"),
            item_boundary
        );
        let ConversationStreamEvent::ProgressiveActivityObserved { batch } = receiver
            .recv()
            .expect("trailing progress should arrive after its item boundary")
        else {
            panic!("trailing progress was reordered across its item boundary")
        };
        assert_eq!(batch.first_sequence(), Some(1));
        assert_eq!(batch.source_observation_count(), 1);
        assert_eq!(batch.superseded_publication_count(), 0);
    }

    #[test]
    fn maximum_control_boundaries_keep_progressive_memory_globally_bounded() {
        let (sender, receiver) = conversation_stream_channel();
        let detail = "x".repeat(MAX_PROGRESSIVE_AGENT_DRAFT_BYTES);

        for sequence in 0..=CONVERSATION_STREAM_CHANNEL_CAPACITY {
            sender
                .try_send(progressive_agent_event(sequence as u64, &detail))
                .expect("progressive segments must never consume control admission");
            if sequence < CONVERSATION_STREAM_CHANNEL_CAPACITY {
                sender
                    .try_send(status_event(sequence))
                    .expect("all eight control boundaries should remain admitted");
            }
        }

        {
            let state = sender.shared.lock_state();
            assert_eq!(
                state.pending_events.len(),
                CONVERSATION_STREAM_CHANNEL_CAPACITY * 2 + 1
            );
            assert_eq!(
                state.control_event_count,
                CONVERSATION_STREAM_CHANNEL_CAPACITY
            );
            assert!(
                state.progressive_dynamic_bytes <= MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES
            );
            assert_eq!(
                state.progressive_dynamic_bytes,
                state
                    .pending_events
                    .iter()
                    .map(super::progressive_event_dynamic_bytes)
                    .sum::<usize>()
            );
            let history_only_segments = state
                .pending_events
                .iter()
                .filter_map(|event| match event {
                    ConversationStreamEvent::ProgressiveActivityObserved { batch } => Some(batch),
                    _ => None,
                })
                .filter(|batch| batch.records().is_empty())
                .count();
            assert!(
                history_only_segments > 0,
                "older progressive detail should be discarded under the global bound"
            );
        }

        assert!(matches!(
            sender.try_send(status_event(CONVERSATION_STREAM_CHANNEL_CAPACITY)),
            Err(TrySendError::Full(_))
        ));

        for sequence in 0..=CONVERSATION_STREAM_CHANNEL_CAPACITY {
            let ConversationStreamEvent::ProgressiveActivityObserved { batch } = receiver
                .recv()
                .expect("each progressive ordering segment should remain present")
            else {
                panic!("control boundary moved ahead of its progressive segment")
            };
            batch
                .validate()
                .expect("mailbox truncation must preserve a structurally valid batch");
            assert_eq!(batch.first_sequence(), Some(sequence as u64));
            assert_eq!(batch.last_sequence(), Some(sequence as u64));
            assert_eq!(batch.source_observation_count(), 1);
            if batch.records().is_empty() {
                assert!(batch.loss_event_count() >= 1);
                assert!(batch.history_incomplete());
            }
            if sequence < CONVERSATION_STREAM_CHANNEL_CAPACITY {
                assert_eq!(
                    receiver
                        .recv()
                        .expect("control boundary should remain next"),
                    status_event(sequence)
                );
            }
        }

        let state = sender.shared.lock_state();
        assert_eq!(state.progressive_dynamic_bytes, 0);
        assert_eq!(state.control_event_count, 0);
        assert!(state.pending_events.is_empty());
    }

    #[test]
    fn slow_consumer_observes_control_fifo_and_bounded_progressive_segments() {
        const PUBLICATION_COUNT: u64 = 25_000;
        let (sender, receiver) = conversation_stream_channel();
        let producer = thread::spawn(move || {
            for sequence in 0..PUBLICATION_COUNT {
                sender
                    .try_send(progressive_event(sequence))
                    .expect("slow consumption must not apply progressive backpressure");
                if sequence % 5_000 == 0 {
                    sender
                        .try_send(status_event((sequence / 5_000) as usize))
                        .expect("bounded control sample should remain admitted");
                }
            }
        });

        thread::sleep(Duration::from_millis(20));
        producer.join().expect("producer should not deadlock");
        let events = receiver.iter().collect::<Vec<_>>();
        let control_labels = events
            .iter()
            .filter_map(|event| match event {
                ConversationStreamEvent::StatusUpdated { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(control_labels, vec!["0", "1", "2", "3", "4"]);
        let batches = events
            .iter()
            .filter_map(|event| match event {
                ConversationStreamEvent::ProgressiveActivityObserved { batch } => Some(batch),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            batches
                .iter()
                .map(|batch| batch.source_observation_count())
                .sum::<u64>(),
            PUBLICATION_COUNT
        );
        assert_eq!(
            batches
                .iter()
                .map(|batch| batch.superseded_publication_count())
                .sum::<u64>(),
            PUBLICATION_COUNT - batches.len() as u64
        );
    }

    #[test]
    fn incompatible_adjacent_progressive_publications_are_rejected_without_mutating_mailbox() {
        let (sender, receiver) = conversation_stream_channel();
        sender
            .send(progressive_event(0))
            .expect("first progressive publication should be admitted");
        let bytes_before = sender.shared.lock_state().progressive_dynamic_bytes;

        let wrong_correlation = progressive_event_for("thread-other", "turn-other", 1);
        assert!(matches!(
            sender.send(wrong_correlation),
            Err(SendError(
                ConversationStreamEvent::ProgressiveActivityObserved { .. }
            ))
        ));
        assert!(matches!(
            sender.try_send(progressive_event(0)),
            Err(TrySendError::Full(
                ConversationStreamEvent::ProgressiveActivityObserved { .. }
            ))
        ));
        {
            let state = sender.shared.lock_state();
            assert_eq!(state.pending_events.len(), 1);
            assert_eq!(state.progressive_dynamic_bytes, bytes_before);
        }

        sender
            .try_send(progressive_event(1))
            .expect("a later compatible publication should still merge");
        let ConversationStreamEvent::ProgressiveActivityObserved { batch } = receiver
            .recv()
            .expect("compatible progressive batch should remain available")
        else {
            panic!("progressive publication returned a control event")
        };
        assert_eq!(batch.first_sequence(), Some(0));
        assert_eq!(batch.last_sequence(), Some(1));
        assert_eq!(batch.source_observation_count(), 2);
    }

    #[test]
    fn payload_counter_overflow_rejects_owned_publication_without_mutating_mailbox() {
        let (sender, receiver) = conversation_stream_channel();
        let first = progressive_agent_event_with_accounting(0, "x", u64::MAX, u64::MAX - 1);
        let expected_first = first.clone();
        sender
            .try_send(first)
            .expect("first extreme publication should be admitted");
        let bytes_before = sender.shared.lock_state().progressive_dynamic_bytes;

        let incoming = progressive_agent_event_with_accounting(1, "y", 1, 0);
        let expected_incoming = incoming.clone();
        let Err(TrySendError::Full(returned)) = sender.try_send(incoming) else {
            panic!("overflowing publication should be rejected with ownership")
        };
        assert_eq!(returned, expected_incoming);
        {
            let state = sender.shared.lock_state();
            assert_eq!(state.pending_events.len(), 1);
            assert_eq!(state.progressive_dynamic_bytes, bytes_before);
        }

        let received = receiver
            .try_recv()
            .expect("the original extreme publication should remain intact");
        assert_eq!(received, expected_first);
        let ConversationStreamEvent::ProgressiveActivityObserved { batch } = received else {
            unreachable!()
        };
        assert_eq!(batch.source_observation_count(), 1);
        assert_eq!(batch.superseded_publication_count(), 0);
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn default_empty_progressive_publication_is_a_zero_accounting_no_op() {
        let (sender, receiver) = conversation_stream_channel();
        sender
            .try_send(empty_progressive_event())
            .expect("default empty publication should succeed");
        {
            let state = sender.shared.lock_state();
            assert_eq!(state.pending_events.len(), 0);
            assert_eq!(state.progressive_dynamic_bytes, 0);
        }

        sender
            .try_send(progressive_agent_event(0, "kept"))
            .expect("non-empty publication should be admitted");
        let bytes_before = sender.shared.lock_state().progressive_dynamic_bytes;
        sender
            .try_send(empty_progressive_event())
            .expect("empty publication after detail should remain a no-op");
        {
            let state = sender.shared.lock_state();
            assert_eq!(state.pending_events.len(), 1);
            assert_eq!(state.progressive_dynamic_bytes, bytes_before);
        }

        let ConversationStreamEvent::ProgressiveActivityObserved { batch } = receiver
            .try_recv()
            .expect("the non-empty publication should remain available")
        else {
            unreachable!()
        };
        assert_eq!(batch.source_observation_count(), 1);
        assert_eq!(batch.superseded_publication_count(), 0);
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn blocking_control_send_waits_for_capacity_and_keeps_fifo_order() {
        let (sender, receiver) = conversation_stream_channel();
        for sequence in 0..CONVERSATION_STREAM_CHANNEL_CAPACITY {
            sender
                .try_send(status_event(sequence))
                .expect("control FIFO should fill to its exact capacity");
        }

        let blocked_sender = sender.clone();
        let (started_tx, started_rx) = sync_channel(1);
        let (finished_tx, finished_rx) = sync_channel(1);
        let producer = thread::spawn(move || {
            started_tx
                .send(())
                .expect("test start signal should arrive");
            let result = blocked_sender.send(status_event(CONVERSATION_STREAM_CHANNEL_CAPACITY));
            finished_tx
                .send(result)
                .expect("test completion signal should arrive");
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("producer should reach the blocking send");
        assert_eq!(
            finished_rx.recv_timeout(Duration::from_millis(20)),
            Err(RecvTimeoutError::Timeout),
            "ninth control event must wait while the FIFO is full"
        );

        assert_eq!(
            receiver.recv().expect("oldest event should drain"),
            status_event(0)
        );
        finished_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("capacity release should wake the blocked sender")
            .expect("receiver is still connected");
        producer.join().expect("producer should not deadlock");

        for sequence in 1..=CONVERSATION_STREAM_CHANNEL_CAPACITY {
            assert_eq!(
                receiver.recv().expect("remaining FIFO event should drain"),
                status_event(sequence)
            );
        }
    }

    #[test]
    fn receiver_drop_wakes_blocked_sender_with_original_event() {
        let (sender, receiver) = conversation_stream_channel();
        for sequence in 0..CONVERSATION_STREAM_CHANNEL_CAPACITY {
            sender
                .try_send(status_event(sequence))
                .expect("control FIFO should fill");
        }
        let blocked_sender = sender.clone();
        let (started_tx, started_rx) = sync_channel(1);
        let producer = thread::spawn(move || {
            started_tx
                .send(())
                .expect("test start signal should arrive");
            blocked_sender
                .send(status_event(99))
                .map_err(|error| Box::new(error.0))
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("producer should reach the blocking send");

        drop(receiver);
        let error = producer
            .join()
            .expect("producer should wake when the receiver drops")
            .expect_err("disconnected receiver must reject the pending event");
        assert_eq!(*error, status_event(99));
        assert!(matches!(
            sender.try_send(status_event(100)),
            Err(TrySendError::Disconnected(event)) if event == status_event(100)
        ));
        assert!(matches!(
            sender.try_send(progressive_event(100)),
            Err(TrySendError::Disconnected(
                ConversationStreamEvent::ProgressiveActivityObserved { .. }
            ))
        ));
    }

    #[test]
    fn timeout_sender_lifetime_and_iterator_match_mpsc_contract() {
        let (sender, receiver) = conversation_stream_channel();
        let last_sender = sender.clone();
        drop(sender);
        assert_eq!(
            receiver.recv_timeout(Duration::from_millis(10)),
            Err(RecvTimeoutError::Timeout),
            "a live cloned sender keeps an empty mailbox connected"
        );

        for sequence in 0..3 {
            last_sender
                .send(status_event(sequence))
                .expect("live sender should publish control events");
        }
        drop(last_sender);
        assert_eq!(
            receiver.iter().collect::<Vec<_>>(),
            vec![status_event(0), status_event(1), status_event(2)]
        );
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Disconnected));
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Err(RecvTimeoutError::Disconnected)
        );
    }

    #[test]
    fn poisoned_mailbox_mutex_recovers_without_losing_channel_liveness() {
        let (sender, receiver) = conversation_stream_channel();
        let shared = Arc::clone(&sender.shared);
        let poisoned = catch_unwind(AssertUnwindSafe(|| {
            let _state = shared
                .state
                .lock()
                .expect("fresh mailbox mutex should not start poisoned");
            panic!("poison mailbox for recovery test");
        }));
        assert!(poisoned.is_err());

        sender
            .try_send(status_event(7))
            .expect("poison recovery should retain sender admission");
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("poison recovery should retain receiver delivery"),
            status_event(7)
        );
    }

    #[test]
    fn confirmed_test_stream_emits_correlated_terminal_event_and_return_receipt() {
        let (sender, receiver) = conversation_stream_channel();

        let returned =
            emit_confirmed_test_terminal_receipt(&sender, "thread-test", "/tmp/test-workspace")
                .expect("test terminal stream should emit");

        assert!(matches!(
            receiver.recv().expect("thread event should arrive"),
            ConversationStreamEvent::ThreadPrepared {
                ref thread_id,
                ref cwd,
                ..
            } if thread_id == "thread-test" && cwd == "/tmp/test-workspace"
        ));
        assert!(matches!(
            receiver.recv().expect("turn event should arrive"),
            ConversationStreamEvent::TurnStarted { ref turn_id, .. } if turn_id == "test-turn"
        ));
        assert_eq!(
            receiver.recv().expect("terminal event should arrive"),
            ConversationStreamEvent::TurnTerminal {
                receipt: returned.clone(),
            }
        );
        assert!(returned.is_completed_and_confirmed());
    }

    #[test]
    fn projection_keeps_out_of_order_turn_rejection_after_late_thread_preparation() {
        let mut projection = ConversationRuntimeEnvelopeProjection::default();
        projection.apply_event(&ConversationStreamEvent::TurnStarted {
            turn_id: "turn-1".to_string(),
            runtime_request: Box::new(ConversationRuntimeConfigurationRequest::default()),
        });
        projection.apply_event(&ConversationStreamEvent::ThreadPrepared {
            thread_id: "thread-1".to_string(),
            title: "Planning worker".to_string(),
            cwd: "/tmp/workspace".to_string(),
            runtime_envelope: Box::new(ConversationRuntimeEnvelope::unobserved()),
        });

        assert_eq!(
            projection.last_rejection,
            Some(ConversationRuntimeEnvelopeProjectionRejection::EnvelopeNotPrepared)
        );
        assert!(
            projection
                .runtime_envelope
                .as_ref()
                .is_some_and(|envelope| envelope.turn_request.is_none())
        );
    }
}
