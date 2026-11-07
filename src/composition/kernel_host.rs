//! 커널 호스트이다 (plan 18 K3-2). 커널(`NativeClientRuntime` + core)을 전용 OS 스레드에서 돌리고, 클라이언트는
//! `KernelClientHandle` 로 명령을 보내고, 조회하고, 이벤트를 받는다. 커널 상태는 이 스레드만 바꾼다. 비동기
//! 전송(Gateway)과는 채널로만 만나므로 core 와 application 은 tokio 를 모른다.

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::composition::native_client_runtime::NativeClientRuntime;
use crate::core::app::{AppCommand, AppEvent, AppSnapshot, CoreInput, ThreadActivitySummary};
use crate::core::native_client_port::{NativeClientDispatchOutcome, NativeClientEvent};

/// 커널 스레드가 커널 입력을 기다리는 최대 시간이다. 클라이언트 명령도 이 주기 안에 처리된다.
/// shortcut: 클라이언트 명령은 이 대기가 끝나야 처리되어 최대 50ms 늦다. 지연이 문제가 되면 클라이언트 입력함과
/// 커널 입력함을 하나의 깨우기로 합친다.
const HOST_WAIT: Duration = Duration::from_millis(50);
const HOST_MAX_INPUTS_PER_CYCLE: usize = 64;
/// 클라이언트마다 쌓아 둘 이벤트 수이다. 넘치면 그 클라이언트에 `Resync` 를 보내고 증분은 버린다.
const CLIENT_EVENT_CAPACITY: usize = 1024;

pub(crate) type KernelClientId = u64;

/// 클라이언트가 받는 이벤트이다. core 이벤트를 프로토콜 이벤트로 바꾸는 일은 inbound adapter 가 한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KernelHostEvent {
    Core(Box<AppEvent>),
    /// 이벤트가 넘쳐 일부를 버렸다. 클라이언트는 조회로 다시 맞춘다.
    Resync,
}

/// 조회 결과이다. 클라이언트가 지금 상태를 다시 그릴 때 쓴다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KernelHostView {
    pub(crate) snapshot: AppSnapshot,
    pub(crate) thread_activity: Vec<ThreadActivitySummary>,
}

enum KernelHostInput {
    Attach {
        reply: Sender<(KernelClientId, Receiver<KernelHostEvent>)>,
    },
    Command {
        command: AppCommand,
        reply: Sender<()>,
    },
    #[cfg(test)]
    Inputs(Vec<CoreInput>),
    Query {
        reply: Sender<KernelHostView>,
    },
    Detach {
        client_id: KernelClientId,
    },
    Shutdown,
}

pub(crate) struct KernelHost {
    inbox: Sender<KernelHostInput>,
    thread: Option<JoinHandle<()>>,
}

impl KernelHost {
    pub(crate) fn start(runtime: NativeClientRuntime) -> std::io::Result<Self> {
        let (inbox, inputs) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("akra-kernel-host".to_string())
            .spawn(move || KernelHostLoop::new(runtime, inputs).run())?;
        Ok(Self {
            inbox,
            thread: Some(thread),
        })
    }

    /// 클라이언트 하나를 붙인다. 커널 스레드가 끝났으면 `None` 이다.
    pub(crate) fn attach(&self) -> Option<KernelClientHandle> {
        let (reply, answer) = mpsc::channel();
        self.inbox.send(KernelHostInput::Attach { reply }).ok()?;
        let (client_id, events) = answer.recv().ok()?;
        Some(KernelClientHandle {
            client_id,
            inbox: self.inbox.clone(),
            events,
        })
    }
}

