use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};

use crate::domain::conversation::{ConversationTurnSteerReceipt, ConversationTurnSteerRequest};

pub(super) struct AppServerTurnSteerCommand {
    binding_id: u64,
    request_id: u64,
    pub(super) request: ConversationTurnSteerRequest,
}

impl AppServerTurnSteerCommand {
    pub(super) fn binding_id(&self) -> u64 {
        self.binding_id
    }

    pub(super) fn request_id(&self) -> u64 {
        self.request_id
    }
}

pub(super) struct AppServerTurnSteerBinding {
    binding_id: u64,
    receiver: Receiver<AppServerTurnSteerCommand>,
}

impl AppServerTurnSteerBinding {
    pub(super) fn binding_id(&self) -> u64 {
        self.binding_id
    }

    pub(super) fn try_receive(&self) -> Result<Option<AppServerTurnSteerCommand>> {
        match self.receiver.try_recv() {
            Ok(command) => Ok(Some(command)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                Err(anyhow!("active turn steering command channel disconnected"))
            }
        }
    }
}

struct ActiveTurnBinding {
    binding_id: u64,
    thread_id: String,
    turn_id: String,
    command_sender: SyncSender<AppServerTurnSteerCommand>,
}

struct PendingTurnSteer {
    binding_id: u64,
    request_id: u64,
    result_sender: SyncSender<std::result::Result<ConversationTurnSteerReceipt, String>>,
    allow_detached: bool,
}

#[derive(Default)]
struct AppServerTurnSteerBrokerState {
    next_binding_id: u64,
    next_request_id: u64,
    active: Option<ActiveTurnBinding>,
    pending: Option<PendingTurnSteer>,
}

#[derive(Default)]
pub(super) struct AppServerTurnSteerBroker {
    state: Mutex<AppServerTurnSteerBrokerState>,
}

