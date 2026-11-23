use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::future::BoxFuture;
use tokio::sync::oneshot;

use crate::application::port::outbound::agent_backend_port::BackendHealth;

const REPLY: &str = r#"{"decision":"start","confidence":9500,"reasons":["The bounded evidence supports this option."],"blockers":[]}"#;

#[derive(Clone)]
struct Captured {
    id: String,
    job_id: String,
    input: String,
    max_resumes: Option<u32>,
    chat: bool,
    callback_token: String,
    callback_workspace: String,
    model: Option<String>,
    profile: Option<TextDispatchProfile>,
    timeout: Duration,
}

#[derive(Default)]
struct DriverState {
    calls: Vec<Captured>,
    attempts: BTreeMap<String, Option<String>>,
    stopped: Vec<String>,
    forgotten: Vec<String>,
}

#[derive(Default)]
struct Driver {
    state: Arc<Mutex<DriverState>>,
    dispatch_fails: AtomicBool,
    completion_gate: Mutex<Option<oneshot::Receiver<()>>>,
    completion_ack: Mutex<Option<oneshot::Sender<()>>>,
}

fn observation(delivery: Delivery, phase: Phase, reason: Option<&str>) -> Observation {
    Observation {
        delivery,
        phase,
        reason: reason.map(str::to_owned),
        conversation_url: None,
        recovery_exhausted: phase != Phase::Working,
        recovery: None,
        resumes: Some(0),
        retry_at: None,
    }
}

impl Driver {
    fn status(&self, id: &str) -> Observation {
        match self.state.lock().unwrap().attempts.get(id) {
            Some(Some(_)) => observation(Delivery::Observed, Phase::Complete, None),
            Some(None) => observation(Delivery::Uncertain, Phase::Working, None),
            None => observation(
                Delivery::Uncertain,
                Phase::Disconnected,
                Some("codex_restart"),
            ),
        }
    }

    fn complete(&self, id: &str, json: &str) {
        self.state
            .lock()
            .unwrap()
            .attempts
            .insert(id.to_owned(), Some(json.to_owned()));
    }
}

impl AgentBackendPort for Driver {
    fn health(&self) -> BoxFuture<'_, BackendHealth> {
        Box::pin(async { panic!("assessment must not start a health/authentication probe") })
    }

    fn dispatch<'a>(&'a self, request: Dispatch<'a>) -> BoxFuture<'a, Result<Observation>> {
        Box::pin(async move {
            self.state.lock().unwrap().calls.push(Captured {
                id: request.attempt_id.to_owned(),
                job_id: request.job_id.to_owned(),
                input: request.instruction.to_owned(),
                max_resumes: request.max_resumes,
                chat: request.chat_session.is_some(),
                callback_token: request.callback_token.to_owned(),
                callback_workspace: request.callback_workspace.to_owned(),
                model: request.model.map(str::to_owned),
                profile: request.profile,
                timeout: request.timeout,
            });
            if self.dispatch_fails.load(Ordering::SeqCst) {
                return Err(anyhow!("fixture lost the dispatch response"));
            }
            self.state
                .lock()
                .unwrap()
                .attempts
                .insert(request.attempt_id.to_owned(), None);
            if let Some(gate) = self.completion_gate.lock().unwrap().take() {
                let state = Arc::clone(&self.state);
                let id = request.attempt_id.to_owned();
                let acknowledgement = self.completion_ack.lock().unwrap().take();
                tokio::spawn(async move {
                    if gate.await.is_ok() {
                        state
                            .lock()
                            .unwrap()
                            .attempts
                            .insert(id, Some(REPLY.to_owned()));
                    }
                    if let Some(acknowledgement) = acknowledgement {
                        let _ = acknowledgement.send(());
                    }
                });
            }
            Ok(self.status(request.attempt_id))
        })
    }

    fn observe<'a>(&'a self, attempt_id: &'a str) -> BoxFuture<'a, Result<Observation>> {
        Box::pin(async move { Ok(self.status(attempt_id)) })
    }

    fn collect<'a>(&'a self, attempt_id: &'a str) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            self.state
                .lock()
                .unwrap()
                .attempts
                .get(attempt_id)
                .cloned()
                .flatten()
                .ok_or_else(|| anyhow!("fixture response unavailable"))
        })
    }

    fn stop<'a>(&'a self, attempt_id: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            state.stopped.push(attempt_id.to_owned());
            state.attempts.remove(attempt_id);
            Ok(())
        })
    }

    fn forget(&self, attempt_id: &str) {
        let mut state = self.state.lock().unwrap();
        state.forgotten.push(attempt_id.to_owned());
        state.attempts.remove(attempt_id);
    }
}

fn configuration() -> IssueAssessmentModelConfiguration {
    IssueAssessmentModelConfiguration {
        route_revision: "a".repeat(64),
        model: Some("fixture-model".into()),
        max_running: 1,
        max_started_per_day: 5,
        timeout_seconds: 180,
        profile_version: ISSUE_ASSESSMENT_PROFILE_VERSION.into(),
    }
}

