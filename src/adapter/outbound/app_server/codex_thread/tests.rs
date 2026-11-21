use super::*;

#[test]
fn codex_thread_capabilities_do_not_claim_defensive_isolation() {
    use crate::application::port::outbound::agent_backend_port::AgentBackendPort;
    assert!(
        !CodexThreadDriver::default()
            .capabilities()
            .defensive_isolation
    );
}

#[test]
#[ignore = "requires pinned stock Codex; opens an ephemeral thread without starting a turn"]
fn real_no_turn_isolation() {
    CodexThreadBackend::default().probe().unwrap();
}

#[cfg(unix)]
pub(crate) struct Fixture(IsolatedDirectory);

#[cfg(unix)]
impl Fixture {
    pub(crate) fn new(scenario: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let directory = IsolatedDirectory::create().unwrap();
        let path = directory.root.join("codex-fixture");
        let fixture = include_str!("fixture.py").replace("D2_SCENARIO", scenario);
        fs::write(&path, fixture).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(directory)
    }
    fn backend(&self) -> CodexThreadBackend {
        CodexThreadBackend {
            fixture: Some(self.0.root.join("codex-fixture")),
        }
    }
    pub(crate) fn driver(&self) -> CodexThreadDriver {
        CodexThreadDriver::fixture(self.0.root.join("codex-fixture"))
    }

    pub(crate) fn calls(&self) -> String {
        fs::read_to_string(self.0.root.join("calls")).unwrap_or_default()
    }
}

#[cfg(unix)]
fn run(
    backend: &CodexThreadBackend,
    cancel: &AtomicBool,
    started: impl FnOnce(),
) -> CodexThreadOutcome {
    let draft = r#"{"title":"Trusted title","body":"Run with [masked-1]."}"#;
    backend.run(
        issue_draft_review_request(Some("fixture-model"), draft),
        cancel,
        Duration::from_secs(5),
        started,
    )
}

#[cfg(unix)]
#[test]
fn fixture_classifies_terminal_receipts_and_accepts_only_final_text() {
    for (scenario, reason) in [
        ("completed", None),
        ("approval", None),
        ("quota", Some(EndReason::Quota)),
        ("failed", Some(EndReason::Transient)),
        ("unknown", Some(EndReason::Unknown)),
        ("interrupted", Some(EndReason::Interrupted)),
        ("exit_after_start", Some(EndReason::Interrupted)),
        ("bad_policy", Some(EndReason::Unavailable)),
        ("no_mcp_isolation", Some(EndReason::Unavailable)),
    ] {
        let fixture = Fixture::new(scenario);
        let result = run(&fixture.backend(), &AtomicBool::new(false), || {});
        let expected = match reason {
            None => CodexThreadOutcome::Complete(r#"{"suggestions":[]}"#.into()),
            Some(reason) => CodexThreadOutcome::End {
                reason,
                delivery: if reason == EndReason::Unavailable {
                    Delivery::NotSent
                } else {
                    Delivery::Observed
                },
            },
        };
        assert_eq!(result, expected, "{scenario}: {}", fixture.calls());
        assert_eq!(
            fixture
                .calls()
                .lines()
                .filter(|method| *method == "turn/start")
                .count(),
            usize::from(reason != Some(EndReason::Unavailable))
        );
    }
}

#[cfg(unix)]
#[test]
fn fixture_interrupts_exact_turn_and_never_resumes() {
    let fixture = Fixture::new("cancel");
    let cancel = AtomicBool::new(false);
    let result = run(&fixture.backend(), &cancel, || {
        cancel.store(true, Ordering::Release)
    });
    assert_eq!(
        result,
        CodexThreadOutcome::End {
            reason: EndReason::Interrupted,
            delivery: Delivery::Observed
        }
    );
    assert_eq!(
        fixture
            .calls()
            .lines()
            .filter(|method| *method == "turn/interrupt")
            .count(),
        1
    );
    assert!(!fixture.calls().contains("resume"));
}

#[cfg(unix)]
#[test]
fn missing_executable_is_unavailable_before_send() {
    let backend = CodexThreadBackend {
        fixture: Some(PathBuf::from("/__akra_d2_missing_fixture__")),
    };
    assert!(!backend.ready());
    assert_eq!(
        run(&backend, &AtomicBool::new(false), || panic!(
            "must not send"
        )),
        CodexThreadOutcome::End {
            reason: EndReason::Unavailable,
            delivery: Delivery::NotSent
        },
    );
}
