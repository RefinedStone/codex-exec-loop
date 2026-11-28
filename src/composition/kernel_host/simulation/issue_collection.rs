//! 이슈 worker는 제어 가능한 가짜 외부 조회이고, 커널 호스트와 시계 결합은 실제 코드다.
use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::application::port::inbound::stored_command_port::{
    StoredCommand, StoredCommandOutcome, StoredCommandPort, StoredCommandResult,
};
use crate::application::service::issue_collection::{
    IssueCollectionRunOutcome, IssueCollectionTrackedFact,
};
use crate::application::service::issue_triage::IssueTriagedFact;
use crate::composition::issue_observer::{IssueObserver, IssueObserverCommittedFact};
use crate::composition::native_client_runtime::{KernelClientCommand, StoredCommandEnd};
use crate::domain::issue_automation::collection::{
    IssueCollectionConfig, IssueCollectionMode, IssueCollectionProjection, IssueCollectionStatus,
    StoredIssueCollection,
};
use crate::domain::issue_automation::{IssueAutomationBinding, IssueAutomationBindingId};
use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};
use crate::domain::kernel_event::ReasonCode;
use crate::domain::kernel_spec::KernelCommandKind;

const WAIT: Duration = Duration::from_secs(5);
type ReadResponse = Sender<anyhow::Result<IssueCollectionRunOutcome>>;

struct Collector {
    requests: Receiver<ReadResponse>,
    finished: Receiver<bool>,
    calls: Arc<AtomicUsize>,
}

struct Modes(Arc<Mutex<IssueCollectionProjection>>);
impl StoredCommandPort for Modes {
    fn execute(&self, command: StoredCommand) -> anyhow::Result<StoredCommandOutcome> {
        let StoredCommand::SetIssueAutomationMode { mode } = command else {
            anyhow::bail!("the fake only changes issue mode")
        };
        let mut current = self.0.lock().expect("mode store locks");
        let previous = current.state.config.mode;
        let changed = previous != mode;
        if changed {
            current.state.config.mode = mode;
            current.state.epoch += 1;
            current.state.revision += 1;
        }
        Ok(StoredCommandOutcome::Done(
            StoredCommandResult::IssueAutomationModeChanged {
                previous: Some(previous),
                projection: Box::new(current.clone()),
                changed,
            },
        ))
    }
}

fn projection(mode: IssueCollectionMode, epoch: u64, revision: u64) -> IssueCollectionProjection {
    let binding_id = IssueAutomationBindingId::new("sim-issues").expect("binding ID");
    let binding = IssueAutomationBinding::new(
        binding_id.clone(),
        IssueProvider::GitHub,
        "github.com",
        "R_sim",
        IssueProjectScope {
            workspace_dir: WORKSPACE.to_string(),
        },
    )
    .expect("binding");
    IssueCollectionProjection {
        state: StoredIssueCollection {
            config: IssueCollectionConfig { mode },
            epoch,
            revision,
            binding_id: Some(binding_id),
            discovery_cursor: None,
            resync_cursor: None,
            status: IssueCollectionStatus::default(),
        },
        binding: Some(binding),
        catalog_revision: revision,
        account_quota: Default::default(),
    }
}

fn references() -> Vec<IssueReference> {
    (1..=3)
        .map(|number| IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_string(),
            repository_id: "R_sim".to_string(),
            issue_id: format!("I_{number}"),
            project_scope: IssueProjectScope {
                workspace_dir: WORKSPACE.to_string(),
            },
        })
        .collect()
}