impl AppServerTurnSteerBroker {
    pub(super) fn active_turn_id(&self, thread_id: &str) -> Result<Option<String>> {
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow!("turn steering broker mutex was poisoned"))?;
        Ok(state
            .active
            .as_ref()
            .filter(|active| active.thread_id == thread_id)
            .map(|active| active.turn_id.clone()))
    }

    pub(super) fn bind(&self, thread_id: &str, turn_id: &str) -> Result<AppServerTurnSteerBinding> {
        if thread_id.is_empty() || turn_id.is_empty() {
            return Err(anyhow!(
                "active turn steering requires nonempty thread and turn identifiers"
            ));
        }

        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("turn steering broker mutex was poisoned"))?;
        if state.active.is_some() || state.pending.is_some() {
            return Err(anyhow!("another active turn already owns steering"));
        }

        state.next_binding_id = state.next_binding_id.saturating_add(1);
        let binding_id = state.next_binding_id;
        let (command_sender, receiver) = mpsc::sync_channel(1);
        state.active = Some(ActiveTurnBinding {
            binding_id,
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            command_sender,
        });
        Ok(AppServerTurnSteerBinding {
            binding_id,
            receiver,
        })
    }

    pub(super) fn submit(
        &self,
        request: ConversationTurnSteerRequest,
    ) -> Result<ConversationTurnSteerReceipt> {
        self.submit_with_deadline(request, None)
    }

    pub(super) fn submit_bounded(
        &self,
        request: ConversationTurnSteerRequest,
        timeout: std::time::Duration,
    ) -> Result<ConversationTurnSteerReceipt> {
        self.submit_with_deadline(request, Some(timeout))
    }

    fn submit_with_deadline(
        &self,
        request: ConversationTurnSteerRequest,
        timeout: Option<std::time::Duration>,
    ) -> Result<ConversationTurnSteerReceipt> {
        if request.thread_id.is_empty()
            || request.expected_turn_id.is_empty()
            || request.prompt.trim().is_empty()
        {
            return Err(anyhow!(
                "turn steering requires nonempty thread, turn, and prompt values"
            ));
        }

        let (result_sender, result_receiver) = mpsc::sync_channel(1);
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow!("turn steering broker mutex was poisoned"))?;
            let active = state
                .active
                .as_ref()
                .ok_or_else(|| anyhow!("no steerable active turn is connected"))?;
            if active.thread_id != request.thread_id || active.turn_id != request.expected_turn_id {
                return Err(anyhow!(
                    "turn steering request did not match the active thread and turn"
                ));
            }
            if state.pending.is_some() {
                return Err(anyhow!("another turn steering request is already pending"));
            }

            let binding_id = active.binding_id;
            let command_sender = active.command_sender.clone();
            state.next_request_id = state.next_request_id.saturating_add(1);
            let request_id = state.next_request_id;
            state.pending = Some(PendingTurnSteer {
                binding_id,
                request_id,
                result_sender,
                allow_detached: timeout.is_some(),
            });
            let command = AppServerTurnSteerCommand {
                binding_id,
                request_id,
                request,
            };
            if let Err(error) = command_sender.try_send(command) {
                state.pending = None;
                return Err(match error {
                    TrySendError::Full(_) => {
                        anyhow!("another turn steering request is already queued")
                    }
                    TrySendError::Disconnected(_) => {
                        anyhow!("active turn steering connection is no longer available")
                    }
                });
            }
        }

        let response = if let Some(timeout) = timeout {
            result_receiver
                .recv_timeout(timeout)
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => {
                        anyhow!("worker steering delivery is unknown after the response deadline")
                    }
                    mpsc::RecvTimeoutError::Disconnected => {
                        anyhow!("active turn steering connection closed without a result")
                    }
                })?
        } else {
            result_receiver
                .recv()
                .map_err(|_| anyhow!("active turn steering connection closed without a result"))?
        };
        response.map_err(anyhow::Error::msg)
    }

    pub(super) fn complete(
        &self,
        binding_id: u64,
        request_id: u64,
        result: std::result::Result<ConversationTurnSteerReceipt, String>,
    ) -> Result<()> {
        let (result_sender, allow_detached) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow!("turn steering broker mutex was poisoned"))?;
            let pending = state
                .pending
                .as_ref()
                .filter(|pending| {
                    pending.binding_id == binding_id && pending.request_id == request_id
                })
                .ok_or_else(|| anyhow!("turn steering request is no longer pending"))?;
            let result_sender = pending.result_sender.clone();
            let allow_detached = pending.allow_detached;
            state.pending = None;
            (result_sender, allow_detached)
        };
        // worker의 bounded 대기 종료는 모델 턴의 실패가 아니다. 늦은 응답만 소모한다.
        match result_sender.send(result) {
            Ok(()) => Ok(()),
            Err(_) if allow_detached => Ok(()),
            Err(_) => Err(anyhow!("turn steering caller is no longer waiting")),
        }
    }

    pub(super) fn unbind(&self, binding_id: u64, reason: &str) {
        let pending_sender = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state
                .active
                .as_ref()
                .is_none_or(|active| active.binding_id != binding_id)
            {
                return;
            }
            state.active = None;
            state.pending.take().and_then(|pending| {
                (pending.binding_id == binding_id).then_some(pending.result_sender)
            })
        };
        if let Some(sender) = pending_sender {
            let _ = sender.send(Err(reason.to_string()));
        }
    }

    #[cfg(test)]
    pub(super) fn pending_count(&self) -> usize {
        self.state
            .lock()
            .map_or(0, |state| usize::from(state.pending.is_some()))
    }
}

/// 병렬 stream마다 독립 broker를 보관한다. main의 단일 실행/대기 계약은 그대로 둔다.
#[derive(Default)]
pub(super) struct AppServerParallelTurnSteerRegistry {
    brokers: Mutex<BTreeMap<String, Arc<AppServerTurnSteerBroker>>>,
}