fn request() -> IssueAssessmentModelRequest {
    IssueAssessmentModelRequest {
        attempt_id: "issue-arbiter:fixture-1".into(),
        input_json: r#"{"shared_issue":"bounded fixture","options":["start","wait"]}"#.into(),
        expected_route_revision: configuration().route_revision,
        model: configuration().model,
        timeout: Duration::from_secs(30),
    }
}

fn adapter(driver: Arc<Driver>) -> CodexIssueAssessmentModelAdapter {
    CodexIssueAssessmentModelAdapter::with_driver(driver, Arc::new(|| Ok(configuration()))).unwrap()
}

#[test]
fn assessment_uses_the_fixed_profile_without_a_workspace_or_callback_capability() {
    let driver = Arc::new(Driver::default());
    let adapter = adapter(Arc::clone(&driver));
    let request = request();
    assert_eq!(adapter.dispatch(request.clone()).unwrap(), Status::Pending);
    let calls = &driver.state.lock().unwrap().calls;
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.id, request.attempt_id);
    assert_eq!(call.job_id, request.attempt_id);
    assert_eq!(call.input, request.input_json);
    assert_eq!(call.max_resumes, Some(0));
    assert!(!call.chat);
    assert!(call.callback_token.is_empty());
    assert!(call.callback_workspace.is_empty());
    assert_eq!(call.model, request.model);
    assert_eq!(call.timeout, request.timeout);
    let profile = call.profile.unwrap();
    assert_eq!(
        profile.developer_instructions,
        ISSUE_ASSESSMENT_INSTRUCTIONS
    );
    assert!((profile.accepts_reply)(REPLY));
    assert!(!(profile.accepts_reply)(
        r#"{"decision":"start","confidence":10000,"reasons":[],"blockers":[],"by":"human"}"#
    ));
}

#[test]
fn working_uncertain_is_pending_and_the_runtime_survives_dispatch_return() {
    let driver = Arc::new(Driver::default());
    let (release, gate) = oneshot::channel();
    let (ack, finished) = oneshot::channel();
    *driver.completion_gate.lock().unwrap() = Some(gate);
    *driver.completion_ack.lock().unwrap() = Some(ack);
    let adapter = adapter(Arc::clone(&driver));
    let request = request();
    assert_eq!(adapter.dispatch(request.clone()).unwrap(), Status::Pending);
    assert_eq!(
        adapter.observe(&request.attempt_id).unwrap(),
        Status::Pending
    );
    release.send(()).unwrap();
    adapter.runtime().unwrap().block_on(async {
        tokio::time::timeout(Duration::from_secs(2), finished)
            .await
            .unwrap()
            .unwrap();
    });
    assert_eq!(
        adapter.observe(&request.attempt_id).unwrap(),
        Status::Complete { json: REPLY.into() }
    );
    assert_eq!(driver.state.lock().unwrap().calls.len(), 1);
}

#[test]
fn restart_observation_never_dispatches_and_lost_dispatch_is_not_retried() {
    let driver = Arc::new(Driver::default());
    let adapter = adapter(Arc::clone(&driver));
    assert_eq!(
        adapter.observe("issue-arbiter:from-durable-claim").unwrap(),
        Status::Unusable(Failure::Uncertain)
    );
    assert!(driver.state.lock().unwrap().calls.is_empty());
    driver.dispatch_fails.store(true, Ordering::SeqCst);
    let request = request();
    assert_eq!(
        adapter.dispatch(request.clone()).unwrap(),
        Status::Unusable(Failure::Uncertain)
    );
    assert_eq!(
        adapter.dispatch(request).unwrap(),
        Status::Unusable(Failure::Uncertain)
    );
    assert_eq!(driver.state.lock().unwrap().calls.len(), 1);
}

#[test]
fn a_complete_response_is_retained_until_the_host_confirms_its_durable_finish() {
    let driver = Arc::new(Driver::default());
    let adapter = adapter(Arc::clone(&driver));
    let request = request();
    adapter.dispatch(request.clone()).unwrap();
    driver.complete(&request.attempt_id, REPLY);
    let expected = Status::Complete { json: REPLY.into() };
    // A simulated store failure does not call forget. Re-observation returns identical input.
    assert_eq!(adapter.observe(&request.attempt_id).unwrap(), expected);
    assert_eq!(adapter.observe(&request.attempt_id).unwrap(), expected);
    assert!(driver.state.lock().unwrap().forgotten.is_empty());
    adapter.forget(&request.attempt_id);
    assert_eq!(
        driver.state.lock().unwrap().forgotten,
        vec![request.attempt_id.clone()]
    );
    assert_eq!(
        adapter.observe(&request.attempt_id).unwrap(),
        Status::Unusable(Failure::Uncertain)
    );
    assert_eq!(driver.state.lock().unwrap().calls.len(), 1);
}

