//! Run the same application-owned stream contract against real Codex wire
//! decoding (a scripted local peer) and a non-Codex runtime. No model call.
use crate::adapter::outbound::app_server::{
    AppServerConnectionConfig, AppServerExecutionPolicy, CodexAppServerAdapter,
};
use crate::application::port::conversation_stream::{
    ConversationStreamEvent as Event, ConversationStreamSender, conversation_stream_channel,
};
use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalRequest, ConversationApprovalRequestKind,
    ConversationApprovalResolution, ConversationRuntimeControlTruth, ConversationSnapshot,
    ConversationTurnOptions,
};
use crate::domain::conversation_progressive_activity::{
    ConversationProgressiveActivityBatch, ConversationProgressiveActivityKind,
    ConversationProgressiveActivityObservation, ConversationProgressiveActivityPayload,
    ConversationProgressiveTokenUsage, ConversationProgressiveTokenUsageBreakdown as Tokens,
};
use crate::domain::task_usage::{UsageGap, UsageTurn};
use crate::domain::turn_terminal::{
    ConversationTurnApplicationDelivery, ConversationTurnError, ConversationTurnTerminalOutcome,
    ConversationTurnTerminalReceipt, ConversationTurnTerminalUncertainty,
};
use anyhow::{Result, bail};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    UsageRetry,
    Interrupted,
    Failed,
    Unknown,
    Approval,
    CancelRace,
}

impl Scenario {
    fn wire(self) -> &'static str {
        match self {
            Self::UsageRetry => "conformance_usage_retry",
            Self::Interrupted => "terminal_interrupted",
            Self::Failed => "terminal_failed",
            Self::Unknown => "terminal_unknown",
            Self::Approval => "conformance_approval",
            Self::CancelRace => "conformance_cancel_race",
        }
    }
    fn expected(self) -> &'static str {
        match self {
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            _ => "completed",
        }
    }
    fn approval(self) -> bool {
        matches!(self, Self::Approval | Self::CancelRace)
    }
}

#[derive(Default)]
struct Control {
    active: Option<(String, String)>,
    pending: bool,
    resolution: Option<ConversationApprovalResolution>,
}

struct NonCodex {
    scenario: Scenario,
    control: Mutex<Control>,
    changed: Condvar,
}

impl NonCodex {
    fn run(
        &self,
        thread_id: &str,
        sender: ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        let turn_id = "fake-turn";
        sender.send(Event::ThreadPrepared {
            thread_id: thread_id.into(),
            title: "fixture".into(),
            cwd: "fixture".into(),
            runtime_envelope: Box::default(),
        })?;
        sender.send(Event::TurnStarted {
            turn_id: turn_id.into(),
            runtime_request: Box::default(),
        })?;
        if self.scenario == Scenario::UsageRetry {
            let tokens = Tokens {
                input_tokens: 60,
                total_tokens: 60,
                ..Default::default()
            };
            for sequence in 1..=2 {
                let batch = ConversationProgressiveActivityBatch::single(
                    ConversationProgressiveActivityObservation {
                        sequence,
                        thread_id: thread_id.into(),
                        turn_id: Some(turn_id.into()),
                        item_id: None,
                        kind: ConversationProgressiveActivityKind::TokenUsage,
                        payload: ConversationProgressiveActivityPayload::TokenUsage {
                            usage: ConversationProgressiveTokenUsage {
                                last: tokens,
                                total: tokens,
                                model_context_window: Some(1000),
                            },
                        },
                    },
                )
                .expect("valid synthetic usage");
                sender.send(Event::ProgressiveActivityObserved {
                    batch: Box::new(batch),
                })?;
            }
            sender.send(Event::TurnRetrying {
                thread_id: thread_id.into(),
                turn_id: turn_id.into(),
                error: ConversationTurnError::new("temporary fixture retry", None::<&str>, None),
            })?;
        }
        if self.scenario.approval() {
            let request = ConversationApprovalRequest {
                approval_id: "approval-fake".into(),
                server_request_id: "server-fake".into(),
                method: "fixture".into(),
                kind: ConversationApprovalRequestKind::CommandExecution,
                summary: "fixture".into(),
                details: Vec::new(),
            };
            let identity = request.identity();
            {
                let mut control = self.control.lock().unwrap();
                control.pending = true;
                control.resolution = None;
                control.active = Some((thread_id.into(), turn_id.into()));
            }
            sender.send(Event::ApprovalRequested { request })?;
            let control = self.control.lock().unwrap();
            let (mut control, _) = self
                .changed
                .wait_timeout_while(control, Duration::from_secs(5), |c| c.resolution.is_none())
                .unwrap();
            let resolution = control
                .resolution
                .take()
                .ok_or_else(|| anyhow::anyhow!("fixture approval timed out"))?;
            control.pending = false;
            control.active = None;
            drop(control);
            sender.send(Event::ApprovalResolved {
                request_identity: identity,
                resolution,
            })?;
        }
        let outcome = match self.scenario {
            Scenario::Interrupted => ConversationTurnTerminalOutcome::Interrupted,
            Scenario::Failed => ConversationTurnTerminalOutcome::Failed {
                error: ConversationTurnError::new("fixture failed", None::<&str>, None),
            },
            Scenario::Unknown => ConversationTurnTerminalOutcome::Unknown {
                reason: ConversationTurnTerminalUncertainty::UnknownStatus("fixture".into()),
                observed_error: None,
            },
            _ => ConversationTurnTerminalOutcome::Completed,
        };
        let receipt = ConversationTurnTerminalReceipt::new(thread_id, turn_id, outcome)
            .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);
        sender.send(Event::TurnTerminal {
            receipt: receipt.clone(),
        })?;
        Ok(receipt)
    }
}