impl AppServerParallelTurnSteerRegistry {
    pub(super) fn bind(
        self: &Arc<Self>,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<AppServerParallelTurnSteerRegistration> {
        let mut brokers = self
            .brokers
            .lock()
            .map_err(|_| anyhow!("parallel turn steering registry mutex was poisoned"))?;
        if brokers.contains_key(thread_id) {
            return Err(anyhow!("another stream already owns this parallel thread"));
        }
        let broker = Arc::new(AppServerTurnSteerBroker::default());
        let binding = broker.bind(thread_id, turn_id)?;
        let binding_id = binding.binding_id();
        brokers.insert(thread_id.to_owned(), broker.clone());
        Ok(AppServerParallelTurnSteerRegistration {
            registry: self.clone(),
            thread_id: thread_id.to_owned(),
            broker,
            binding: Some(binding),
            binding_id,
        })
    }

    pub(super) fn broker(&self, thread_id: &str) -> Result<Option<Arc<AppServerTurnSteerBroker>>> {
        Ok(self
            .brokers
            .lock()
            .map_err(|_| anyhow!("parallel turn steering registry mutex was poisoned"))?
            .get(thread_id)
            .cloned())
    }
}

/// 반환, 실패, panic에도 과거 등록을 남기지 않는다. 다른 세대의 소유권은 지우지 않는다.
pub(super) struct AppServerParallelTurnSteerRegistration {
    registry: Arc<AppServerParallelTurnSteerRegistry>,
    thread_id: String,
    pub(super) broker: Arc<AppServerTurnSteerBroker>,
    binding: Option<AppServerTurnSteerBinding>,
    binding_id: u64,
}

impl AppServerParallelTurnSteerRegistration {
    pub(super) fn take_binding(&mut self) -> Option<AppServerTurnSteerBinding> {
        self.binding.take()
    }
}

impl Drop for AppServerParallelTurnSteerRegistration {
    fn drop(&mut self) {
        self.broker
            .unbind(self.binding_id, "parallel turn stream ended");
        if let Ok(mut brokers) = self.registry.brokers.lock()
            && brokers
                .get(&self.thread_id)
                .is_some_and(|current| Arc::ptr_eq(current, &self.broker))
        {
            brokers.remove(&self.thread_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;

    fn request(thread_id: &str, turn_id: &str, prompt: &str) -> ConversationTurnSteerRequest {
        ConversationTurnSteerRequest {
            thread_id: thread_id.to_string(),
            expected_turn_id: turn_id.to_string(),
            prompt: prompt.to_string(),
        }
    }

    #[test]
    fn bounded_worker_steer_keeps_pending_fence_and_accepts_a_late_reply_without_ending_turn() {
        let broker = AppServerTurnSteerBroker::default();
        let binding = broker.bind("worker", "turn").unwrap();
        assert!(
            broker
                .submit_bounded(
                    request("worker", "turn", "instruction"),
                    std::time::Duration::from_millis(5)
                )
                .unwrap_err()
                .to_string()
                .contains("unknown")
        );
        assert_eq!(broker.pending_count(), 1);
        assert!(
            broker
                .submit_bounded(
                    request("worker", "turn", "another instruction"),
                    std::time::Duration::from_millis(5)
                )
                .unwrap_err()
                .to_string()
                .contains("already pending")
        );
        let command = binding.try_receive().unwrap().unwrap();
        broker
            .complete(
                command.binding_id(),
                command.request_id(),
                Ok(ConversationTurnSteerReceipt {
                    turn_id: "turn".to_owned(),
                }),
            )
            .unwrap();
        assert_eq!(broker.pending_count(), 0);
        assert_eq!(
            broker.active_turn_id("worker").unwrap().as_deref(),
            Some("turn")
        );
        assert!(
            broker
                .complete(
                    command.binding_id(),
                    command.request_id(),
                    Ok(ConversationTurnSteerReceipt {
                        turn_id: "turn".to_owned()
                    })
                )
                .is_err()
        );
        broker.unbind(binding.binding_id(), "done");
    }

    #[test]
    fn parallel_steering_routes_exact_turns_without_replacing_other_workers_or_main() {
        let registry = Arc::new(AppServerParallelTurnSteerRegistry::default());
        let main = AppServerTurnSteerBroker::default();
        let main_binding = main.bind("main-thread", "main-turn").unwrap();
        let mut first = registry.bind("worker-a", "turn-a").unwrap();
        let mut second = registry.bind("worker-b", "turn-b").unwrap();
        let first_binding = first.take_binding().unwrap();
        let second_binding = second.take_binding().unwrap();
        assert!(registry.bind("worker-a", "replacement-turn").is_err());
        assert!(
            first
                .broker
                .submit(request("worker-a", "old-turn", "stale"))
                .is_err()
        );
        let first_broker = registry.broker("worker-a").unwrap().unwrap();
        let second_broker = registry.broker("worker-b").unwrap().unwrap();
        let first_call =
            thread::spawn(move || first_broker.submit(request("worker-a", "turn-a", "a")));
        let second_call =
            thread::spawn(move || second_broker.submit(request("worker-b", "turn-b", "b")));
        for (binding, broker, expected) in [
            (&first_binding, &first.broker, "turn-a"),
            (&second_binding, &second.broker, "turn-b"),
        ] {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let command = loop {
                if let Some(command) = binding.try_receive().unwrap() {
                    break command;
                }
                assert!(std::time::Instant::now() < deadline, "steer was not routed");
                thread::yield_now();
            };
            assert_eq!(command.request.expected_turn_id, expected);
            broker
                .complete(
                    command.binding_id(),
                    command.request_id(),
                    Ok(ConversationTurnSteerReceipt {
                        turn_id: expected.to_owned(),
                    }),
                )
                .unwrap();
        }
        assert_eq!(first_call.join().unwrap().unwrap().turn_id, "turn-a");
        assert_eq!(second_call.join().unwrap().unwrap().turn_id, "turn-b");
        drop(first);
        assert!(registry.broker("worker-a").unwrap().is_none());
        assert_eq!(
            second.broker.active_turn_id("worker-b").unwrap().as_deref(),
            Some("turn-b")
        );
        assert_eq!(
            main.active_turn_id("main-thread").unwrap().as_deref(),
            Some("main-turn")
        );
        main.unbind(main_binding.binding_id(), "done");
    }

    #[test]
    fn parallel_steering_stream_end_rejects_pending_delivery_and_allows_new_generation() {
        let registry = Arc::new(AppServerParallelTurnSteerRegistry::default());
        let registration = registry.bind("worker-a", "turn-a").unwrap();
        let broker = registration.broker.clone();
        let sender = broker.clone();
        let call =
            thread::spawn(move || sender.submit(request("worker-a", "turn-a", "instruction")));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while broker.pending_count() == 0 {
            assert!(std::time::Instant::now() < deadline, "steer was not queued");
            thread::yield_now();
        }
        drop(registration);
        assert!(call.join().unwrap().is_err());
        assert!(broker.active_turn_id("worker-a").unwrap().is_none());
        let next = registry.bind("worker-a", "turn-b").unwrap();
        assert!(
            next.broker
                .submit(request("worker-a", "turn-a", "late old request"))
                .is_err()
        );
        assert_eq!(
            next.broker.active_turn_id("worker-a").unwrap().as_deref(),
            Some("turn-b")
        );
    }

    #[test]
    fn broker_rejects_stale_identity_and_duplicate_pending_request() {
        let broker = Arc::new(AppServerTurnSteerBroker::default());
        let binding = broker.bind("thread-1", "turn-1").expect("bind turn");

        assert!(
            broker
                .submit(request("thread-2", "turn-1", "wrong thread"))
                .expect_err("thread mismatch must fail")
                .to_string()
                .contains("did not match")
        );
        assert!(
            broker
                .submit(request("thread-1", "turn-0", "stale turn"))
                .expect_err("turn mismatch must fail")
                .to_string()
                .contains("did not match")
        );

        let caller_broker = broker.clone();
        let caller =
            thread::spawn(move || caller_broker.submit(request("thread-1", "turn-1", "first")));
        while broker.pending_count() == 0 {
            thread::yield_now();
        }
        assert!(
            broker
                .submit(request("thread-1", "turn-1", "duplicate"))
                .expect_err("duplicate pending request must fail")
                .to_string()
                .contains("already pending")
        );

        broker.unbind(binding.binding_id(), "turn ended");
        assert!(
            caller
                .join()
                .expect("caller should finish")
                .expect_err("unbind must reject pending caller")
                .to_string()
                .contains("turn ended")
        );
    }

    #[test]
    fn broker_completes_the_exact_bound_request_once() {
        let broker = Arc::new(AppServerTurnSteerBroker::default());
        let binding = broker.bind("thread-1", "turn-1").expect("bind turn");
        let caller_broker = broker.clone();
        let caller = thread::spawn(move || {
            caller_broker.submit(request("thread-1", "turn-1", "correct course"))
        });

        let command = loop {
            if let Some(command) = binding.try_receive().expect("receive command") {
                break command;
            }
            thread::yield_now();
        };
        broker
            .complete(
                command.binding_id(),
                command.request_id(),
                Ok(ConversationTurnSteerReceipt {
                    turn_id: "turn-1".to_string(),
                }),
            )
            .expect("complete request");

        assert_eq!(
            caller
                .join()
                .expect("caller should finish")
                .expect("caller should receive receipt"),
            ConversationTurnSteerReceipt {
                turn_id: "turn-1".to_string()
            }
        );
        assert!(
            broker
                .complete(
                    command.binding_id(),
                    command.request_id(),
                    Ok(ConversationTurnSteerReceipt {
                        turn_id: "turn-1".to_string(),
                    }),
                )
                .is_err()
        );
        broker.unbind(binding.binding_id(), "turn ended");
    }
}
