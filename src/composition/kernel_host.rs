//! 커널 호스트이다 (plan 18 K3-2). 커널(`NativeClientRuntime` + core)을 전용 OS 스레드에서 돌리고, 클라이언트는
//! `KernelClientHandle` 로 명령을 보내고, 조회하고, 이벤트를 받는다. 커널 상태는 이 스레드만 바꾼다. 비동기
//! 전송(Gateway)과는 채널로만 만나므로 core 와 application 은 tokio 를 모른다.

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::composition::native_client_runtime::NativeClientRuntime;
use crate::core::app::{AppCommand, AppEvent, AppSnapshot, CoreInput, ThreadActivitySummary};
use crate::core::native_client_port::{
    NativeClientDispatchOutcome, NativeClientEvent, ParallelModeControlPlanePresentationEvent,
};

/// 커널 스레드가 커널 입력을 기다리는 최대 시간이다. 클라이언트 명령도 이 주기 안에 처리된다.
/// shortcut: 클라이언트 명령은 이 대기가 끝나야 처리되어 최대 50ms 늦다. 지연이 문제가 되면 클라이언트 입력함과
/// 커널 입력함을 하나의 깨우기로 합친다.
const HOST_WAIT: Duration = Duration::from_millis(50);
const HOST_MAX_INPUTS_PER_CYCLE: usize = 64;
/// 클라이언트마다 쌓아 둘 이벤트 수이다. 넘치면 그 클라이언트에 `Resync` 를 보내고 증분은 버린다.
const CLIENT_EVENT_CAPACITY: usize = 1024;
/// 붙기 요청에 커널 스레드가 답할 때까지 기다리는 최대 시간이다. 보통 한 주기(`HOST_WAIT`) 안에 답한다.
const ATTACH_REPLY_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) type KernelClientId = u64;

/// 커널 호출이 답을 받지 못한 이유이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KernelCallError {
    /// 커널 스레드가 끝났다.
    Stopped,
    /// 커널 스레드가 정한 시간 안에 답하지 않았다. 요청은 그 뒤에 처리될 수 있다.
    TimedOut,
}

impl std::fmt::Display for KernelCallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped => formatter.write_str("the kernel stopped"),
            Self::TimedOut => formatter.write_str("the kernel did not answer in time"),
        }
    }
}

impl std::error::Error for KernelCallError {}

fn wait_reply<T>(answer: &Receiver<T>, timeout: Duration) -> Result<T, KernelCallError> {
    answer.recv_timeout(timeout).map_err(|error| match error {
        mpsc::RecvTimeoutError::Timeout => KernelCallError::TimedOut,
        mpsc::RecvTimeoutError::Disconnected => KernelCallError::Stopped,
    })
}

/// 클라이언트가 받는 이벤트이다. core 이벤트를 프로토콜 이벤트로 바꾸는 일은 inbound adapter 가 한다.
#[derive(Debug, Clone)]
pub(crate) enum KernelHostEvent {
    Core(Box<AppEvent>),
    /// 병렬 제어부의 이벤트이다 (병렬 작업 진행, 감독 스냅샷). 프로토콜 이벤트로 바꾸는 일은 K3-6 에서 한다.
    // shortcut: Gateway 는 아직 이 이벤트를 보내지 않는다. K3-6 에서 프로토콜 이벤트로 바꾸면 이 allow 를 지운다.
    Parallel(
        #[cfg_attr(not(test), allow(dead_code))] Box<ParallelModeControlPlanePresentationEvent>,
    ),
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
        reply: Sender<Vec<AppEvent>>,
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

/// 커널이 한가해진 시각이다 (plan 18 K3-5). 클라이언트도 남은 일도 없으면 정해지고, 클라이언트가 붙거나 일이
/// 생기면 지워진다. 커널 스레드가 쓰고, 커널 목록이 커널을 내릴지 정할 때 읽는다.
#[derive(Clone, Default)]
struct KernelIdleClock(Arc<Mutex<Option<Instant>>>);

impl KernelIdleClock {
    fn record(&self, busy: bool, now: Instant) {
        let mut idle_since = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if busy {
            *idle_since = None;
        } else if idle_since.is_none() {
            *idle_since = Some(now);
        }
    }

