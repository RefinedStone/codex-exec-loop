//! Short synchronous calls around one long-lived read-only Codex driver.
//! This adapter never owns decision state, retry policy or durable call reservations.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow, ensure};
use sha2::{Digest, Sha256};

use crate::adapter::outbound::app_server::codex_thread::CodexThreadDriver;
use crate::application::port::outbound::agent_backend_port::{
    AgentBackendPort, Dispatch, Observation, TextDispatchProfile,
};
use crate::application::port::outbound::issue_assessment_model_port::{
    ISSUE_ASSESSMENT_PROFILE_VERSION, IssueAssessmentModelConfiguration,
    IssueAssessmentModelFailure as Failure, IssueAssessmentModelPort, IssueAssessmentModelRequest,
    IssueAssessmentModelStatus as Status, MAX_ISSUE_ASSESSMENT_INPUT_BYTES,
    MAX_ISSUE_ASSESSMENT_REPLY_BYTES,
};
use crate::domain::delegation::{Delivery, Phase};
use crate::domain::issue_automation::arbiter::IssueArbiterModelReply;

pub(crate) const ISSUE_ASSESSMENT_INSTRUCTIONS: &str =
    include_str!("../../../assets/app-server/skills/akra-issue-assessment/SKILL.md");
const MAX_DRIVER_ATTEMPTS: usize = 32;

type ConfigurationReader = dyn Fn() -> Result<IssueAssessmentModelConfiguration> + Send + Sync;

/// Keeps durable claims observable when a host cannot create its model runtime.
/// It never probes credentials, starts a backend or requests a replacement attempt.
pub(crate) struct UnavailableIssueAssessmentModelAdapter;

impl IssueAssessmentModelPort for UnavailableIssueAssessmentModelAdapter {
    fn configuration(&self) -> Result<IssueAssessmentModelConfiguration> {
        Err(anyhow!("the issue assessment runtime is unavailable"))
    }

    fn dispatch(&self, _request: IssueAssessmentModelRequest) -> Result<Status> {
        Ok(Status::Unusable(Failure::Unavailable))
    }

    fn observe(&self, _attempt_id: &str) -> Result<Status> {
        Ok(Status::Unusable(Failure::Uncertain))
    }

    fn cancel(&self, _attempt_id: &str) -> Result<()> {
        Ok(())
    }

    fn forget(&self, _attempt_id: &str) {}
}

pub(crate) struct CodexIssueAssessmentModelAdapter {
    // Production owns this driver exclusively. Dropping it cancels all live attempts
    // before the runtime is detached; no blocking model turn is awaited on host shutdown.
    driver: Option<Arc<dyn AgentBackendPort>>,
    runtime: Option<tokio::runtime::Runtime>,
    configuration: Arc<ConfigurationReader>,
    attempts: Mutex<BTreeMap<String, String>>,
}

impl CodexIssueAssessmentModelAdapter {
    pub(crate) fn new(configuration: Arc<ConfigurationReader>) -> Result<Self> {
        Self::build(Arc::new(CodexThreadDriver::default()), configuration)
    }