fn install(sim: &mut SimKernel, mode: IssueCollectionMode) -> Collector {
    let current = Arc::new(Mutex::new(projection(mode, 1, 1)));
    sim.host
        .runtime
        .attach_stored_commands(Arc::new(Modes(current.clone())), WORKSPACE);
    let (requests, receiver) = mpsc::channel();
    let (finished, completion) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let worker_calls = calls.clone();
    let initial = current.lock().expect("initial projection").clone();
    let observer = IssueObserver::for_test_with_committed(
        initial,
        Arc::new(move |cancel, _, report| {
            worker_calls.fetch_add(1, Ordering::SeqCst);
            let (response, answer) = mpsc::channel();
            requests
                .send(response)
                .expect("simulation receives the request");
            let result: anyhow::Result<IssueCollectionRunOutcome> =
                answer.recv().expect("simulation releases the request");
            let cancelled = cancel.is_cancelled();
            if !cancelled && let Ok(outcome) = &result {
                *current.lock().expect("fake commit") = outcome.projection.clone();
                report(
                    outcome
                        .newly_tracked
                        .iter()
                        .map(|issue| IssueCollectionTrackedFact {
                            issue: issue.clone(),
                            binding_id: outcome
                                .projection
                                .state
                                .binding_id
                                .clone()
                                .expect("committed binding"),
                            revision: outcome.projection.state.revision,
                        })
                        .map(IssueObserverCommittedFact::Tracked)
                        .collect(),
                );
            }
            finished
                .send(cancelled)
                .expect("simulation receives completion");
            result
        }),
        sim.host.runtime.host_waker(),
    );
    sim.host.runtime.attach_issue_observer_for_test(observer);
    Collector {
        requests: receiver,
        finished: completion,
        calls,
    }
}

fn successful(revision: u64, newly_tracked: Vec<IssueReference>) -> IssueCollectionRunOutcome {
    IssueCollectionRunOutcome {
        projection: projection(IssueCollectionMode::Observe, 1, revision),
        newly_tracked,
        discarded: false,
    }
}

fn release(
    sim: &mut SimKernel,
    collector: &Collector,
    request: ReadResponse,
    outcome: IssueCollectionRunOutcome,
    now: Instant,
) -> bool {
    // 이전 명령의 깨우기를 비우고, worker가 mailbox를 채운 뒤의 깨우기로 다음 주기를 돈다.
    sim.host.run_cycle(now, Duration::from_nanos(1));
    request.send(Ok(outcome)).expect("worker receives result");
    let cancelled = collector
        .finished
        .recv_timeout(WAIT)
        .expect("worker finishes");
    for _ in 0..3 {
        sim.host.run_cycle(now, WAIT);
        if !sim
            .host
            .runtime
            .issue_collection_view(now)
            .expect("view")
            .collecting
        {
            break;
        }
    }
    cancelled
}

fn mode_command(sim: &mut SimKernel, mode: IssueCollectionMode) -> KernelCommandOutcome {
    let (reply, answer) = mpsc::channel();
    sim.host.handle_input(KernelHostInput::Command {
        command: KernelClientCommand::Stored(Box::new(StoredCommand::SetIssueAutomationMode {
            mode,
        })),
        descriptor: Some(KernelCommandDescriptor {
            command: KernelCommandKind::SetIssueAutomationMode,
            thread_id: String::new(),
            fingerprint: format!("issue-mode-{mode:?}"),
            idempotency_key: None,
            confirmed: true,
            caller: Some("collector-simulation".to_string()),
        }),
        reply,
    });
    answer
        .try_recv()
        .expect("mode command answers without a planning worker")
}

