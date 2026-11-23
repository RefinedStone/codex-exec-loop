//! Deterministic transport for coordinator and kernel tests. Never starts a backend.
use std::collections::BTreeMap;
use std::sync::Mutex;

use anyhow::{Result, anyhow};

use crate::application::port::outbound::issue_assessment_model_port::{
    ISSUE_ASSESSMENT_PROFILE_VERSION, IssueAssessmentModelConfiguration,
    IssueAssessmentModelFailure, IssueAssessmentModelPort, IssueAssessmentModelRequest,
    IssueAssessmentModelStatus,
};

pub(crate) struct FakeIssueAssessmentModelAdapter {
    state: Mutex<State>,
}

struct State {
    configuration: Option<IssueAssessmentModelConfiguration>,
    dispatch_status: IssueAssessmentModelStatus,
    requests: Vec<IssueAssessmentModelRequest>,
    observations: Vec<String>,
    cancelled: Vec<String>,
    forgotten: Vec<String>,
    attempts: BTreeMap<String, IssueAssessmentModelStatus>,
}

impl Default for FakeIssueAssessmentModelAdapter {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                configuration: Some(IssueAssessmentModelConfiguration {
                    route_revision: "a".repeat(64),
                    model: None,
                    max_running: 1,
                    max_started_per_day: 5,
                    timeout_seconds: 180,
                    profile_version: ISSUE_ASSESSMENT_PROFILE_VERSION.to_owned(),
                }),
                dispatch_status: IssueAssessmentModelStatus::Pending,
                requests: Vec::new(),
                observations: Vec::new(),
                cancelled: Vec::new(),
                forgotten: Vec::new(),
                attempts: BTreeMap::new(),
            }),
        }
    }
}

impl FakeIssueAssessmentModelAdapter {
    pub(crate) fn set_configuration(&self, configuration: IssueAssessmentModelConfiguration) {
        self.state.lock().unwrap().configuration = Some(configuration);
    }

    pub(crate) fn make_unavailable(&self) {
        self.state.lock().unwrap().configuration = None;
    }

    pub(crate) fn set_dispatch_status(&self, status: IssueAssessmentModelStatus) {
        self.state.lock().unwrap().dispatch_status = status;
    }

    pub(crate) fn set_status(&self, attempt_id: &str, status: IssueAssessmentModelStatus) {
        self.state
            .lock()
            .unwrap()
            .attempts
            .insert(attempt_id.to_owned(), status);
    }

    pub(crate) fn complete(&self, attempt_id: &str, json: impl Into<String>) {
        self.set_status(
            attempt_id,
            IssueAssessmentModelStatus::Complete { json: json.into() },
        );
    }

    pub(crate) fn requests(&self) -> Vec<IssueAssessmentModelRequest> {
        self.state.lock().unwrap().requests.clone()
    }

    pub(crate) fn observations(&self) -> Vec<String> {
        self.state.lock().unwrap().observations.clone()
    }

    pub(crate) fn cancelled(&self) -> Vec<String> {
        self.state.lock().unwrap().cancelled.clone()
    }

    pub(crate) fn forgotten(&self) -> Vec<String> {
        self.state.lock().unwrap().forgotten.clone()
    }
}

impl IssueAssessmentModelPort for FakeIssueAssessmentModelAdapter {
    fn configuration(&self) -> Result<IssueAssessmentModelConfiguration> {
        self.state
            .lock()
            .unwrap()
            .configuration
            .clone()
            .ok_or_else(|| anyhow!("fixture configuration unavailable"))
    }

    fn dispatch(&self, request: IssueAssessmentModelRequest) -> Result<IssueAssessmentModelStatus> {
        let mut state = self.state.lock().unwrap();
        let status = state.dispatch_status.clone();
        state
            .attempts
            .insert(request.attempt_id.clone(), status.clone());
        state.requests.push(request);
        Ok(status)
    }

    fn observe(&self, attempt_id: &str) -> Result<IssueAssessmentModelStatus> {
        let mut state = self.state.lock().unwrap();
        state.observations.push(attempt_id.to_owned());
        Ok(state
            .attempts
            .get(attempt_id)
            .cloned()
            .unwrap_or(IssueAssessmentModelStatus::Unusable(
                IssueAssessmentModelFailure::Uncertain,
            )))
    }

    fn cancel(&self, attempt_id: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.cancelled.push(attempt_id.to_owned());
        state.attempts.insert(
            attempt_id.to_owned(),
            IssueAssessmentModelStatus::Unusable(IssueAssessmentModelFailure::Interrupted),
        );
        Ok(())
    }

    fn forget(&self, attempt_id: &str) {
        let mut state = self.state.lock().unwrap();
        state.forgotten.push(attempt_id.to_owned());
        state.attempts.remove(attempt_id);
    }
}
