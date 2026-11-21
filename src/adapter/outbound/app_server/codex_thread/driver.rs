use std::collections::HashMap;
use std::sync::Mutex;

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

    pub(crate) fn forget(&self, id: &str) {
        self.attempts.lock().unwrap().remove(id);
    }

    fn attempt(&self, id: &str) -> Option<Arc<AttemptState>> {
        self.attempts.lock().unwrap().get(id).cloned()
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

    async fn health(&self) -> BackendHealth {
        let backend = self.backend.clone();
        let ready = tokio::task::spawn_blocking(move || backend.ready())
            .await
            .unwrap_or(false);
        BackendHealth {
            status: BackendStatus {
                state: if ready {
                    BackendState::Ready
                } else {
                    BackendState::Unavailable
                },
                reason: (!ready).then(|| "codex_unavailable".into()),
            },
            retry_at: None,
        }
    }

    async fn dispatch(&self, request: Dispatch<'_>) -> Result<Observation> {
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
                let issue_draft = request.issue_draft;
                let timeout = request.timeout;
                tokio::task::spawn_blocking(move || {
                    let request = if issue_draft {
                        issue_draft_review_request(model.as_deref(), &text)
                    } else {
                        CodexThreadRequest {
                            model: model.as_deref(),
                            developer_instructions: "Use the read-only isolated environment. Return text only.",
                            text: &text,
                            output_schema: None,
                        }
                    };
                    let mut outcome = backend.run(request, &state.cancelled, timeout, || {
                        state.started.store(true, Ordering::Release);
                    });
                    if issue_draft && let CodexThreadOutcome::Complete(reply) = outcome {
                        outcome = issue_draft_review_reply(reply);
                    }
                    *state.outcome.lock().unwrap() = Some(outcome);
                });
            }
        }
        self.observe(request.attempt_id).await
    }

    async fn observe(&self, attempt_id: &str) -> Result<Observation> {
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
    }

    async fn collect(&self, attempt_id: &str) -> Result<String> {
        let state = self
            .attempt(attempt_id)
            .context("Codex attempt is no longer available")?;
        let result = state.outcome.lock().unwrap();
        match result.as_ref() {
            Some(CodexThreadOutcome::Complete(text)) => Ok(text.clone()),
            _ => anyhow::bail!("Codex result is not available"),
        }
    }

    async fn stop(&self, attempt_id: &str) -> Result<()> {
        if let Some(state) = self.attempts.lock().unwrap().remove(attempt_id) {
            state.cancelled.store(true, Ordering::Release);
        }
        Ok(())
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