#[test]
fn kernel_simulation_observe_collects_three_new_records_once_at_the_five_minute_deadline() {
    let mut sim = build_sim_kernel();
    let log = Arc::new(SimKernelLog::default());
    sim.host
        .runtime
        .attach_kernel_event_log(log.clone(), WORKSPACE);
    let collector = install(&mut sim, IssueCollectionMode::Observe);
    let (reply, attached) = mpsc::channel();
    sim.host.handle_input(KernelHostInput::Attach { reply });
    let (_, events) = attached.recv().expect("client attached");
    let now = Instant::now() + Duration::from_secs(86_400);
    assert_eq!(sim.host.runtime.next_deadline(now), Some(now));
    sim.host.run_cycle(now, Duration::ZERO);
    let first = collector
        .requests
        .recv_timeout(WAIT)
        .expect("first discovery");
    assert!(!release(
        &mut sim,
        &collector,
        first,
        successful(2, references()),
        now
    ));
    assert_eq!(
        sim.host.runtime.next_deadline(now),
        Some(now + Duration::from_secs(300))
    );
    sim.host
        .run_cycle(now + Duration::from_millis(299_999), Duration::ZERO);
    assert!(collector.requests.try_recv().is_err());
    let due = now + Duration::from_secs(300);
    sim.host.run_cycle(due, Duration::ZERO);
    let second = collector
        .requests
        .recv_timeout(WAIT)
        .expect("next discovery");
    assert!(!release(
        &mut sim,
        &collector,
        second,
        successful(3, Vec::new()),
        due
    ));
    let events: Vec<_> = events.try_iter().collect();
    assert!(events.iter().any(|event| matches!(event,
        KernelHostEvent::Issues(state) if !state.collecting && state.projection.state.revision == 2
            && state.next_poll_after_ms == Some(300_000)
    )), "events use the same fake clock as the collection deadline");
    let visible = events
        .iter()
        .filter(|event| {
            matches!(
                event, KernelHostEvent::Recorded(record) if record.event.kind() == "issue_tracked"
            )
        })
        .count();
    assert_eq!(
        visible, 3,
        "a repeated page does not repeat committed new-item events"
    );
    assert_eq!(collector.calls.load(Ordering::SeqCst), 2);
    drop(sim);
    let records = log.0.lock().expect("flushed log");
    assert_eq!(
        records
            .iter()
            .filter(|record| record.event.kind() == "issue_tracked")
            .count(),
        3
    );
}

#[test]
fn kernel_simulation_off_makes_no_source_calls_and_mode_answers_bypass_planning() {
    let mut sim = build_sim_kernel();
    let log = Arc::new(SimKernelLog::default());
    sim.host
        .runtime
        .attach_kernel_event_log(log.clone(), WORKSPACE);
    let collector = install(&mut sim, IssueCollectionMode::Off);
    let now = Instant::now() + Duration::from_secs(86_400);
    sim.host.run_cycle(now, Duration::ZERO);
    assert_eq!(
        sim.host
            .handle_command(KernelClientCommand::SyncIssues, None),
        KernelCommandOutcome::Rejected(ReasonCode::IssueAutomationOff),
    );
    sim.host
        .run_cycle(now + Duration::from_secs(86_400), Duration::ZERO);
    assert_eq!(collector.calls.load(Ordering::SeqCst), 0);
    let answer = mode_command(&mut sim, IssueCollectionMode::Observe);
    let KernelCommandOutcome::Handled {
        events,
        stored:
            Some(StoredCommandEnd::Finished(StoredCommandOutcome::Done(
                StoredCommandResult::IssueAutomationModeChanged { changed, .. },
            ))),
        ..
    } = answer
    else {
        panic!("stored mode result")
    };
    assert!(changed);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AppEvent::PlanningRuntimeRefreshStarted { .. }))
    );
    assert!(sim.host.held_stored_answers.is_empty());
    let again = mode_command(&mut sim, IssueCollectionMode::Observe);
    assert!(matches!(
        again,
        KernelCommandOutcome::Handled {
            stored: Some(StoredCommandEnd::Finished(StoredCommandOutcome::Done(
                StoredCommandResult::IssueAutomationModeChanged { changed: false, .. }
            ))),
            ..
        }
    ));
    drop(sim);
    let records = log.0.lock().expect("flushed log");
    let modes: Vec<_> = records
        .iter()
        .filter(|record| record.event.kind() == "issue_automation_mode_changed")
        .collect();
    assert_eq!(modes.len(), 1, "no-op mode saves emit no mode change");
    let json = serde_json::to_value(&modes[0].event).expect("record serializes");
    assert_eq!(json["caller"], "collector-simulation");
    assert_eq!(json["by"], "command");
}