    fn build(
        driver: Arc<dyn AgentBackendPort>,
        configuration: Arc<ConfigurationReader>,
    ) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(MAX_DRIVER_ATTEMPTS)
            .enable_all()
            .build()
            .map_err(|_| anyhow!("the issue assessment runtime is unavailable"))?;
        Ok(Self {
            driver: Some(driver),
            runtime: Some(runtime),
            configuration,
            attempts: Mutex::new(BTreeMap::new()),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_driver(
        driver: Arc<dyn AgentBackendPort>,
        configuration: Arc<ConfigurationReader>,
    ) -> Result<Self> {
        Self::build(driver, configuration)
    }

    fn driver(&self) -> &dyn AgentBackendPort {
        self.driver.as_deref().expect("live assessment adapter")
    }

    fn runtime(&self) -> Result<&tokio::runtime::Runtime> {
        ensure!(
            tokio::runtime::Handle::try_current().is_err(),
            "issue assessment calls require a blocking host worker"
        );
        Ok(self.runtime.as_ref().expect("live assessment runtime"))
    }

    fn status(&self, attempt_id: &str, observation: Observation) -> Result<Status> {
        if observation.phase == Phase::Working && !observation.recovery_exhausted {
            // The driver cannot know whether a newly spawned turn was sent yet. That
            // Working/Uncertain state is different from a lost attempt after restart.
            return Ok(Status::Pending);
        }
        if observation.phase == Phase::Complete && observation.delivery == Delivery::Observed {
            return Ok(
                match self.runtime()?.block_on(self.driver().collect(attempt_id)) {
                    Ok(json) if valid_reply_shape(&json) => Status::Complete { json },
                    Ok(_) => Status::Unusable(Failure::InvalidReply),
                    Err(_) => Status::Unusable(Failure::Uncertain),
                },
            );
        }
        let failure = match observation.reason.as_deref() {
            Some("unavailable") => Failure::Unavailable,
            Some("quota") => Failure::Quota,
            Some("rejected" | "needs_human") => Failure::Rejected,
            Some("result_invalid") => Failure::InvalidReply,
            Some("codex_interrupted" | "interrupted") => Failure::Interrupted,
            Some("codex_restart") => Failure::Uncertain,
            _ if observation.phase == Phase::Stopped => Failure::Interrupted,
            _ => Failure::Uncertain,
        };
        Ok(Status::Unusable(failure))
    }
}

impl IssueAssessmentModelPort for CodexIssueAssessmentModelAdapter {
    fn configuration(&self) -> Result<IssueAssessmentModelConfiguration> {
        let configuration = (self.configuration)()
            .map_err(|_| anyhow!("the issue assessment configuration is unavailable"))?;
        ensure!(
            valid_configuration(&configuration),
            "the issue assessment configuration is invalid"
        );
        Ok(configuration)
    }

    fn dispatch(&self, request: IssueAssessmentModelRequest) -> Result<Status> {
        let configuration = match self.configuration() {
            Ok(configuration) => configuration,
            Err(_) => return Ok(Status::Unusable(Failure::Unavailable)),
        };
        if configuration.route_revision != request.expected_route_revision {
            return Ok(Status::Unusable(Failure::ConfigurationChanged));
        }
        if !valid_identifier(&request.attempt_id)
            || request.input_json.len() > MAX_ISSUE_ASSESSMENT_INPUT_BYTES
            || !serde_json::from_str::<serde_json::Value>(&request.input_json)
                .is_ok_and(|value| value.is_object())
            || request.model != configuration.model
            || request.timeout.is_zero()
            || request.timeout > Duration::from_secs(configuration.timeout_seconds)
            || configuration.max_running == 0
            || configuration.max_started_per_day == 0
        {
            return Ok(Status::Unusable(Failure::Rejected));
        }
        let identity = serde_json::to_vec(&(
            &request.expected_route_revision,
            &request.model,
            request.timeout.as_nanos(),
            &request.input_json,
        ))?;
        let fingerprint = format!("{:x}", Sha256::digest(identity));
        let mut attempts = self
            .attempts
            .lock()
            .map_err(|_| anyhow!("the issue assessment attempt registry is unavailable"))?;
        if let Some(previous) = attempts.get(&request.attempt_id) {
            if previous != &fingerprint {
                return Ok(Status::Unusable(Failure::Rejected));
            }
            drop(attempts);
            return self.observe(&request.attempt_id);
        }
        if attempts.len() >= MAX_DRIVER_ATTEMPTS {
            return Ok(Status::Unusable(Failure::Unavailable));
        }
        let runtime = self.runtime()?;
        // The durable claim already exists. A lost dispatch response is never retried.
        attempts.insert(request.attempt_id.clone(), fingerprint);
        let observation = runtime.block_on(self.driver().dispatch(Dispatch {
            attempt_id: &request.attempt_id,
            job_id: &request.attempt_id,
            instruction: &request.input_json,
            max_resumes: Some(0),
            chat_session: None,
            callback_token: "",
            callback_workspace: "",
            model: request.model.as_deref(),
            profile: Some(TextDispatchProfile {
                developer_instructions: ISSUE_ASSESSMENT_INSTRUCTIONS,
                accepts_reply: valid_reply_shape,
            }),
            timeout: request.timeout,
        }));
        drop(attempts);
        match observation {
            Ok(observation) => self.status(&request.attempt_id, observation),
            Err(_) => Ok(Status::Unusable(Failure::Uncertain)),
        }
    }

    fn observe(&self, attempt_id: &str) -> Result<Status> {
        ensure!(
            valid_identifier(attempt_id),
            "invalid assessment attempt identifier"
        );
        match self.runtime()?.block_on(self.driver().observe(attempt_id)) {
            Ok(observation) => self.status(attempt_id, observation),
            Err(_) => Ok(Status::Unusable(Failure::Uncertain)),
        }
    }

    fn cancel(&self, attempt_id: &str) -> Result<()> {
        ensure!(
            valid_identifier(attempt_id),
            "invalid assessment attempt identifier"
        );
        self.runtime()?
            .block_on(self.driver().stop(attempt_id))
            .map_err(|_| anyhow!("the issue assessment attempt could not be stopped"))
    }

    fn forget(&self, attempt_id: &str) {
        if !valid_identifier(attempt_id) {
            return;
        }
        self.driver().forget(attempt_id);
        if let Ok(mut attempts) = self.attempts.lock() {
            attempts.remove(attempt_id);
        }
    }
}

impl Drop for CodexIssueAssessmentModelAdapter {
    fn drop(&mut self) {
        drop(self.driver.take());
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-' | b'_' | b'.'))
}

fn valid_configuration(configuration: &IssueAssessmentModelConfiguration) -> bool {
    configuration.route_revision.len() == 64
        && configuration
            .route_revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && configuration.model.as_ref().is_none_or(|model| {
            !model.is_empty()
                && model.len() <= 128
                && model.trim() == model
                && !model.chars().any(char::is_control)
        })
        && (1..=86_400).contains(&configuration.timeout_seconds)
        && configuration.profile_version == ISSUE_ASSESSMENT_PROFILE_VERSION
}

fn valid_reply_shape(reply: &str) -> bool {
    reply.len() <= MAX_ISSUE_ASSESSMENT_REPLY_BYTES
        && IssueArbiterModelReply::parse_shape(reply).is_ok()
}

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;
