use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};

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

        result_receiver
            .recv()
            .map_err(|_| anyhow!("active turn steering connection closed without a result"))?
            .map_err(anyhow::Error::msg)
    }

    pub(super) fn complete(
        &self,
        binding_id: u64,
        request_id: u64,
        result: std::result::Result<ConversationTurnSteerReceipt, String>,
    ) -> Result<()> {
        let result_sender = {
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
            state.pending = None;
            result_sender
        };
        result_sender
            .send(result)
            .map_err(|_| anyhow!("turn steering caller is no longer waiting"))
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