#[test]
fn kernel_simulation_off_during_a_slow_read_discards_its_response() {
    let mut sim = build_sim_kernel();
    let log = Arc::new(SimKernelLog::default());
    sim.host
        .runtime
        .attach_kernel_event_log(log.clone(), WORKSPACE);
    let collector = install(&mut sim, IssueCollectionMode::Observe);
    let now = Instant::now();
    sim.host.run_cycle(now, Duration::ZERO);
    let request = collector
        .requests
        .recv_timeout(WAIT)
        .expect("slow discovery started");
    let _ = mode_command(&mut sim, IssueCollectionMode::Off);
    assert_eq!(
        sim.host
            .handle_command(KernelClientCommand::SyncIssues, None),
        KernelCommandOutcome::Rejected(ReasonCode::IssueAutomationOff),
    );
    assert!(release(
        &mut sim,
        &collector,
        request,
        successful(2, references()),
        now
    ));
    let current = sim
        .host
        .runtime
        .issue_collection_view(now)
        .expect("collection projection");
    assert_eq!(
        current.projection.state.config.mode,
        IssueCollectionMode::Off
    );
    assert_eq!(current.projection.state.epoch, 2);
    assert!(!current.collecting);
    assert_eq!(current.next_poll_after_ms, None);
    assert_eq!(collector.calls.load(Ordering::SeqCst), 1);
    drop(sim);
    let records = log.0.lock().expect("flushed log");
    assert_eq!(
        records
            .iter()
            .filter(|record| record.event.kind() == "issue_tracked")
            .count(),
        0
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.event.kind() == "issue_automation_mode_changed")
            .count(),
        1
    );
}

#[test]
fn kernel_simulation_a_slow_read_allows_commands_and_does_not_prevent_idle_unload() {
    let mut sim = build_sim_kernel();
    let collector = install(&mut sim, IssueCollectionMode::Observe);
    let now = Instant::now();
    sim.host.run_cycle(now, Duration::ZERO);
    let request = collector
        .requests
        .recv_timeout(WAIT)
        .expect("slow discovery started");
    let outcome = sim.host.handle_command(
        KernelClientCommand::Core(AppCommand::SetAutoFollowMaxTurns { value: 2 }),
        None,
    );
    assert!(matches!(outcome, KernelCommandOutcome::Handled { .. }));
    assert_eq!(
        sim.host
            .runtime
            .snapshot()
            .conversation_runtime
            .auto_follow
            .max_auto_turns,
        2
    );
    assert!(
        !sim.host.runtime.has_remaining_work(),
        "observe is not executable remaining work"
    );
    sim.host.record_idle(now);
    assert_eq!(sim.host.idle.idle_since(), Some(now));
    let (closed, closure) = mpsc::channel();
    let drop_worker =
        crate::composition::core_effect_worker::spawn_joinable_redacted_worker(move || {
            drop(sim);
            closed.send(()).expect("test receives unload");
        });
    closure
        .recv_timeout(WAIT)
        .expect("unload does not join a blocked provider");
    drop_worker.join().expect("idle unload worker completed");
    request
        .send(Ok(successful(2, references())))
        .expect("release old worker");
    assert!(
        collector
            .finished
            .recv_timeout(WAIT)
            .expect("detached worker finishes cancelled")
    );
}

struct CommittedRead {
    commit: Sender<()>,
    committed: Receiver<()>,
    finish: Sender<anyhow::Result<IssueCollectionRunOutcome>>,
    finished: Receiver<bool>,
}