impl Drop for KernelHost {
    fn drop(&mut self) {
        let _ = self.inbox.send(KernelHostInput::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// 클라이언트 하나의 연결이다. 버리면 커널에서 떨어진다.
pub(crate) struct KernelClientHandle {
    client_id: KernelClientId,
    inbox: Sender<KernelHostInput>,
    events: Receiver<KernelHostEvent>,
}

impl KernelClientHandle {
    pub(crate) fn client_id(&self) -> KernelClientId {
        self.client_id
    }

    /// 명령을 커널에 넣는다. 커널이 처리한 뒤 돌아온다. 결과는 이벤트로 온다. 커널이 끝났으면 `false` 이다.
    pub(crate) fn send_command(&self, command: AppCommand) -> bool {
        let (reply, answer) = mpsc::channel();
        if self
            .inbox
            .send(KernelHostInput::Command { command, reply })
            .is_err()
        {
            return false;
        }
        answer.recv().is_ok()
    }

    pub(crate) fn query(&self) -> Option<KernelHostView> {
        let (reply, answer) = mpsc::channel();
        self.inbox.send(KernelHostInput::Query { reply }).ok()?;
        answer.recv().ok()
    }

    pub(crate) fn recv_event_timeout(&self, timeout: Duration) -> Option<KernelHostEvent> {
        self.events.recv_timeout(timeout).ok()
    }

    /// 시험이 커널 입력(스트림 이벤트, 효과 완료)을 직접 넣는다. 한 묶음은 호스트 주기 사이에 끊기지 않는다.
    #[cfg(test)]
    pub(crate) fn send_inputs_for_test(&self, inputs: Vec<CoreInput>) {
        let _ = self.inbox.send(KernelHostInput::Inputs(inputs));
    }
}

impl Drop for KernelClientHandle {
    fn drop(&mut self) {
        let _ = self.inbox.send(KernelHostInput::Detach {
            client_id: self.client_id,
        });
    }
}

struct ClientSink {
    events: SyncSender<KernelHostEvent>,
    resync_pending: bool,
}

struct KernelHostLoop {
    runtime: NativeClientRuntime,
    inputs: Receiver<KernelHostInput>,
    clients: BTreeMap<KernelClientId, ClientSink>,
    next_client_id: KernelClientId,
    thread_activity: Vec<ThreadActivitySummary>,
}

impl KernelHostLoop {
    fn new(runtime: NativeClientRuntime, inputs: Receiver<KernelHostInput>) -> Self {
        Self {
            runtime,
            inputs,
            clients: BTreeMap::new(),
            next_client_id: 1,
            thread_activity: Vec::new(),
        }
    }

    fn run(mut self) {
        loop {
            loop {
                match self.inputs.try_recv() {
                    Ok(KernelHostInput::Shutdown) | Err(mpsc::TryRecvError::Disconnected) => return,
                    Ok(input) => self.handle_input(input),
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            let outcomes =
                self.runtime
                    .run_host_cycle(Instant::now(), HOST_WAIT, HOST_MAX_INPUTS_PER_CYCLE);
            for outcome in outcomes {
                self.publish(outcome);
            }
        }
    }

    fn handle_input(&mut self, input: KernelHostInput) {
        match input {
            KernelHostInput::Attach { reply } => {
                let client_id = self.next_client_id;
                self.next_client_id += 1;
                let (events, receiver) = mpsc::sync_channel(CLIENT_EVENT_CAPACITY);
                self.clients.insert(
                    client_id,
                    ClientSink {
                        events,
                        resync_pending: false,
                    },
                );
                if reply.send((client_id, receiver)).is_err() {
                    self.clients.remove(&client_id);
                }
            }
            KernelHostInput::Command { command, reply } => {
                let outcome = self
                    .runtime
                    .dispatch_client_event(NativeClientEvent::core(CoreInput::Command(command)));
                self.publish(outcome);
                let _ = reply.send(());
            }
            #[cfg(test)]
            KernelHostInput::Inputs(inputs) => {
                for input in inputs {
                    let outcome = self
                        .runtime
                        .dispatch_client_event(NativeClientEvent::core(input));
                    self.publish(outcome);
                }
            }
            KernelHostInput::Query { reply } => {
                let _ = reply.send(KernelHostView {
                    snapshot: self.runtime.snapshot(),
                    thread_activity: self.thread_activity.clone(),
                });
            }
            KernelHostInput::Detach { client_id } => {
                self.clients.remove(&client_id);
            }
            KernelHostInput::Shutdown => {}
        }
    }

    /// core 이벤트를 모든 클라이언트에 나눈다. K3-3 에서 대상(thread, 클라이언트)별로 나눈다.
    fn publish(&mut self, outcome: NativeClientDispatchOutcome) {
        let events = match outcome {
            NativeClientDispatchOutcome::Core(core)
            | NativeClientDispatchOutcome::Combined { core, .. } => core.events,
            NativeClientDispatchOutcome::Parallel(_) => return,
        };
        for event in events {
            if let AppEvent::ThreadActivityChanged(activity) = &event {
                self.thread_activity = activity.clone();
            }
            self.clients.retain(|_, client| {
                deliver(client, KernelHostEvent::Core(Box::new(event.clone())))
            });
        }
    }
}

/// 이벤트 하나를 보낸다. 받는 쪽이 사라졌으면 `false` 이다.
fn deliver(client: &mut ClientSink, event: KernelHostEvent) -> bool {
    if client.resync_pending {
        match client.events.try_send(KernelHostEvent::Resync) {
            Ok(()) => client.resync_pending = false,
            Err(TrySendError::Full(_)) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
        }
    }
    match client.events.try_send(event) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            client.resync_pending = true;
            true
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::native_client_runtime::tests::{
        post_turn_correlation, post_turn_execution, recording_runtime_for_test,
    };
    use crate::core::app::{CoreEffectCompletion, TurnStreamEvent};
    use crate::domain::conversation_runtime_envelope::{
        ConversationRuntimeConfigurationRequest, ConversationRuntimeEnvelope,
    };
    use crate::domain::planning::TurnSnapshotCapture;
    use crate::domain::turn_terminal::{
        ConversationTurnApplicationDelivery, ConversationTurnTerminalReceipt,
    };

    const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

    fn wait_for(
        client: &KernelClientHandle,
        mut matches: impl FnMut(&KernelHostEvent) -> bool,
    ) -> bool {
        let deadline = Instant::now() + EVENT_TIMEOUT;
        while Instant::now() < deadline {
            if client
                .recv_event_timeout(Duration::from_millis(100))
                .is_some_and(|event| matches(&event))
            {
                return true;
            }
        }
        false
    }

    #[test]
    fn a_client_drives_the_kernel_through_the_host_thread() {
        let (mut runtime, turns) = recording_runtime_for_test();
        let turn = runtime.begin_test_turn_submission();
        let host = KernelHost::start(runtime).expect("kernel host thread should start");
        let client = host.attach().expect("a client should attach");
        assert_eq!(client.client_id(), 1);

        assert!(client.send_command(AppCommand::SetAutoFollowMaxTurns { value: 2 }));
        assert_eq!(
            client
                .query()
                .expect("the host should answer queries")
                .snapshot
                .conversation_runtime
                .auto_follow
                .max_auto_turns,
            2
        );

        let mut inputs = Vec::new();
        for event in [
            TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Hosted loop".to_string(),
                cwd: "/workspace".to_string(),
                runtime_envelope: Box::<ConversationRuntimeEnvelope>::default(),
            },
            TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::<ConversationRuntimeConfigurationRequest>::default(),
            },
            TurnStreamEvent::TurnTerminal {
                receipt: ConversationTurnTerminalReceipt::completed(
                    "thread-1",
                    "turn-1",
                    Vec::new(),
                )
                .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed),
                execution_snapshot_capture: Some(TurnSnapshotCapture::capture_failed(
                    "/workspace",
                    "test capture skipped".to_string(),
                )),
            },
        ] {
            inputs.push(CoreInput::ConversationStreamUpdated {
                correlation: turn,
                event,
            });
        }
        // The host starts the evaluation while it handles the terminal. The test completion is in
        // the same batch, so it lands before the real worker's completion can.
        let correlation = post_turn_correlation(1);
        inputs.push(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: correlation.clone(),
                execution: Box::new(post_turn_execution(&correlation)),
            },
        ));
        client.send_inputs_for_test(inputs);
        assert!(
            wait_for(&client, |event| matches!(
                event,
                KernelHostEvent::Core(core) if matches!(**core, AppEvent::PostTurnEvaluationStarted(_))
            )),
            "the host should start the evaluation by itself"
        );
        assert!(
            wait_for(&client, |event| matches!(
                event,
                KernelHostEvent::Core(core) if matches!(
                    **core,
                    AppEvent::AutoFollowTurnSubmissionResolved { admitted: true, .. }
                )
            )),
            "the client should learn that the follow-up turn was submitted"
        );
        let deadline = Instant::now() + EVENT_TIMEOUT;
        while turns.lock().expect("recorded turns should lock").is_empty() {
            assert!(Instant::now() < deadline, "the follow-up turn should run");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            turns.lock().expect("recorded turns should lock").as_slice(),
            [("thread-1".to_string(), "continue".to_string())]
        );
    }

    #[test]
    fn a_slow_client_gets_resync_instead_of_blocking_the_kernel() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut client = ClientSink {
            events: sender,
            resync_pending: false,
        };
        let event = KernelHostEvent::Core(Box::new(AppEvent::ThreadActivityChanged(Vec::new())));
        assert!(deliver(&mut client, event.clone()));
        assert!(
            deliver(&mut client, event.clone()),
            "a full queue drops, not blocks"
        );
        assert!(client.resync_pending);
        assert_eq!(receiver.try_recv(), Ok(event.clone()));
        assert!(deliver(&mut client, event.clone()));
        assert_eq!(
            receiver.try_recv(),
            Ok(KernelHostEvent::Resync),
            "the client learns it missed events before new ones"
        );
        drop(receiver);
        assert!(!deliver(&mut client, event), "a gone client is dropped");
    }
}
