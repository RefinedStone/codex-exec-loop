use super::*;
use crate::application::port::agent_action::{AgentActionAuthorization, AgentActionGuard};
use crate::application::port::outbound::agent_backend_port::{
    AgentBackendPort, Dispatch, TextDispatchProfile,
};
use crate::application::service::issue_binding_admission::IssueBindingAdmission;
use crate::domain::issue_automation::{
    IssueAutomationBindingId, binding_pause::IssueBindingPauseState,
};

const INPUT: &str = r#"{"title":"Trusted title","body":"Run with [masked-1]."}"#;

fn state(binding: &IssueAutomationBindingId, paused: bool) -> IssueBindingPauseState {
    IssueBindingPauseState {
        binding_id: binding.clone(),
        revision: u64::from(paused),
        paused,
        pending: None,
    }
}

#[test]
fn aliased_temporary_parent_preserves_the_exact_isolated_protocol_cwd() {
    let fixture = Fixture::new("completed");
    let physical_parent = fixture.0.root.join("physical-temp");
    let alias = fixture.0.root.join("aliased-temp");
    fs::create_dir(&physical_parent).unwrap();
    std::os::unix::fs::symlink(&physical_parent, &alias).unwrap();
    let directory = IsolatedDirectory::create_beneath(&alias).unwrap();
    assert_eq!(directory.root, fs::canonicalize(&directory.root).unwrap());
    assert!(directory.root.starts_with(&physical_parent));
    assert_eq!(directory.cwd.parent(), directory.home.parent());

    let (mut connection, _) = fixture.backend().connect_authenticated(&directory).unwrap();
    let thread = prepare_thread(
        &mut connection,
        &directory.cwd,
        Some("fixture-model"),
        crate::application::service::issue_review::ISSUE_DRAFT_REVIEW_SKILL_TEXT,
    )
    .unwrap();
    assert_eq!(thread, "thread");
    assert!(fixture.calls().contains("thread/start"));
    assert!(!fixture.calls().contains("turn/start"));
}

#[tokio::test]
async fn delayed_arbiter_dispatch_obtains_binding_admission_inside_the_actual_rpc() {
    let fixture = Fixture::new("completed");
    let driver = fixture.driver();
    let admission = Arc::new(IssueBindingAdmission::new());
    let binding = IssueAutomationBindingId::new("arbiter-paused-binding").unwrap();
    admission
        .register_if_absent(&state(&binding, false))
        .unwrap();
    let (entered, waiting) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let released = std::sync::Mutex::new(released);
    let admission_for_rpc = admission.clone();
    let binding_for_rpc = binding.clone();
    let authorization: Arc<dyn AgentActionAuthorization> = Arc::new(move || {
        entered.send(()).unwrap();
        released
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(30))
            .unwrap();
        let guard = admission_for_rpc.acquire(&binding_for_rpc, 0)?;
        Ok(Box::new(guard) as Box<dyn AgentActionGuard>)
    });
    let observed = driver
        .dispatch_with_authorization(
            Dispatch {
                attempt_id: "paused-arbiter",
                job_id: "paused-arbiter",
                instruction: INPUT,
                max_resumes: Some(0),
                chat_session: None,
                callback_token: "",
                callback_workspace: "",
                model: Some("fixture-model"),
                profile: Some(TextDispatchProfile {
                    developer_instructions:
                        crate::application::service::issue_review::ISSUE_DRAFT_REVIEW_SKILL_TEXT,
                    accepts_reply: |_| true,
                }),
                timeout: Duration::from_secs(30),
            },
            Some(authorization),
        )
        .await
        .unwrap();
    assert_eq!(observed.phase, crate::domain::delegation::Phase::Working);
    waiting.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(fixture.calls().contains("thread/start"));
    assert!(!fixture.calls().contains("turn/start"));
    admission
        .begin_transition(&binding, || Ok(((), state(&binding, true))))
        .unwrap();
    assert!(
        admission
            .finish_if_drained(&binding, 1, || Ok(((), state(&binding, true))))
            .unwrap()
            .is_some()
    );
    release.send(()).unwrap();

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let observed = driver.observe("paused-arbiter").await.unwrap();
        if observed.recovery_exhausted {
            assert_eq!(observed.delivery, Delivery::NotSent);
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!fixture.calls().contains("turn/start"));
    assert!(!fixture.calls().contains("thread/resume"));
    assert!(!admission.has_inflight());
}