fn install_committed_read(sim: &mut SimKernel) -> CommittedRead {
    let current = Arc::new(Mutex::new(projection(IssueCollectionMode::Observe, 1, 1)));
    sim.host
        .runtime
        .attach_stored_commands(Arc::new(Modes(current.clone())), WORKSPACE);
    let (commit, commit_read) = mpsc::channel();
    let (committed_send, committed) = mpsc::channel();
    let (finish, finish_read) = mpsc::channel();
    let (finished_send, finished) = mpsc::channel();
    let commit_read = Mutex::new(commit_read);
    let finish_read = Mutex::new(finish_read);
    let observer = IssueObserver::for_test_with_committed(
        projection(IssueCollectionMode::Observe, 1, 1),
        Arc::new(move |cancel, _, report| {
            commit_read
                .lock()
                .expect("commit trigger")
                .recv()
                .expect("commit requested");
            let committed_projection = projection(IssueCollectionMode::Observe, 1, 2);
            *current.lock().expect("fake committed state") = committed_projection.clone();
            report(
                references()
                    .into_iter()
                    .map(|issue| IssueCollectionTrackedFact {
                        issue,
                        binding_id: committed_projection
                            .state
                            .binding_id
                            .clone()
                            .expect("binding"),
                        revision: 2,
                    })
                    .flat_map(|fact| {
                        let triaged = IssueTriagedFact {
                            issue: fact.issue.clone(),
                            binding_id: fact.binding_id.clone(),
                            revision: fact.revision,
                            assessment_revision: 1,
                            gate_revision: 1,
                            policy_version: "a".repeat(64),
                            field_hash: None,
                        };
                        [
                            IssueObserverCommittedFact::Tracked(fact),
                            IssueObserverCommittedFact::Triaged(triaged),
                        ]
                    })
                    .collect(),
            );
            committed_send.send(()).expect("commit acknowledged");
            let result = finish_read
                .lock()
                .expect("finish trigger")
                .recv()
                .expect("finish requested");
            finished_send
                .send(cancel.is_cancelled())
                .expect("finish acknowledged");
            result
        }),
        sim.host.runtime.host_waker(),
    );
    sim.host.runtime.attach_issue_observer_for_test(observer);
    CommittedRead {
        commit,
        committed,
        finish,
        finished,
    }
}

#[test]
fn kernel_simulation_committed_issue_facts_survive_off_and_a_later_store_error() {
    let mut sim = build_sim_kernel();
    let log = Arc::new(SimKernelLog::default());
    sim.host
        .runtime
        .attach_kernel_event_log(log.clone(), WORKSPACE);
    let worker = install_committed_read(&mut sim);
    let now = Instant::now();
    sim.host.run_cycle(now, Duration::ZERO);
    worker.commit.send(()).expect("allow commit");
    worker
        .committed
        .recv_timeout(WAIT)
        .expect("facts enqueued before Off");
    let _ = mode_command(&mut sim, IssueCollectionMode::Off);
    worker
        .finish
        .send(Err(anyhow::anyhow!("synthetic later store failure")))
        .expect("finish worker");
    assert!(worker.finished.recv_timeout(WAIT).expect("cancel observed"));
    sim.host.run_cycle(now, Duration::ZERO);
    let current = sim
        .host
        .runtime
        .issue_collection_view(now)
        .expect("current mode");
    assert_eq!(
        current.projection.state.config.mode,
        IssueCollectionMode::Off
    );
    assert_eq!(current.projection.state.revision, 3);
    drop(sim);
    let records = log.0.lock().expect("flushed log");
    let facts: Vec<_> = records
        .iter()
        .filter(|record| record.event.kind() == "issue_tracked")
        .collect();
    assert_eq!(facts.len(), 3);
    assert!(
        facts
            .iter()
            .all(|record| matches!(&record.event, KernelEvent::IssueTracked { revision: 2, .. }))
    );
    assert_committed_audit_order(&records);
}

#[test]
fn kernel_simulation_unload_drains_committed_facts_before_journal_shutdown() {
    let mut sim = build_sim_kernel();
    let log = Arc::new(SimKernelLog::default());
    sim.host
        .runtime
        .attach_kernel_event_log(log.clone(), WORKSPACE);
    let worker = install_committed_read(&mut sim);
    sim.host.run_cycle(Instant::now(), Duration::ZERO);
    worker.commit.send(()).expect("allow commit");
    worker
        .committed
        .recv_timeout(WAIT)
        .expect("facts enqueued after the host cycle");
    let (closed, closure) = mpsc::channel();
    let drop_worker =
        crate::composition::core_effect_worker::spawn_joinable_redacted_worker(move || {
            drop(sim);
            closed.send(()).expect("unloaded");
        });
    closure
        .recv_timeout(WAIT)
        .expect("shutdown does not wait for the remaining read");
    drop_worker
        .join()
        .expect("journal shutdown worker completed");
    {
        let records = log.0.lock().expect("flushed log");
        assert_eq!(
            records
                .iter()
                .filter(|record| record.event.kind() == "issue_tracked")
                .count(),
            3
        );
        assert_committed_audit_order(&records);
    }
    worker
        .finish
        .send(Err(anyhow::anyhow!("synthetic post-unload failure")))
        .expect("release detached worker");
    assert!(
        worker
            .finished
            .recv_timeout(WAIT)
            .expect("detached worker cancelled")
    );
}