    fn idle_since(&self) -> Option<Instant> {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(crate) struct KernelHost {
    inbox: Sender<KernelHostInput>,
    thread: Option<JoinHandle<()>>,
    idle: KernelIdleClock,
}

impl KernelHost {
    pub(crate) fn start(runtime: NativeClientRuntime) -> std::io::Result<Self> {
        let (inbox, inputs) = mpsc::channel();
        let idle = KernelIdleClock::default();
        let host_idle = idle.clone();
        let thread = std::thread::Builder::new()
            .name("akra-kernel-host".to_string())
            .spawn(move || KernelHostLoop::new(runtime, inputs, host_idle).run())?;
        Ok(Self {
            inbox,
            thread: Some(thread),
            idle,
        })
    }

    /// 클라이언트도 남은 일도 없이 지난 시간이다. 클라이언트가 있거나 일이 남았으면 `None` 이다.
    pub(crate) fn idle_for(&self, now: Instant) -> Option<Duration> {
        self.idle
            .idle_since()
            .map(|since| now.saturating_duration_since(since))
    }

    /// 클라이언트 하나를 붙인다. 커널 스레드가 끝났으면 `Stopped`, 답이 늦으면 `TimedOut` 이다. 늦은 붙기는 커널이
    /// 답을 보내지 못하므로 스스로 지운다.
    pub(crate) fn attach(&self) -> Result<KernelClientHandle, KernelCallError> {
        let (reply, answer) = mpsc::channel();
        self.inbox
            .send(KernelHostInput::Attach { reply })
            .map_err(|_| KernelCallError::Stopped)?;
        let (client_id, events) = wait_reply(&answer, ATTACH_REPLY_TIMEOUT)?;
        Ok(KernelClientHandle {
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

/// 명령과 조회만 보내는 쪽이다. 이벤트를 받는 스레드가 핸들을 쥐는 동안 다른 작업이 명령을 보낼 때 쓴다.
/// 핸들이 떨어진 뒤에도 보낼 수 있지만, 그 클라이언트는 이벤트를 받지 않는다.
#[derive(Clone)]
pub(crate) struct KernelCommandSender {
    client_id: KernelClientId,
    inbox: Sender<KernelHostInput>,
}

impl KernelCommandSender {
    /// 명령을 커널에 넣는다. 커널이 처리한 뒤 그 처리에서 나온 core 이벤트를 돌려준다. 요청한 클라이언트는 이것으로
    /// 거절 이유를 안다. 같은 이벤트는 모든 클라이언트에도 간다. `TimedOut` 이어도 커널은 명령을 나중에 처리할 수 있다.
    pub(crate) fn send_command(
        &self,
        command: AppCommand,
        timeout: Duration,
    ) -> Result<Vec<AppEvent>, KernelCallError> {
        let (reply, answer) = mpsc::channel();
        self.inbox
            .send(KernelHostInput::Command { command, reply })
            .map_err(|_| KernelCallError::Stopped)?;
        wait_reply(&answer, timeout)
    }

    pub(crate) fn query(&self, timeout: Duration) -> Result<KernelHostView, KernelCallError> {
        let (reply, answer) = mpsc::channel();
        self.inbox
            .send(KernelHostInput::Query { reply })
            .map_err(|_| KernelCallError::Stopped)?;
        wait_reply(&answer, timeout)
    }

    /// 이 클라이언트를 커널에서 뗀다. 커널은 이벤트 채널을 닫고, 이벤트를 기다리던 쪽은 바로 끝을 본다.
    pub(crate) fn detach(&self) {
        let _ = self.inbox.send(KernelHostInput::Detach {
            client_id: self.client_id,
        });
    }
}

impl KernelClientHandle {
    pub(crate) fn client_id(&self) -> KernelClientId {
        self.client_id
    }

    pub(crate) fn commands(&self) -> KernelCommandSender {
        KernelCommandSender {
            client_id: self.client_id,
            inbox: self.inbox.clone(),
        }
    }

    #[cfg(test)]
    pub(crate) fn send_command(&self, command: AppCommand) -> Option<Vec<AppEvent>> {
        self.commands()
            .send_command(command, TEST_REPLY_TIMEOUT)
            .ok()
    }

    #[cfg(test)]
    pub(crate) fn query(&self) -> Option<KernelHostView> {
        self.commands().query(TEST_REPLY_TIMEOUT).ok()
    }

    #[cfg(test)]
    pub(crate) fn recv_event_timeout(&self, timeout: Duration) -> Option<KernelHostEvent> {
        self.recv_event(timeout).ok()
    }

    /// 이벤트를 기다린다. `Disconnected` 는 커널이 끝난 경우이다.
    pub(crate) fn recv_event(
        &self,
        timeout: Duration,
    ) -> Result<KernelHostEvent, mpsc::RecvTimeoutError> {
        self.events.recv_timeout(timeout)
    }

    /// 시험이 커널 입력(스트림 이벤트, 효과 완료)을 직접 넣는다. 한 묶음은 호스트 주기 사이에 끊기지 않는다.
    #[cfg(test)]
    pub(crate) fn send_inputs_for_test(&self, inputs: Vec<CoreInput>) {
        let _ = self.inbox.send(KernelHostInput::Inputs(inputs));
    }
}

#[cfg(test)]
const TEST_REPLY_TIMEOUT: Duration = Duration::from_secs(10);

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
    idle: KernelIdleClock,
}

impl KernelHostLoop {
    fn new(
        runtime: NativeClientRuntime,
        inputs: Receiver<KernelHostInput>,
        idle: KernelIdleClock,
    ) -> Self {
        Self {
            runtime,
            inputs,
            clients: BTreeMap::new(),
            next_client_id: 1,
            thread_activity: Vec::new(),
            idle,
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
            self.flush_pending_resyncs();
            self.record_idle(Instant::now());
        }
    }

    /// 클라이언트도 남은 일도 없으면 한가한 시각을 정한다. 하나라도 있으면 지운다.
    fn record_idle(&self, now: Instant) {
        let busy = !self.clients.is_empty() || self.runtime.has_remaining_work();
        self.idle.record(busy, now);
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
                // 답하기 전에 지운다. 붙기가 끝난 뒤 커널 목록이 이 커널을 한가하다고 보지 않는다.
                self.idle.record(true, Instant::now());
                if reply.send((client_id, receiver)).is_err() {
                    self.clients.remove(&client_id);
                }
            }
            KernelHostInput::Command { command, reply } => {
                let outcome = self
                    .runtime
                    .dispatch_client_event(NativeClientEvent::core(CoreInput::Command(command)));
                let events = core_events(&outcome).to_vec();
                self.publish(outcome);
                let _ = reply.send(events);
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

    /// core 이벤트와 병렬 제어부 이벤트를 모든 클라이언트에 나눈다. 원격 클라이언트는 보는 자리가 없고 thread 를
    /// 지정하므로(§3.2) 이벤트는 thread id 를 담고 모두에게 간다. 클라이언트별 대상은 K6 의 클라이언트별 보기와 함께 정한다.
    fn publish(&mut self, outcome: NativeClientDispatchOutcome) {
        let (core_events, parallel_events) = match outcome {
            NativeClientDispatchOutcome::Core(core) => (core.events, Vec::new()),
            NativeClientDispatchOutcome::Parallel(parallel) => {
                (Vec::new(), parallel.presentation_events)
            }
            NativeClientDispatchOutcome::Combined { core, parallel } => {
                (core.events, parallel.presentation_events)
            }
        };
        for event in core_events {
            if let AppEvent::ThreadActivityChanged(activity) = &event {
                self.thread_activity = activity.clone();
            }
            self.broadcast(KernelHostEvent::Core(Box::new(event)));
        }
        for event in parallel_events {
            self.broadcast(KernelHostEvent::Parallel(Box::new(event)));
        }
    }

    fn broadcast(&mut self, event: KernelHostEvent) {
        self.clients
            .retain(|_, client| deliver(client, event.clone()));
    }

    /// 이벤트를 버린 클라이언트에 `Resync` 를 보낸다. 새 이벤트가 없어도 다음 주기에 알린다.
    fn flush_pending_resyncs(&mut self) {
        self.clients.retain(|_, client| flush_resync(client));
    }
}

/// 이벤트 하나를 보낸다. 받는 쪽이 사라졌으면 `false` 이다.
/// dispatch 결과의 core 이벤트이다. 병렬 제어부 이벤트는 명령 응답에 넣지 않는다.
fn core_events(outcome: &NativeClientDispatchOutcome) -> &[AppEvent] {
    match outcome {
        NativeClientDispatchOutcome::Core(core)
        | NativeClientDispatchOutcome::Combined { core, .. } => &core.events,
        NativeClientDispatchOutcome::Parallel(_) => &[],
    }
}

fn deliver(client: &mut ClientSink, event: KernelHostEvent) -> bool {
    if !flush_resync(client) {
        return false;
    }
    if client.resync_pending {
        return true;
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

/// 밀린 `Resync` 를 보낸다. 아직 자리가 없으면 다음에 다시 보낸다. 받는 쪽이 사라졌으면 `false` 이다.
fn flush_resync(client: &mut ClientSink) -> bool {
    if !client.resync_pending {
        return true;
    }
    match client.events.try_send(KernelHostEvent::Resync) {
        Ok(()) => {
            client.resync_pending = false;
            true
        }
        Err(TrySendError::Full(_)) => true,
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
    use crate::core::native_client_port::NativeParallelDispatchOutcome;
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

        assert!(
            client
                .send_command(AppCommand::SetAutoFollowMaxTurns { value: 2 })
                .is_some()
        );
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
    fn a_client_submits_a_prompt_to_a_thread_the_tui_does_not_show() {
        // plan 18 K3-3b: core prepares the prompt and submits the turn to the addressed thread.
        let (runtime, turns) = recording_runtime_for_test();
        let host = KernelHost::start(runtime).expect("kernel host thread should start");
        let client = host.attach().expect("a client should attach");
        // 파일 아래 경로라서 실제 worker(준비, 병렬 pool 잠금)는 아무것도 만들지 못한다. 시험은 준비 결과를 직접
        // 넣는다.
        let workspace = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml/workspace");

        assert!(
            client
                .send_command(AppCommand::LoadSessionCatalog(
                    crate::core::app::SessionCatalogLoadIntent::ensure_loaded(10, workspace),
                ))
                .is_some()
        );
        assert!(
            wait_for(&client, |event| matches!(
                event,
                KernelHostEvent::Core(core) if matches!(
                    &**core,
                    AppEvent::SessionCatalogChanged(crate::core::app::SessionCatalogSnapshot::Ready(_))
                )
            )),
            "the kernel should list thread-2"
        );

        // 한 묶음이라 실제 준비 worker 의 결과보다 먼저 들어간다.
        client.send_inputs_for_test(vec![
            CoreInput::Command(AppCommand::SubmitThreadPrompt(Box::new(
                crate::core::app::ThreadPromptSubmission {
                    workspace_directory: workspace.to_string(),
                    thread_id: "thread-2".to_string(),
                    prompt: "fix the test".to_string(),
                },
            ))),
            CoreInput::EffectCompleted(CoreEffectCompletion::ManualPromptPrepared(Box::new(
                crate::domain::planning::ManualPromptOutcome::PromptReady {
                    correlation: crate::domain::planning::ManualPromptCorrelation {
                        request_id: 1,
                        generation: 1,
                        workspace_directory: workspace.to_string(),
                    },
                    transcript_text: "fix the test".to_string(),
                    runtime_projection: Box::new(
                        crate::application::service::planning::PlanningRuntimeProjection::invalid(
                            "test",
                        ),
                    ),
                    intake: Box::new(
                        crate::domain::planning::ManualPromptIntakeOutcome::TaskCommitted {
                            committed_task_id: "task-1".to_string(),
                            committed_planning_revision: 1,
                            handoff: crate::domain::planning::ManualPromptMainSessionHandoff {
                                prompt: "planned prompt".to_string(),
                                transcript_text: "fix the test".to_string(),
                                task: None,
                            },
                        },
                    ),
                },
            ))),
        ]);
        assert!(
            wait_for(&client, |event| matches!(
                event,
                KernelHostEvent::Core(core) if matches!(
                    &**core,
                    AppEvent::ThreadPromptSubmitted {
                        thread_id,
                        outcome: crate::core::app::ThreadPromptOutcome::Submitted {
                            admission: crate::core::app::TurnSubmissionAdmission::Accepted { .. },
                            ..
                        },
                    } if thread_id == "thread-2"
                )
            )),
            "the client should learn that the turn was submitted"
        );
        let deadline = Instant::now() + EVENT_TIMEOUT;
        while turns.lock().expect("recorded turns should lock").is_empty() {
            assert!(Instant::now() < deadline, "the turn should run");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            turns.lock().expect("recorded turns should lock").as_slice(),
            [("thread-2".to_string(), "planned prompt".to_string())]
        );
        assert!(
            client
                .query()
                .expect("the host should answer queries")
                .snapshot
                .conversation_runtime
                .active_turn
                .is_none(),
            "the shown slot did not move"
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
        assert!(matches!(receiver.try_recv(), Ok(KernelHostEvent::Core(_))));
        assert!(deliver(&mut client, event.clone()));
        assert!(
            matches!(receiver.try_recv(), Ok(KernelHostEvent::Resync)),
            "the client learns it missed events before new ones"
        );
        drop(receiver);
        assert!(!deliver(&mut client, event), "a gone client is dropped");
    }

    #[test]
    fn parallel_control_plane_events_reach_clients() {
        let (runtime, _turns) = recording_runtime_for_test();
        let (_inbox, inputs) = mpsc::channel();
        let mut host = KernelHostLoop::new(runtime, inputs, KernelIdleClock::default());
        let (sender, receiver) = mpsc::sync_channel(4);
        host.clients.insert(
            1,
            ClientSink {
                events: sender,
                resync_pending: false,
            },
        );

        host.publish(NativeClientDispatchOutcome::Parallel(
            NativeParallelDispatchOutcome::from_events(vec![
                ParallelModeControlPlanePresentationEvent::ModeDisabled,
            ]),
        ));

        assert!(matches!(
            receiver.try_recv(),
            Ok(KernelHostEvent::Parallel(event))
                if matches!(*event, ParallelModeControlPlanePresentationEvent::ModeDisabled)
        ));
    }

    #[test]
    fn a_pending_resync_goes_out_without_a_new_event() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut client = ClientSink {
            events: sender,
            resync_pending: false,
        };
        let event = KernelHostEvent::Core(Box::new(AppEvent::ThreadActivityChanged(Vec::new())));
        assert!(deliver(&mut client, event.clone()));
        assert!(deliver(&mut client, event));
        assert!(client.resync_pending);

        assert!(flush_resync(&mut client), "a full queue keeps the resync");
        assert!(client.resync_pending);
        assert!(matches!(receiver.try_recv(), Ok(KernelHostEvent::Core(_))));
        assert!(flush_resync(&mut client));
        assert!(
            matches!(receiver.try_recv(), Ok(KernelHostEvent::Resync)),
            "the next host cycle tells the client even when no event follows"
        );
        assert!(!client.resync_pending);
    }

    fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + EVENT_TIMEOUT;
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn a_kernel_is_idle_only_without_clients_and_work() {
        let (runtime, _turns) = recording_runtime_for_test();
        let host = KernelHost::start(runtime).expect("kernel host thread should start");
        assert!(
            wait_until(|| host.idle_for(Instant::now()).is_some()),
            "a kernel without clients and work becomes idle"
        );

        let client = host.attach().expect("a client should attach");
        assert_eq!(
            host.idle_for(Instant::now()),
            None,
            "an attached client clears the idle time before attach returns"
        );

        drop(client);
        assert!(
            wait_until(|| host.idle_for(Instant::now()).is_some()),
            "the kernel is idle again after the client leaves"
        );
    }

    /// 이미 한가한 시각이 정해진 호스트 고리에서 `record_idle` 을 한 번 부르고 한가한지 돌려준다.
    fn idle_after_record(runtime: NativeClientRuntime) -> bool {
        let (_inbox, inputs) = mpsc::channel();
        let idle = KernelIdleClock::default();
        idle.record(false, Instant::now());
        let host = KernelHostLoop::new(runtime, inputs, idle.clone());

        host.record_idle(Instant::now());

        idle.idle_since().is_some()
    }

    #[test]
    fn remaining_work_keeps_the_kernel_busy() {
        let (runtime, _turns) = recording_runtime_for_test();
        assert!(
            idle_after_record(runtime),
            "without clients and work the idle mark stays"
        );

        let (mut runtime, _turns) = recording_runtime_for_test();
        runtime.begin_test_turn_submission();
        assert!(!idle_after_record(runtime), "a turn is remaining work");

        let (runtime, _turns) = recording_runtime_for_test();
        runtime.force_parallel_epoch_for_test("/repo", 7);
        assert!(
            !idle_after_record(runtime),
            "an open parallel automation epoch is remaining work"
        );
    }

    #[test]
    fn stopping_the_host_closes_the_parallel_automation_epoch() {
        // K3-5: worker threads of a dropped kernel must not dispatch next to a new kernel.
        let (runtime, _turns) = recording_runtime_for_test();
        runtime.force_parallel_epoch_for_test("/repo", 7);
        let epoch_is_active = runtime.parallel_automation_probe_for_test();
        assert!(epoch_is_active("/repo", 7));
        let host = KernelHost::start(runtime).expect("kernel host thread should start");

        drop(host);

        assert!(
            !epoch_is_active("/repo", 7),
            "the host thread closes the epoch before the kernel releases its lock"
        );
    }
}