impl InteractiveTurnRuntimePort for NonCodex {
    fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
        ConversationRuntimeControlTruth::default()
    }
    fn load_conversation_snapshot(&self, _: &str) -> Result<ConversationSnapshot> {
        bail!("unsupported fixture snapshot")
    }
    fn request_stop_all_sessions(&self) -> Result<()> {
        bail!("unsupported fixture global stop")
    }
    fn run_new_thread_stream(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: ConversationTurnOptions,
        sender: ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        self.run("fake-thread", sender)
    }
    fn run_turn_stream(
        &self,
        thread_id: &str,
        _: &str,
        _: &[String],
        _: ConversationTurnOptions,
        sender: ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        self.run(thread_id, sender)
    }
    fn resolve_approval_request(
        &self,
        id: &str,
        decision: ConversationApprovalDecision,
    ) -> Result<()> {
        let mut control = self.control.lock().unwrap();
        if !control.pending || id != "approval-fake" {
            bail!("approval is no longer pending");
        }
        control.pending = false;
        control.resolution = Some(match decision {
            ConversationApprovalDecision::Accept => ConversationApprovalResolution::Accepted,
            ConversationApprovalDecision::Decline => ConversationApprovalResolution::Declined,
        });
        self.changed.notify_all();
        Ok(())
    }
    fn request_turn_interrupt(&self, thread_id: &str, turn_id: &str) -> Result<()> {
        let mut control = self.control.lock().unwrap();
        if control.pending
            && control
                .active
                .as_ref()
                .is_some_and(|(thread, turn)| thread == thread_id && turn == turn_id)
        {
            control.pending = false;
            control.resolution = Some(ConversationApprovalResolution::Interrupted);
            self.changed.notify_all();
        }
        Ok(())
    }
}

