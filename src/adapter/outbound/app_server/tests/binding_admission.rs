use super::*;
use crate::application::port::agent_action::AgentActionGuard;
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerStartAuthorization;
use crate::application::service::issue_binding_admission::{
    IssueBindingActionGuard, IssueBindingAdmission,
};
use crate::domain::issue_automation::{
    IssueAutomationBindingId, binding_pause::IssueBindingPauseState,
};

fn state(
    binding: &IssueAutomationBindingId,
    revision: u64,
    paused: bool,
) -> IssueBindingPauseState {
    IssueBindingPauseState {
        binding_id: binding.clone(),
        revision,
        paused,
        pending: None,
    }
}

struct Authorization {
    admission: Arc<IssueBindingAdmission>,
    binding: IssueAutomationBindingId,
    close_before_admission: bool,
    close_after_admission: bool,
}

impl ParallelAgentWorkerStartAuthorization for Authorization {
    fn validate(&self) -> Result<()> {
        Ok(())
    }
    fn acquire(&self) -> Result<Box<dyn AgentActionGuard>> {
        if self.close_before_admission {
            self.admission
                .begin_transition(&self.binding, || Ok(((), state(&self.binding, 1, true))))?;
        }
        let guard = self.admission.acquire(&self.binding, 0)?;
        Ok(Box::new(Guard {
            guard,
            admission: self.admission.clone(),
            binding: self.binding.clone(),
            close_after_admission: self.close_after_admission,
        }))
    }
}

struct Guard {
    guard: IssueBindingActionGuard,
    admission: Arc<IssueBindingAdmission>,
    binding: IssueAutomationBindingId,
    close_after_admission: bool,
}

impl AgentActionGuard for Guard {
    fn validate(&self) -> Result<()> {
        self.guard.validate()?;
        if self.close_after_admission {
            self.admission
                .begin_transition(&self.binding, || Ok(((), state(&self.binding, 1, true))))?;
            assert!(self.admission.has_inflight());
            assert!(
                self.admission
                    .finish_if_drained(
                        &self.binding,
                        1,
                        || -> Result<((), IssueBindingPauseState)> {
                            panic!("Pause cannot complete before the admitted RPC settles")
                        }
                    )?
                    .is_none()
            );
        }
        Ok(())
    }
}

fn authorization(close_before: bool, close_after: bool) -> Authorization {
    let admission = Arc::new(IssueBindingAdmission::new());
    let binding = IssueAutomationBindingId::new("worker-binding-a").unwrap();
    admission
        .register_if_absent(&state(&binding, 0, false))
        .unwrap();
    Authorization {
        admission,
        binding,
        close_before_admission: close_before,
        close_after_admission: close_after,
    }
}

fn run_new(
    fake: &FakeCodex,
    authorization: &Authorization,
) -> Result<ConversationTurnTerminalReceipt> {
    let (sender, _receiver) = conversation_stream_channel();
    test_adapter_with_fake(fake).run_authorized_isolated_new_thread_stream(
        ParallelAgentWorkerStreamRequest {
            cwd: "/repo",
            evidence_incoming_root: None,
            prompt: "accepted work",
            developer_instructions: "",
            service_name: "pause-fixture",
        },
        sender,
        authorization,
    )
}

#[test]
fn pause_after_new_or_resume_preparation_prevents_the_actual_turn_start() {
    for resumed in [false, true] {
        let fake = FakeCodex::install_with_scenario(
            "pause-worker-prepared",
            if resumed {
                "continuation_active"
            } else {
                "completed"
            },
        );
        let authorization = authorization(true, false);
        let result = if resumed {
            let (sender, _receiver) = conversation_stream_channel();
            test_adapter_with_fake(&fake).run_isolated_resumed_thread_stream(
                ParallelAgentWorkerResumeRequest {
                    cwd: "/repo",
                    evidence_incoming_root: None,
                    thread_id: "waiting-thread",
                    prompt: "accepted answer",
                    service_name: "pause-fixture",
                },
                sender,
                &authorization,
            )
        } else {
            run_new(&fake, &authorization)
        };
        assert!(result.is_err());
        let methods = fake.logged_methods();
        assert!(methods.iter().any(|method| method
            == if resumed {
                "thread/resume"
            } else {
                "thread/start"
            }));
        assert!(!methods.iter().any(|method| method == "turn/start"));
        assert!(!authorization.admission.has_inflight());

        let other = IssueAutomationBindingId::new("worker-binding-b").unwrap();
        authorization
            .admission
            .register_if_absent(&state(&other, 0, false))
            .unwrap();
        let other = Authorization {
            binding: other,
            close_before_admission: false,
            close_after_admission: false,
            admission: authorization.admission.clone(),
        };
        let other_fake = FakeCodex::install("unpaused-worker");
        assert!(
            run_new(&other_fake, &other)
                .unwrap()
                .is_completed_and_confirmed()
        );
    }
}

#[test]
fn accepted_and_unknown_start_responses_drain_before_pause_completion() {
    for scenario in ["completed", "fail_turn_start"] {
        let fake = FakeCodex::install_with_scenario("pause-rpc-drain", scenario);
        let authorization = authorization(false, true);
        let result = run_new(&fake, &authorization);
        assert_eq!(result.is_ok(), scenario == "completed");
        assert_eq!(
            fake.logged_methods()
                .iter()
                .filter(|method| *method == "turn/start")
                .count(),
            1
        );
        assert!(!authorization.admission.has_inflight());
        assert!(
            authorization
                .admission
                .finish_if_drained(&authorization.binding, 1, || {
                    Ok(((), state(&authorization.binding, 1, true)))
                })
                .unwrap()
                .is_some()
        );
        assert!(
            authorization
                .admission
                .acquire(&authorization.binding, 0)
                .is_err()
        );
    }
}