#[test]
fn repeated_identical_dispatch_observes_and_rebinding_the_attempt_is_rejected() {
    let driver = Arc::new(Driver::default());
    let adapter = adapter(Arc::clone(&driver));
    let mut request = request();
    assert_eq!(adapter.dispatch(request.clone()).unwrap(), Status::Pending);
    assert_eq!(adapter.dispatch(request.clone()).unwrap(), Status::Pending);
    request.input_json = r#"{"shared_issue":"different"}"#.into();
    assert_eq!(
        adapter.dispatch(request).unwrap(),
        Status::Unusable(Failure::Rejected)
    );
    assert_eq!(driver.state.lock().unwrap().calls.len(), 1);
}

#[test]
fn configuration_drift_or_unapproved_model_timeout_and_size_dispatch_nothing() {
    let driver = Arc::new(Driver::default());
    let current = Arc::new(Mutex::new(configuration()));
    let reader = Arc::clone(&current);
    let adapter = CodexIssueAssessmentModelAdapter::with_driver(
        Arc::clone(&driver) as Arc<dyn AgentBackendPort>,
        Arc::new(move || Ok(reader.lock().unwrap().clone())),
    )
    .unwrap();
    current.lock().unwrap().route_revision = "b".repeat(64);
    assert_eq!(
        adapter.dispatch(request()).unwrap(),
        Status::Unusable(Failure::ConfigurationChanged)
    );
    *current.lock().unwrap() = configuration();
    let mut unapproved_model = request();
    unapproved_model.model = Some("unapproved-model".into());
    assert_eq!(
        adapter.dispatch(unapproved_model).unwrap(),
        Status::Unusable(Failure::Rejected)
    );
    let mut oversized = request();
    oversized.input_json = format!(
        "{{\"input\":\"{}\"}}",
        "x".repeat(MAX_ISSUE_ASSESSMENT_INPUT_BYTES)
    );
    assert_eq!(
        adapter.dispatch(oversized).unwrap(),
        Status::Unusable(Failure::Rejected)
    );
    let mut over_deadline = request();
    over_deadline.timeout = Duration::from_secs(181);
    assert_eq!(
        adapter.dispatch(over_deadline).unwrap(),
        Status::Unusable(Failure::Rejected)
    );
    let mut invalid_shape = request();
    invalid_shape.input_json = "[]".into();
    assert_eq!(
        adapter.dispatch(invalid_shape).unwrap(),
        Status::Unusable(Failure::Rejected)
    );
    current.lock().unwrap().max_started_per_day = 0;
    assert_eq!(
        adapter.dispatch(request()).unwrap(),
        Status::Unusable(Failure::Rejected)
    );
    assert!(driver.state.lock().unwrap().calls.is_empty());
}

#[test]
fn malformed_or_unbounded_replies_are_unusable_without_altering_the_model_output() {
    let driver = Arc::new(Driver::default());
    let adapter = adapter(Arc::clone(&driver));
    let request = request();
    adapter.dispatch(request.clone()).unwrap();
    for json in [
        "not JSON".to_owned(),
        r#"{"decision":"start","confidence":10001,"reasons":[],"blockers":[]}"#.into(),
        r#"{"decision":"start","confidence":0.99,"reasons":[],"blockers":[]}"#.into(),
        r#"{"decision":"start","confidence":9500,"reasons":[],"blockers":[],"permit":true}"#.into(),
        "x".repeat(MAX_ISSUE_ASSESSMENT_REPLY_BYTES + 1),
    ] {
        driver.complete(&request.attempt_id, &json);
        assert_eq!(
            adapter.observe(&request.attempt_id).unwrap(),
            Status::Unusable(Failure::InvalidReply)
        );
    }
    driver.complete(&request.attempt_id, REPLY);
    assert_eq!(
        adapter.observe(&request.attempt_id).unwrap(),
        Status::Complete { json: REPLY.into() }
    );
}

#[test]
fn cancellation_stops_the_existing_attempt_without_dispatching_a_replacement() {
    let driver = Arc::new(Driver::default());
    let adapter = adapter(Arc::clone(&driver));
    let request = request();
    adapter.dispatch(request.clone()).unwrap();
    adapter.cancel(&request.attempt_id).unwrap();
    assert_eq!(
        driver.state.lock().unwrap().stopped,
        vec![request.attempt_id.clone()]
    );
    assert_eq!(
        adapter.observe(&request.attempt_id).unwrap(),
        Status::Unusable(Failure::Uncertain)
    );
    assert_eq!(driver.state.lock().unwrap().calls.len(), 1);
}

#[test]
fn using_the_adapter_inside_an_async_worker_fails_before_dispatch_without_panicking() {
    let driver = Arc::new(Driver::default());
    let adapter = adapter(Arc::clone(&driver));
    let caller = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    caller.block_on(async {
        assert!(adapter.dispatch(request()).is_err());
    });
    assert!(driver.state.lock().unwrap().calls.is_empty());
}
