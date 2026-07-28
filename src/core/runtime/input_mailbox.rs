use std::collections::VecDeque;
use std::fmt;
#[cfg(test)]
use std::sync::mpsc::TrySendError;
use std::sync::mpsc::{SendError, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use crate::core::app::{CoreInput, TurnStreamEvent};
use crate::domain::conversation_progressive_activity::MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES;

pub(super) const CORE_INPUT_CHANNEL_CAPACITY: usize = 16;
const CORE_PROGRESSIVE_SEGMENT_CAPACITY: usize = CORE_INPUT_CHANNEL_CAPACITY + 1;

pub(crate) struct CoreInputSender {
    shared: Arc<CoreInputMailbox>,
}

pub(crate) struct CoreInputReceiver {
    shared: Arc<CoreInputMailbox>,
}

struct CoreInputMailbox {
    state: Mutex<CoreInputMailboxState>,
    changed: Condvar,
}

struct CoreInputMailboxState {
    pending_inputs: VecDeque<CoreInput>,
    control_input_count: usize,
    progressive_segment_count: usize,
    progressive_dynamic_bytes: usize,
    latest_progressive_generation: Option<u64>,
    sender_count: usize,
    receiver_alive: bool,
}

impl CoreInputMailboxState {
    fn new() -> Self {
        Self {
            pending_inputs: VecDeque::with_capacity(
                CORE_INPUT_CHANNEL_CAPACITY + CORE_PROGRESSIVE_SEGMENT_CAPACITY,
            ),
            control_input_count: 0,
            progressive_segment_count: 0,
            progressive_dynamic_bytes: 0,
            latest_progressive_generation: None,
            sender_count: 1,
            receiver_alive: true,
        }
    }

    fn publish_control(&mut self, input: CoreInput) {
        self.pending_inputs.push_back(input);
        self.control_input_count = self.control_input_count.saturating_add(1);
    }

    fn publish_progressive(&mut self, input: CoreInput) -> Option<CoreInput> {
        if progressive_core_input_is_empty(&input) {
            return None;
        }
        let incoming_generation = progressive_core_input_generation(&input)
            .expect("progressive Core input must carry turn correlation");
        if self
            .latest_progressive_generation
            .is_some_and(|latest| incoming_generation < latest)
        {
            return None;
        }
        if self
            .latest_progressive_generation
            .is_none_or(|latest| incoming_generation > latest)
        {
            self.latest_progressive_generation = Some(incoming_generation);
            self.pending_inputs.retain(|pending| {
                !is_progressive_core_input(pending)
                    || progressive_core_input_generation(pending) == Some(incoming_generation)
            });
            self.recompute_progressive_accounting();
        }
        if self
            .pending_inputs
            .back()
            .is_some_and(|pending| progressive_core_inputs_can_merge(pending, &input))
        {
            let pending = self
                .pending_inputs
                .back_mut()
                .expect("progressive Core input tail should remain available");
            self.progressive_dynamic_bytes = self
                .progressive_dynamic_bytes
                .saturating_sub(progressive_core_input_dynamic_bytes(pending));
            merge_progressive_core_inputs(pending, input);
            self.progressive_dynamic_bytes = self
                .progressive_dynamic_bytes
                .saturating_add(progressive_core_input_dynamic_bytes(pending));
        } else {
            if self.progressive_segment_count == CORE_PROGRESSIVE_SEGMENT_CAPACITY {
                return Some(input);
            }
            self.progressive_dynamic_bytes = self
                .progressive_dynamic_bytes
                .saturating_add(progressive_core_input_dynamic_bytes(&input));
            self.pending_inputs.push_back(input);
            self.progressive_segment_count = self.progressive_segment_count.saturating_add(1);
        }
        self.enforce_progressive_memory_bound();
        None
    }

    fn recompute_progressive_accounting(&mut self) {
        self.progressive_segment_count = self
            .pending_inputs
            .iter()
            .filter(|input| is_progressive_core_input(input))
            .count();
        self.progressive_dynamic_bytes = self
            .pending_inputs
            .iter()
            .map(progressive_core_input_dynamic_bytes)
            .fold(0usize, usize::saturating_add);
    }

    fn pop_next(&mut self) -> Option<CoreInput> {
        let input = self.pending_inputs.pop_front()?;
        if is_progressive_core_input(&input) {
            self.progressive_segment_count = self.progressive_segment_count.saturating_sub(1);
            self.progressive_dynamic_bytes = self
                .progressive_dynamic_bytes
                .saturating_sub(progressive_core_input_dynamic_bytes(&input));
        } else {
            self.control_input_count = self.control_input_count.saturating_sub(1);
        }
        Some(input)
    }

    fn enforce_progressive_memory_bound(&mut self) {
        if self.progressive_dynamic_bytes <= MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES {
            return;
        }

        for input in &mut self.pending_inputs {
            let before = progressive_core_input_dynamic_bytes(input);
            if before == 0 {
                continue;
            }
            discard_progressive_core_input_records(input);
            let after = progressive_core_input_dynamic_bytes(input);
            self.progressive_dynamic_bytes = self
                .progressive_dynamic_bytes
                .saturating_sub(before)
                .saturating_add(after);
            if self.progressive_dynamic_bytes <= MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES {
                break;
            }
        }
    }
}

impl CoreInputSender {
    #[allow(clippy::result_large_err)]
    pub(crate) fn send(&self, input: CoreInput) -> Result<(), SendError<CoreInput>> {
        let mut state = self.shared.lock_state();
        if !state.receiver_alive {
            return Err(SendError(input));
        }
        if is_progressive_core_input(&input) {
            if let Some(input) = state.publish_progressive(input) {
                return Err(SendError(input));
            }
            drop(state);
            self.shared.changed.notify_one();
            return Ok(());
        }

        while state.control_input_count == CORE_INPUT_CHANNEL_CAPACITY {
            state = self.shared.wait_for_change(state);
            if !state.receiver_alive {
                return Err(SendError(input));
            }
        }
        state.publish_control(input);
        drop(state);
        self.shared.changed.notify_one();
        Ok(())
    }

    #[cfg(test)]
    #[allow(clippy::result_large_err)]
    pub(super) fn try_send(&self, input: CoreInput) -> Result<(), TrySendError<CoreInput>> {
        let mut state = self.shared.lock_state();
        if !state.receiver_alive {
            return Err(TrySendError::Disconnected(input));
        }
        if is_progressive_core_input(&input) {
            if let Some(input) = state.publish_progressive(input) {
                return Err(TrySendError::Full(input));
            }
            drop(state);
            self.shared.changed.notify_one();
            return Ok(());
        }
        if state.control_input_count == CORE_INPUT_CHANNEL_CAPACITY {
            return Err(TrySendError::Full(input));
        }
        state.publish_control(input);
        drop(state);
        self.shared.changed.notify_one();
        Ok(())
    }
}

impl Clone for CoreInputSender {
    fn clone(&self) -> Self {
        let mut state = self.shared.lock_state();
        state.sender_count = state.sender_count.saturating_add(1);
        drop(state);
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl Drop for CoreInputSender {
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

impl fmt::Debug for CoreInputSender {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CoreInputSender")
            .finish_non_exhaustive()
    }
}

impl CoreInputReceiver {
    pub(super) fn try_recv(&self) -> Result<CoreInput, TryRecvError> {
        let mut state = self.shared.lock_state();
        if let Some(input) = state.pop_next() {
            drop(state);
            self.shared.changed.notify_one();
            return Ok(input);
        }
        if state.sender_count == 0 {
            Err(TryRecvError::Disconnected)
        } else {
            Err(TryRecvError::Empty)
        }
    }
}

impl Drop for CoreInputReceiver {
    fn drop(&mut self) {
        let mut state = self.shared.lock_state();
        state.receiver_alive = false;
        state.pending_inputs.clear();
        state.control_input_count = 0;
        state.progressive_segment_count = 0;
        state.progressive_dynamic_bytes = 0;
        state.latest_progressive_generation = None;
        drop(state);
        self.shared.changed.notify_all();
    }
}

impl CoreInputMailbox {
    fn lock_state(&self) -> MutexGuard<'_, CoreInputMailboxState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wait_for_change<'a>(
        &self,
        state: MutexGuard<'a, CoreInputMailboxState>,
    ) -> MutexGuard<'a, CoreInputMailboxState> {
        self.changed
            .wait(state)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn is_progressive_core_input(input: &CoreInput) -> bool {
    matches!(
        input,
        CoreInput::ConversationStreamUpdated {
            event: TurnStreamEvent::ProgressiveActivityObserved { .. },
            ..
        }
    )
}

fn progressive_core_input_is_empty(input: &CoreInput) -> bool {
    matches!(
        input,
        CoreInput::ConversationStreamUpdated {
            event: TurnStreamEvent::ProgressiveActivityObserved { batch },
            ..
        } if batch.source_observation_count() == 0
    )
}

fn progressive_core_input_generation(input: &CoreInput) -> Option<u64> {
    match input {
        CoreInput::ConversationStreamUpdated {
            correlation,
            event: TurnStreamEvent::ProgressiveActivityObserved { .. },
        } => Some(correlation.generation),
        _ => None,
    }
}

fn progressive_core_inputs_can_merge(pending: &CoreInput, incoming: &CoreInput) -> bool {
    let (
        CoreInput::ConversationStreamUpdated {
            correlation: pending_correlation,
            event:
                TurnStreamEvent::ProgressiveActivityObserved {
                    batch: pending_batch,
                },
        },
        CoreInput::ConversationStreamUpdated {
            correlation: incoming_correlation,
            event:
                TurnStreamEvent::ProgressiveActivityObserved {
                    batch: incoming_batch,
                },
        },
    ) = (pending, incoming)
    else {
        return false;
    };
    if pending_correlation != incoming_correlation {
        return false;
    }
    if pending_batch.source_observation_count() == 0
        || incoming_batch.source_observation_count() == 0
    {
        return false;
    }
    pending_batch
        .validate_publication_merge_candidate(incoming_batch)
        .is_ok()
}

fn merge_progressive_core_inputs(pending: &mut CoreInput, incoming: CoreInput) {
    let (
        CoreInput::ConversationStreamUpdated {
            event:
                TurnStreamEvent::ProgressiveActivityObserved {
                    batch: pending_batch,
                },
            ..
        },
        CoreInput::ConversationStreamUpdated {
            event:
                TurnStreamEvent::ProgressiveActivityObserved {
                    batch: incoming_batch,
                },
            ..
        },
    ) = (pending, incoming)
    else {
        debug_assert!(
            false,
            "progressive Core input classification must remain stable"
        );
        return;
    };
    if pending_batch.try_merge_from(*incoming_batch).is_ok()
        && pending_batch.record_superseded_publication().is_ok()
    {
    } else {
        debug_assert!(
            false,
            "preflight-approved progressive Core merge must remain valid"
        );
    }
}

fn progressive_core_input_dynamic_bytes(input: &CoreInput) -> usize {
    match input {
        CoreInput::ConversationStreamUpdated {
            event: TurnStreamEvent::ProgressiveActivityObserved { batch },
            ..
        } => batch.retained_dynamic_bytes(),
        _ => 0,
    }
}

fn discard_progressive_core_input_records(input: &mut CoreInput) {
    let CoreInput::ConversationStreamUpdated {
        event: TurnStreamEvent::ProgressiveActivityObserved { batch },
        ..
    } = input
    else {
        return;
    };
    batch.discard_retained_records();
}

pub(crate) fn core_input_channel() -> (CoreInputSender, CoreInputReceiver) {
    let shared = Arc::new(CoreInputMailbox {
        state: Mutex::new(CoreInputMailboxState::new()),
        changed: Condvar::new(),
    });
    (
        CoreInputSender {
            shared: Arc::clone(&shared),
        },
        CoreInputReceiver { shared },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{TryRecvError, TrySendError};

    use super::*;
    use crate::core::app::TurnSubmissionCorrelation;
    use crate::domain::conversation::{
        ConversationApprovalRequest, ConversationApprovalRequestKind,
    };
    use crate::domain::conversation_progressive_activity::{
        ConversationProgressiveActivityBatch, ConversationProgressiveActivityKind,
        ConversationProgressiveActivityObservation, ConversationProgressiveActivityPayload,
        MAX_PROGRESSIVE_AGENT_DRAFT_BYTES,
    };
    use crate::domain::turn_terminal::{
        ConversationTurnApplicationDelivery, ConversationTurnTerminalReceipt,
    };

    fn correlation() -> TurnSubmissionCorrelation {
        TurnSubmissionCorrelation::new(7)
    }

    fn progressive_input(sequence: u64, detail: &str) -> CoreInput {
        progressive_input_for("thread-core-mailbox", sequence, detail)
    }

    fn progressive_input_for(thread_id: &str, sequence: u64, detail: &str) -> CoreInput {
        let batch = ConversationProgressiveActivityBatch::single(
            ConversationProgressiveActivityObservation {
                sequence,
                thread_id: thread_id.to_string(),
                turn_id: Some("turn-core-mailbox".to_string()),
                item_id: Some(format!("agent-{sequence}")),
                kind: ConversationProgressiveActivityKind::AgentMessageDelta,
                payload: ConversationProgressiveActivityPayload::AgentMessageDelta {
                    phase: None,
                    text: detail.to_string(),
                    source_bytes: detail.len() as u64,
                    truncated_bytes: 0,
                },
            },
        )
        .expect("Core mailbox progressive fixture should be valid");
        CoreInput::ConversationStreamUpdated {
            correlation: correlation(),
            event: TurnStreamEvent::ProgressiveActivityObserved {
                batch: Box::new(batch),
            },
        }
    }

    fn progressive_input_with_accounting(
        sequence: u64,
        text: &str,
        source_bytes: u64,
        truncated_bytes: u64,
    ) -> CoreInput {
        let batch = ConversationProgressiveActivityBatch::single(
            ConversationProgressiveActivityObservation {
                sequence,
                thread_id: "thread-core-mailbox".to_string(),
                turn_id: Some("turn-core-mailbox".to_string()),
                item_id: Some("agent-accounting".to_string()),
                kind: ConversationProgressiveActivityKind::AgentMessageDelta,
                payload: ConversationProgressiveActivityPayload::AgentMessageDelta {
                    phase: None,
                    text: text.to_string(),
                    source_bytes,
                    truncated_bytes,
                },
            },
        )
        .expect("Core mailbox accounting boundary fixture should be valid");
        CoreInput::ConversationStreamUpdated {
            correlation: correlation(),
            event: TurnStreamEvent::ProgressiveActivityObserved {
                batch: Box::new(batch),
            },
        }
    }

    fn empty_progressive_input() -> CoreInput {
        CoreInput::ConversationStreamUpdated {
            correlation: correlation(),
            event: TurnStreamEvent::ProgressiveActivityObserved {
                batch: Box::default(),
            },
        }
    }

    fn status_input(sequence: u64) -> CoreInput {
        CoreInput::ConversationStreamUpdated {
            correlation: correlation(),
            event: TurnStreamEvent::StatusUpdated {
                text: format!("status-{sequence}"),
            },
        }
    }

    fn approval_input() -> CoreInput {
        CoreInput::ConversationStreamUpdated {
            correlation: correlation(),
            event: TurnStreamEvent::ApprovalRequested {
                request: ConversationApprovalRequest {
                    approval_id: "approval-core-mailbox".to_string(),
                    server_request_id: "request-core-mailbox".to_string(),
                    method: "item/commandExecution/requestApproval".to_string(),
                    kind: ConversationApprovalRequestKind::CommandExecution,
                    summary: "approve bounded Core mailbox".to_string(),
                    details: vec!["cargo test".to_string()],
                },
            },
        }
    }

    fn terminal_input() -> CoreInput {
        CoreInput::ConversationStreamUpdated {
            correlation: correlation(),
            event: TurnStreamEvent::TurnTerminal {
                receipt: ConversationTurnTerminalReceipt::completed(
                    "thread-core-mailbox",
                    "turn-core-mailbox",
                    Vec::new(),
                )
                .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed),
                execution_snapshot_capture: None,
            },
        }
    }

    #[test]
    fn progressive_pressure_preserves_control_admission_order_and_global_bytes() {
        let (sender, receiver) = core_input_channel();
        let detail = "x".repeat(MAX_PROGRESSIVE_AGENT_DRAFT_BYTES);

        for sequence in 0..9u64 {
            sender
                .try_send(progressive_input(sequence, &detail))
                .expect("progressive segment should not consume control admission");
            if sequence < 8 {
                sender
                    .try_send(status_input(sequence))
                    .expect("interleaved control should retain admission");
            }
        }
        sender
            .try_send(approval_input())
            .expect("approval should survive progressive pressure");
        sender
            .try_send(terminal_input())
            .expect("terminal should survive progressive pressure");

        {
            let state = sender.shared.lock_state();
            let recomputed = state
                .pending_inputs
                .iter()
                .map(progressive_core_input_dynamic_bytes)
                .sum::<usize>();
            assert_eq!(state.progressive_dynamic_bytes, recomputed);
            assert!(recomputed <= MAX_RETAINED_PROGRESSIVE_ACTIVITY_DYNAMIC_BYTES);
            assert_eq!(state.progressive_segment_count, 9);
            assert_eq!(state.control_input_count, 10);
        }

        for sequence in 0..8u64 {
            assert_progressive_sequence(receiver.try_recv().unwrap(), sequence);
            assert_eq!(receiver.try_recv().unwrap(), status_input(sequence));
        }
        assert_progressive_sequence(receiver.try_recv().unwrap(), 8);
        assert_eq!(receiver.try_recv().unwrap(), approval_input());
        assert_eq!(receiver.try_recv().unwrap(), terminal_input());
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn progressive_segment_overflow_never_consumes_control_capacity() {
        let (sender, _receiver) = core_input_channel();
        for segment in 0..CORE_PROGRESSIVE_SEGMENT_CAPACITY {
            sender
                .try_send(progressive_input_for(
                    &format!("thread-{segment}"),
                    segment as u64,
                    "x",
                ))
                .expect("bounded incompatible progressive segment should be admitted");
        }

        let overflow = progressive_input_for(
            "thread-overflow",
            CORE_PROGRESSIVE_SEGMENT_CAPACITY as u64,
            "x",
        );
        assert!(matches!(
            sender.try_send(overflow),
            Err(TrySendError::Full(CoreInput::ConversationStreamUpdated {
                event: TurnStreamEvent::ProgressiveActivityObserved { .. },
                ..
            }))
        ));

        for sequence in 0..CORE_INPUT_CHANNEL_CAPACITY {
            sender
                .try_send(CoreInput::ConversationRuntimeNotice(sequence.to_string()))
                .expect("progressive segments must not consume control admission");
        }
        assert!(matches!(
            sender.try_send(CoreInput::ConversationRuntimeNotice("full".to_string())),
            Err(TrySendError::Full(CoreInput::ConversationRuntimeNotice(notice)))
                if notice == "full"
        ));
    }

    #[test]
    fn payload_counter_overflow_preserves_two_progressive_segments() {
        let (sender, receiver) = core_input_channel();
        let first = progressive_input_with_accounting(0, "x", u64::MAX, u64::MAX - 1);
        let expected_first = first.clone();
        sender
            .try_send(first)
            .expect("first extreme publication should be admitted");
        let incoming = progressive_input_with_accounting(1, "y", 1, 0);
        let expected_incoming = incoming.clone();
        sender
            .try_send(incoming)
            .expect("non-mergeable publication should use a second bounded segment");

        {
            let state = sender.shared.lock_state();
            assert_eq!(state.pending_inputs.len(), 2);
            assert_eq!(state.progressive_segment_count, 2);
            assert_eq!(
                state.progressive_dynamic_bytes,
                state
                    .pending_inputs
                    .iter()
                    .map(progressive_core_input_dynamic_bytes)
                    .sum::<usize>()
            );
        }
        for expected in [expected_first, expected_incoming] {
            let received = receiver
                .try_recv()
                .expect("each non-mergeable publication should retain its own segment");
            assert_eq!(received, expected);
            let CoreInput::ConversationStreamUpdated {
                event: TurnStreamEvent::ProgressiveActivityObserved { batch },
                ..
            } = received
            else {
                unreachable!()
            };
            assert_eq!(batch.source_observation_count(), 1);
            assert_eq!(batch.superseded_publication_count(), 0);
        }
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn default_empty_progressive_input_is_a_zero_accounting_no_op() {
        let (sender, receiver) = core_input_channel();
        sender
            .try_send(empty_progressive_input())
            .expect("default empty input should succeed");
        {
            let state = sender.shared.lock_state();
            assert_eq!(state.pending_inputs.len(), 0);
            assert_eq!(state.progressive_segment_count, 0);
            assert_eq!(state.progressive_dynamic_bytes, 0);
            assert_eq!(state.latest_progressive_generation, None);
        }

        sender
            .try_send(progressive_input(0, "kept"))
            .expect("non-empty input should be admitted");
        let bytes_before = sender.shared.lock_state().progressive_dynamic_bytes;
        sender
            .try_send(empty_progressive_input())
            .expect("empty input after detail should remain a no-op");
        {
            let state = sender.shared.lock_state();
            assert_eq!(state.pending_inputs.len(), 1);
            assert_eq!(state.progressive_segment_count, 1);
            assert_eq!(state.progressive_dynamic_bytes, bytes_before);
            assert_eq!(state.latest_progressive_generation, Some(7));
        }

        let CoreInput::ConversationStreamUpdated {
            event: TurnStreamEvent::ProgressiveActivityObserved { batch },
            ..
        } = receiver
            .try_recv()
            .expect("the non-empty input should remain available")
        else {
            unreachable!()
        };
        assert_eq!(batch.source_observation_count(), 1);
        assert_eq!(batch.superseded_publication_count(), 0);
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn newer_progressive_generation_prunes_pending_stale_detail_and_ignores_late_stale_input() {
        let (sender, receiver) = core_input_channel();
        let mut stale = progressive_input(0, "stale");
        let CoreInput::ConversationStreamUpdated { correlation, .. } = &mut stale else {
            unreachable!()
        };
        *correlation = TurnSubmissionCorrelation::new(6);
        sender.try_send(stale).unwrap();

        sender.try_send(progressive_input(0, "current")).unwrap();
        let mut late_stale = progressive_input(1, "late stale");
        let CoreInput::ConversationStreamUpdated { correlation, .. } = &mut late_stale else {
            unreachable!()
        };
        *correlation = TurnSubmissionCorrelation::new(6);
        sender
            .try_send(late_stale)
            .expect("late stale correlation should be ignored without affecting current admission");
        sender.try_send(progressive_input(1, " update")).unwrap();

        {
            let state = sender.shared.lock_state();
            assert_eq!(state.latest_progressive_generation, Some(7));
            assert_eq!(state.progressive_segment_count, 1);
            assert_eq!(state.pending_inputs.len(), 1);
        }
        let CoreInput::ConversationStreamUpdated {
            correlation,
            event: TurnStreamEvent::ProgressiveActivityObserved { batch },
        } = receiver.try_recv().unwrap()
        else {
            panic!("current progressive input should remain queued")
        };
        assert_eq!(correlation, TurnSubmissionCorrelation::new(7));
        assert_eq!(batch.first_sequence(), Some(0));
        assert_eq!(batch.last_sequence(), Some(1));
        assert_eq!(batch.source_observation_count(), 2);
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
    }

    fn assert_progressive_sequence(input: CoreInput, expected: u64) {
        let CoreInput::ConversationStreamUpdated {
            event: TurnStreamEvent::ProgressiveActivityObserved { batch },
            ..
        } = input
        else {
            panic!("expected a progressive Core input")
        };
        assert_eq!(batch.first_sequence(), Some(expected));
        assert_eq!(batch.last_sequence(), Some(expected));
    }
}
