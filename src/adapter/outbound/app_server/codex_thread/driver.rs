use std::collections::HashMap;
use std::sync::Mutex;

use futures_util::future::BoxFuture;

use crate::application::port::outbound::agent_backend_port::{
    AgentBackendPort, BackendCapabilities, BackendHealth, Dispatch, Observation,
};
use crate::domain::delegation::{BackendState, BackendStatus, Phase};

use super::*;

#[derive(Default)]
pub(crate) struct CodexThreadDriver {
    pub(super) backend: CodexThreadBackend,
    attempts: Mutex<HashMap<String, Arc<AttemptState>>>,
}

#[derive(Default)]
struct AttemptState {
    cancelled: AtomicBool,
    started: AtomicBool,
    outcome: Mutex<Option<CodexThreadOutcome>>,
}

impl Drop for CodexThreadDriver {
    fn drop(&mut self) {
        for state in self.attempts.get_mut().unwrap().values() {
            state.cancelled.store(true, Ordering::Release);
        }
    }
}

impl CodexThreadDriver {
    #[cfg(all(test, unix))]
    pub(crate) fn fixture(command: PathBuf) -> Self {
        Self {
            backend: CodexThreadBackend {
                fixture: Some(command),
            },
            attempts: Mutex::new(HashMap::new()),
        }
    }

    fn attempt(&self, id: &str) -> Option<Arc<AttemptState>> {
        self.attempts.lock().unwrap().get(id).cloned()
    }
}

impl CodexThreadDriver {
    pub(crate) fn dispatch_with_authorization<'a>(
        &'a self,
        request: Dispatch<'a>,
        authorization: Option<
            Arc<dyn crate::application::port::agent_action::AgentActionAuthorization>,
        >,
    ) -> BoxFuture<'a, Result<Observation>> {
        Box::pin(async move {
            {
                let mut attempts = self.attempts.lock().unwrap();
                if attempts.len() >= 32 && !attempts.contains_key(request.attempt_id) {
                    return Ok(observation(
                        Delivery::NotSent,
                        Phase::Failed,
                        Some("transient"),
                        true,
                    ));
                }
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    attempts.entry(request.attempt_id.to_owned())
                {
                    let state = Arc::new(AttemptState::default());
                    entry.insert(Arc::clone(&state));
                    let backend = self.backend.clone();
                    let model = request.model.map(str::to_owned);
                    let text = request.instruction.to_owned();
                    let profile = request.profile;
                    let timeout = request.timeout;
                    tokio::task::spawn_blocking(move || {
                        let request = CodexThreadRequest {
                            model: model.as_deref(),
                            developer_instructions: profile.map_or(
                                "Use the read-only isolated environment. Return text only.",
                                |profile| profile.developer_instructions,
                            ),
                            text: &text,
                            output_schema: None,
                        };
                        let mut outcome = backend.run_authorized(
                            request,
                            &state.cancelled,
                            timeout,
                            authorization.as_deref(),
                            || {
                                state.started.store(true, Ordering::Release);
                            },
                        );
                        if let Some(profile) = profile
                            && let CodexThreadOutcome::Complete(reply) = &outcome
                            && !(profile.accepts_reply)(reply)
                        {
                            outcome = CodexThreadOutcome::End {
                                reason: EndReason::ResultInvalid,
                                delivery: Delivery::Observed,
                            };
                        }
                        *state.outcome.lock().unwrap() = Some(outcome);
                    });
                }
            }
            self.observe(request.attempt_id).await
        })
    }
}

impl AgentBackendPort for CodexThreadDriver {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            // The profile sets a read-only sandbox and checks the applied policy.
            // Stock Codex still exposes local read tools.
            defensive_isolation: false,
        }
    }

    fn health(&self) -> BoxFuture<'_, BackendHealth> {
        Box::pin(async move {
            let backend = self.backend.clone();
            let readiness = tokio::task::spawn_blocking(move || backend.readiness())
                .await
                .unwrap_or(Err(UNAVAILABLE));
            BackendHealth {
                status: BackendStatus {
                    state: if readiness.is_ok() {
                        BackendState::Ready
                    } else {
                        BackendState::Unavailable
                    },
                    reason: readiness.err().map(str::to_owned),
                },
                retry_at: None,
            }
        })
    }

    fn dispatch<'a>(&'a self, request: Dispatch<'a>) -> BoxFuture<'a, Result<Observation>> {
        self.dispatch_with_authorization(request, None)
    }

    fn observe<'a>(&'a self, attempt_id: &'a str) -> BoxFuture<'a, Result<Observation>> {
        Box::pin(async move {
            let Some(state) = self.attempt(attempt_id) else {
                return Ok(observation(
                    Delivery::Uncertain,
                    Phase::Disconnected,
                    Some("codex_restart"),
                    true,
                ));
            };
            let outcome = state.outcome.lock().unwrap();
            Ok(match outcome.as_ref() {
                Some(CodexThreadOutcome::Complete(_)) => {
                    observation(Delivery::Observed, Phase::Complete, None, true)
                }
                Some(CodexThreadOutcome::End { reason, delivery }) => {
                    let (phase, label) = match reason {
                        EndReason::Unavailable => (Phase::Failed, "unavailable"),
                        EndReason::Quota => (Phase::Failed, "quota"),
                        EndReason::Transient => (Phase::Failed, "transient"),
                        EndReason::Interrupted => (Phase::Disconnected, "codex_interrupted"),
                        EndReason::NeedsHuman => (Phase::Blocked, "needs_human"),
                        EndReason::Rejected => (Phase::Failed, "rejected"),
                        EndReason::ResultInvalid => (Phase::Failed, "result_invalid"),
                        EndReason::Unknown => (Phase::Failed, "unknown"),
                    };
                    observation(*delivery, phase, Some(label), true)
                }
                None => observation(
                    if state.started.load(Ordering::Acquire) {
                        Delivery::Observed
                    } else {
                        Delivery::Uncertain
                    },
                    Phase::Working,
                    None,
                    false,
                ),
            })
        })
    }

    fn collect<'a>(&'a self, attempt_id: &'a str) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let state = self
                .attempt(attempt_id)
                .context("Codex attempt is no longer available")?;
            let result = state.outcome.lock().unwrap();
            match result.as_ref() {
                Some(CodexThreadOutcome::Complete(text)) => Ok(text.clone()),
                _ => anyhow::bail!("Codex result is not available"),
            }
        })
    }

    fn stop<'a>(&'a self, attempt_id: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            if let Some(state) = self.attempts.lock().unwrap().remove(attempt_id) {
                state.cancelled.store(true, Ordering::Release);
            }
            Ok(())
        })
    }

    fn forget(&self, attempt_id: &str) {
        self.attempts.lock().unwrap().remove(attempt_id);
    }
}

fn observation(delivery: Delivery, phase: Phase, reason: Option<&str>, ended: bool) -> Observation {
    Observation {
        delivery,
        phase,
        reason: reason.map(str::to_owned),
        conversation_url: None,
        recovery_exhausted: ended,
        recovery: None,
        resumes: None,
        retry_at: None,
    }
}