#[test]
fn kernel_simulation_committed_facts_emit_an_issue_event_before_the_read_finishes() {
    let mut sim = build_sim_kernel();
    let worker = install_committed_read(&mut sim);
    let (reply, attached) = mpsc::channel();
    sim.host.handle_input(KernelHostInput::Attach { reply });
    let (_, events) = attached.recv().expect("client attached");
    let now = Instant::now();
    sim.host.run_cycle(now, Duration::ZERO);
    events.try_iter().for_each(drop);
    worker.commit.send(()).expect("allow commit");
    worker.committed.recv_timeout(WAIT).expect("facts enqueued");
    sim.host.run_cycle(now, Duration::ZERO);
    assert!(
        sim.host
            .runtime
            .issue_collection_view(now)
            .expect("view")
            .collecting
    );
    assert!(
        events
            .try_iter()
            .any(|event| matches!(event, KernelHostEvent::Issues(_)))
    );
    worker
        .finish
        .send(Ok(successful(2, Vec::new())))
        .expect("finish read");
    worker.finished.recv_timeout(WAIT).expect("read finished");
}

#[test]
fn kernel_simulation_issue_queries_and_events_read_fresh_ledger_mode_and_binding() {
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let directory = Directory(crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-issue-host-view-{}-{suffix}",
        std::process::id()
    )));
    std::fs::create_dir_all(&directory.0).expect("test directory");
    let service =
        crate::composition::issue_collection::build_for_kernel_test(&directory.0.to_string_lossy());
    let mut sim = build_sim_kernel();
    sim.host
        .runtime
        .attach_issue_collection(service.clone())
        .expect("persisted default");
    let collector = install(&mut sim, IssueCollectionMode::Observe);
    let (reply, attached) = mpsc::channel();
    sim.host.handle_input(KernelHostInput::Attach { reply });
    let (_, events) = attached.recv().expect("client attached");
    let now = Instant::now();
    sim.host.run_cycle(now, Duration::ZERO);
    let request = collector
        .requests
        .recv_timeout(WAIT)
        .expect("controlled read");
    let saved = service
        .set_mode(IssueCollectionMode::Off)
        .expect("independent local save");
    let view = sim
        .host
        .runtime
        .issue_collection_view(now)
        .expect("fresh view");
    assert_eq!(view.projection, saved.projection);
    assert!(
        view.projection.binding.is_none(),
        "the cached fake binding is not displayed"
    );
    assert!(!view.collecting);
    assert_eq!(view.next_poll_after_ms, None);
    request
        .send(Err(anyhow::anyhow!("synthetic later store failure")))
        .expect("read finishes");
    collector
        .finished
        .recv_timeout(WAIT)
        .expect("worker releases");
    for _ in 0..3 {
        sim.host.run_cycle(now, WAIT);
        if events.try_iter().any(|event| {
            matches!(event,
                KernelHostEvent::Issues(state) if state.projection == saved.projection
                    && !state.collecting && state.next_poll_after_ms.is_none()
            )
        }) {
            return;
        }
    }
    panic!("the completion event must use the actual stored revision, mode and binding");
}

fn assert_committed_audit_order(records: &[RecordedKernelEvent]) {
    let facts: Vec<_> = records
        .iter()
        .filter(|record| matches!(record.event.kind(), "issue_tracked" | "issue_triaged"))
        .collect();
    assert_eq!(
        facts
            .iter()
            .map(|record| record.event.kind())
            .collect::<Vec<_>>(),
        [
            "issue_tracked",
            "issue_triaged",
            "issue_tracked",
            "issue_triaged",
            "issue_tracked",
            "issue_triaged"
        ]
    );
    assert!(facts.iter().filter(|record| record.event.kind() == "issue_triaged").all(|record|
        matches!(&record.event, KernelEvent::IssueTriaged { revision: 2, gate_revision: 1, policy_version, .. } if policy_version == &"a".repeat(64))));
}