fn assert_scenario(
    runtime: Arc<dyn InteractiveTurnRuntimePort>,
    cwd: &str,
    scenario: Scenario,
    resume: bool,
) {
    println!("conformance: {scenario:?}, resume={resume}");
    let (sender, receiver) = conversation_stream_channel();
    let worker = runtime.clone();
    let cwd = cwd.to_owned();
    let handle = std::thread::spawn(move || {
        if resume {
            worker.run_turn_stream(
                "resume-thread",
                "synthetic prompt",
                &[],
                ConversationTurnOptions::default(),
                sender,
            )
        } else {
            worker.run_new_thread_stream(
                &cwd,
                "synthetic prompt",
                &[],
                ConversationTurnOptions::default(),
                sender,
            )
        }
    });
    let mut thread = None;
    let mut turn = None;
    let mut usage = None;
    let mut retry_count = 0;
    let mut approval = None;
    let mut approval_resolved = false;
    let mut terminal = None;
    while let Ok(event) = receiver.recv_timeout(Duration::from_secs(15)) {
        assert!(terminal.is_none(), "event after terminal: {event:?}");
        match event {
            Event::ThreadPrepared { thread_id, .. } => {
                assert!(thread.is_none());
                if resume {
                    assert_eq!(thread_id, "resume-thread");
                }
                thread = Some(thread_id);
            }
            Event::TurnStarted { turn_id, .. } => {
                assert!(turn.is_none());
                let thread_id = thread.as_ref().expect("thread precedes turn");
                usage = Some(UsageTurn::new(thread_id.clone(), turn_id.clone(), None, 0));
                turn = Some(turn_id);
            }
            Event::ProgressiveActivityObserved { batch } => {
                for record in batch.records() {
                    let observation = record.observation();
                    if let ConversationProgressiveActivityPayload::TokenUsage { usage: tokens } =
                        &observation.payload
                    {
                        assert_eq!(Some(&observation.thread_id), thread.as_ref());
                        assert_eq!(observation.turn_id.as_ref(), turn.as_ref());
                        usage.as_mut().unwrap().observe(
                            i64::try_from(record.last_sequence()).unwrap(),
                            tokens.last,
                            tokens.total,
                        );
                    }
                }
            }
            Event::TurnRetrying {
                thread_id, turn_id, ..
            } => {
                assert_eq!(Some(&thread_id), thread.as_ref());
                assert_eq!(Some(&turn_id), turn.as_ref());
                retry_count += 1;
            }
            Event::ApprovalRequested { request } => {
                assert!(approval.is_none());
                approval = Some(request.identity());
                if scenario == Scenario::CancelRace {
                    runtime
                        .request_turn_interrupt(
                            thread.as_deref().unwrap(),
                            turn.as_deref().unwrap(),
                        )
                        .unwrap();
                } else {
                    runtime
                        .resolve_approval_request(
                            &request.approval_id,
                            ConversationApprovalDecision::Accept,
                        )
                        .unwrap();
                    assert!(
                        runtime
                            .resolve_approval_request(
                                &request.approval_id,
                                ConversationApprovalDecision::Accept
                            )
                            .is_err()
                    );
                }
            }
            Event::ApprovalResolved {
                request_identity,
                resolution,
            } => {
                assert_eq!(Some(&request_identity), approval.as_ref());
                assert!(!approval_resolved);
                assert_eq!(
                    resolution,
                    if scenario == Scenario::CancelRace {
                        ConversationApprovalResolution::Interrupted
                    } else {
                        ConversationApprovalResolution::Accepted
                    }
                );
                approval_resolved = true;
                assert!(
                    runtime
                        .resolve_approval_request(
                            &request_identity.approval_id,
                            ConversationApprovalDecision::Accept
                        )
                        .is_err()
                );
            }
            Event::TurnTerminal { receipt } => {
                assert_eq!(Some(&receipt.thread_id), thread.as_ref());
                assert_eq!(Some(&receipt.turn_id), turn.as_ref());
                terminal = Some(receipt);
            }
            Event::Failed { message } => panic!("unexpected failure: {message}"),
            _ => {}
        }
    }
    let result = handle.join().unwrap().unwrap();
    let terminal = terminal.expect("one authoritative terminal");
    assert_eq!(result, terminal);
    assert_eq!(terminal.outcome.status_label(), scenario.expected());
    assert_eq!(
        terminal.is_completed_and_confirmed(),
        scenario.expected() == "completed"
    );
    assert_eq!(retry_count, usize::from(scenario == Scenario::UsageRetry));
    assert_eq!(approval_resolved, scenario.approval());
    let execution = usage.unwrap().execution();
    if scenario == Scenario::UsageRetry {
        assert_eq!(execution.observed_tokens.unwrap().total_tokens, 60);
        assert!(execution.gaps.is_empty());
    } else {
        assert!(execution.observed_tokens.is_none());
        assert!(execution.gaps.contains(&UsageGap::UsageNotObserved));
    }
    assert!(
        runtime
            .resolve_approval_request("absent-approval", ConversationApprovalDecision::Accept)
            .is_err()
    );
}

const SCENARIOS: [Scenario; 6] = [
    Scenario::UsageRetry,
    Scenario::Interrupted,
    Scenario::Failed,
    Scenario::Unknown,
    Scenario::Approval,
    Scenario::CancelRace,
];

#[test]
fn runtime_conformance_non_codex() {
    for scenario in SCENARIOS {
        for resume in [false, true] {
            let runtime = Arc::new(NonCodex {
                scenario,
                control: Mutex::default(),
                changed: Condvar::new(),
            });
            assert_scenario(runtime, "fixture", scenario, resume);
        }
    }
}

#[test]
fn runtime_conformance_codex_scripted_transport() {
    use std::path::PathBuf;
    use std::process::Command;
    let program = if cfg!(windows) { "python" } else { "python3" };
    let interpreter = Command::new(program)
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .expect("Python interpreter for scripted transport");
    assert!(interpreter.status.success());
    let executable = PathBuf::from(String::from_utf8(interpreter.stdout).unwrap().trim());
    let cwd = std::env::current_dir().unwrap();
    let path = std::env::temp_dir().join(format!(
        "akra-conformance-{}-{}.py",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let cleanup = Cleanup(path);
    std::fs::write(&cleanup.0, super::fake_codex_script()).unwrap();
    for scenario in SCENARIOS {
        for resume in [false, true] {
            let mut environment = vec![
                (
                    "AKRA_FAKE_APP_SERVER_SCENARIO".into(),
                    scenario.wire().into(),
                ),
                ("AKRA_CONFORMANCE_CWD".into(), cwd.as_os_str().to_owned()),
                ("PYTHONIOENCODING".into(), "utf-8".into()),
            ];
            if let Some(path) = std::env::var_os("PATH") {
                environment.push(("PATH".into(), path));
            }
            let config = AppServerConnectionConfig::default()
                .with_test_process(executable.clone(), environment)
                .with_test_script(&cleanup.0);
            let runtime = Arc::new(CodexAppServerAdapter::with_configs(
                "conformance",
                "fixture",
                config,
                AppServerExecutionPolicy::default(),
            ));
            assert_scenario(runtime, cwd.to_str().unwrap(), scenario, resume);
        }
    }
}
