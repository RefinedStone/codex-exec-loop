use super::*;
use crate::application::port::inbound::stored_command_port::{
    StoredCommandContext, issue_confirmation::VerifiedHumanConfirmation,
};
use crate::application::port::outbound::issue_automation_store_port::IssueCollectionModeSaveResult;
use crate::domain::issue_automation::delivery::{
    IssueDeliveryDiff, IssueDeliveryDiffFile, IssueDeliveryDiffLines, IssueDeliveryRiskReason,
};
use crate::domain::issue_automation::execution::{
    IssueExecutionPhase, IssueExecutionTerminalClass, IssueExecutionTerminalRecord,
    IssueWorkExecutionObservation,
};

fn track_result(
    coordinator: &IssueCoordinatorService,
    now: u64,
) -> (Result<()>, Vec<IssueCoordinatorFact>) {
    let facts = Mutex::new(Vec::new());
    let result = coordinator.track_execution(
        &IssueCollectionCancellation::new(),
        now,
        true,
        &|committed| facts.lock().unwrap().extend(committed),
    );
    (result, facts.into_inner().unwrap())
}

fn reassemble(
    a: &ArbiterHarness,
    model: Arc<FakeIssueAssessmentModelAdapter>,
) -> IssueCoordinatorService {
    let h = &a.h;
    let triage = Arc::new(h.fixture.service(h.policy.clone()));
    let decisions = Arc::new(IssueDecisionService::new(
        h.fixture.workspace.clone(),
        h.fixture.store(),
        triage.clone(),
        IssuePolicyService::new(Arc::new(ValidationPolicy)),
    ));
    let clock = a.clock.clone();
    IssueCoordinatorService::new(
        h.fixture.workspace.clone(),
        h.fixture.store(),
        h.source.clone(),
        triage,
        decisions,
        IssuePolicyService::new(Arc::new(ValidationPolicy)),
        h.execution.clone(),
    )
    .with_arbiter(model, a.evidence.clone())
    .with_arbiter_clock(Arc::new(move || *clock.lock().unwrap()))
}

fn admissions(h: &Harness) -> Vec<(String, i64, i64)> {
    let connection = rusqlite::Connection::open_with_flags(
        h.fixture.runtime.join("issue-automation.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut statement = connection
        .prepare(
            "SELECT work_order_id,utc_day,admitted_at_unix_ms FROM issue_work_admissions
         WHERE workspace_dir=?1 ORDER BY work_order_id",
        )
        .unwrap();
    statement
        .query_map([&h.fixture.workspace], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn freeze_protected_delivery(h: &Harness) {
    let mut state = h.execution.state.lock().unwrap();
    state.frozen = Some("b".repeat(40));
    state.delivery_diff = Some(IssueDeliveryDiff {
        base_commit_sha: "a".repeat(40),
        source_commit_sha: "b".repeat(40),
        files: vec![IssueDeliveryDiffFile {
            path: ".github/workflows/ci.yml".into(),
            lines: IssueDeliveryDiffLines::Text {
                added: 3,
                deleted: 1,
            },
        }],
    });
    state.observation = Some(IssueWorkExecutionObservation {
        phase: IssueExecutionPhase::Queued,
        task_ids: vec!["task-1".into()],
        active_task_id: Some("task-1".into()),
        ..Default::default()
    });
}

fn delivery_request(h: &Harness) -> StoredIssueDecision {
    page(&h.triage)
        .items
        .into_iter()
        .flat_map(|item| item.open_decisions)
        .find(|stored| stored.request.definition().is_delivery_risk_exception())
        .unwrap()
}

fn answer_delivery(coordinator: &IssueCoordinatorService, request: &IssueDecisionRequest) {
    let id = request.id().as_str();
    let preview = coordinator
        .decisions
        .prepare_confirmation(id, "deliver")
        .unwrap();
    coordinator
        .decisions
        .answer(
            id,
            "deliver",
            None,
            &StoredCommandContext {
                human_confirmation: Some(
                    VerifiedHumanConfirmation::from_verified(
                        preview.target,
                        format!("desktop-confirmation:{}", "f".repeat(32)),
                    )
                    .unwrap(),
                ),
                caller: Some("Desktop".into()),
                channel: "gateway".into(),
                at: "2026-11-23T00:00:00Z".into(),
            },
        )
        .unwrap();
}

#[test]
fn arbiter_applied_registration_and_grant_crashes_reuse_receipts_and_keep_delivery_human() {
    const DAY: u64 = 86_400_000;
    for failure in ["before_register", "after_register", "after_grant"] {
        let a = ArbiterHarness::with_policy(|policy| {
            format!("{policy}\n[budget]\nmax_running=1\nmax_started_per_day=1\n",)
        });
        a.track(a.due);
        let attempt = a.pending_attempt();
        a.model.complete(&attempt, REPLY);
        {
            let mut state = a.h.execution.state.lock().unwrap();
            state.fail_before_register = failure == "before_register";
            state.fail_after_register = failure == "after_register";
            state.fail_after_grant = failure == "after_grant";
        }
        let (result, facts) = track_result(&a.coordinator(), a.due + 1);
        assert!(result.is_err(), "{failure}");
        let applied = events(&facts);
        assert_eq!(applied.len(), 1, "{failure}");
        assert_eq!(applied[0].outcome, IssueArbiterFinishedOutcome::Applied {});
        let approved = a.stored();
        assert_eq!(
            approved.request.answer().unwrap().by,
            IssueDecisionAnswerBy::Arbiter
        );
        let original = a.h.work().unwrap();
        let admission = admissions(&a.h);
        assert_eq!(admission.len(), 1);
        assert_eq!(admission[0].0, original.order.work_order_id);
        assert_eq!(admission[0].2, i64::try_from(a.due + 1).unwrap());
        assert_eq!(a.model.requests().len(), 1);
        assert_eq!(a.model.forgotten(), vec![attempt]);

        // Rebuild the services and model around the durable SQLite state. A new UTC
        // day must not charge an existing admission or issue another model request.
        let restarted_model = Arc::new(FakeIssueAssessmentModelAdapter::default());
        let recovered = reassemble(&a, restarted_model.clone());
        let now = a.due + DAY;
        let replay = track(&recovered, now, true);
        assert_eq!(events(&replay), applied);
        recovered.acknowledge_event(&applied[0].event_id).unwrap();
        assert!(events(&track(&recovered, now + 1, true)).is_empty());
        let work = a.h.work().unwrap();
        assert_eq!(work.order, original.order, "{failure}");
        assert_eq!(work.approved_basis, original.approved_basis, "{failure}");
        assert_eq!(
            work.start_authority,
            IssueWorkStartAuthority::TimeoutArbiter
        );
        assert_eq!(work.decision_id, original.decision_id);
        assert_eq!(a.stored(), approved);
        assert_eq!(admissions(&a.h), admission);
        assert!(restarted_model.requests().is_empty());
        assert!(restarted_model.observations().is_empty());
        {
            let state = a.h.execution.state.lock().unwrap();
            assert_eq!(state.registrations, 1, "{failure}");
            assert_eq!(state.grants.len(), 1, "{failure}");
            assert!(!state.grants[0].delivery);
            assert!(state.grants[0].delivery_review.is_none());
        }

        freeze_protected_delivery(&a.h);
        track(&recovered, now + 2, true);
        let pending = a.h.work().unwrap();
        let review = pending.delivery_assessment.as_ref().unwrap();
        assert_eq!(
            review.verdict.reasons,
            [IssueDeliveryRiskReason::ProtectedPath]
        );
        assert_eq!(
            pending.current_start_authority(),
            IssueWorkStartAuthority::TimeoutArbiter
        );
        assert_eq!(pending.approved_basis, original.approved_basis);
        assert!(!pending.delivery_granted);
        assert_eq!(
            pending.blocked_reason,
            Some(IssueWorkBlockReason::AwaitingDeliveryApproval)
        );
        let risk = delivery_request(&a.h);
        assert_eq!(
            risk.request.definition().on_timeout,
            IssueDecisionTimeout::Wait
        );
        assert_eq!(risk.request.definition().basis, review.basis);
        track(&recovered, now + 3, true);
        assert!(
            a.h.execution
                .state
                .lock()
                .unwrap()
                .grants
                .iter()
                .all(|grant| !grant.delivery)
        );
        assert!(restarted_model.requests().is_empty());

        answer_delivery(&recovered, &risk.request);
        track(&recovered, now + 4, true);
        let delivered = a.h.work().unwrap();
        assert!(delivered.delivery_granted);
        assert_eq!(delivered.order, original.order);
        assert_eq!(delivered.approved_basis, original.approved_basis);
        assert_eq!(
            delivered.current_start_authority(),
            IssueWorkStartAuthority::TimeoutArbiter
        );
        assert_eq!(a.stored(), approved);
        assert_eq!(admissions(&a.h), admission);
        let human =
            a.h.fixture
                .store()
                .get_issue_decision(&a.h.fixture.workspace, risk.request.id())
                .unwrap()
                .unwrap();
        assert_eq!(
            human.request.answer().unwrap().by,
            IssueDecisionAnswerBy::Human
        );
        let state = a.h.execution.state.lock().unwrap();
        assert_eq!(state.registrations, 1);
        let grants: Vec<_> = state.grants.iter().filter(|grant| grant.delivery).collect();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].delivery_review.as_ref(), Some(&review.basis));
        assert_eq!(a.model.requests().len(), 1);
        assert!(restarted_model.requests().is_empty());
    }
}

fn started_timeout(authority: IssueWorkStartAuthority) -> (Harness, u64) {
    if authority == IssueWorkStartAuthority::TimeoutArbiter {
        let a = ArbiterHarness::new();
        a.track(a.due);
        a.model.complete(&a.pending_attempt(), REPLY);
        a.track(a.due + 1);
        return (a.h, a.due + 1);
    }
    assert_eq!(authority, IssueWorkStartAuthority::TimeoutDefault);
    let h = Harness::new();
    *h.policy.text.lock().unwrap() =
        gated_policy().replace("start = \"auto\"", "start = \"timeout_default\"");
    enable_gated(&h);
    h.tick(true).unwrap();
    let due = chrono::DateTime::parse_from_rfc3339(
        h.decision().request.definition().due_at.as_deref().unwrap(),
    )
    .unwrap()
    .timestamp_millis() as u64;
    track(&h.coordinator(), due, true);
    (h, due)
}

#[test]
fn timeout_authorities_observe_off_facts_without_registering_validation_recovery() {
    for authority in [
        IssueWorkStartAuthority::TimeoutDefault,
        IssueWorkStartAuthority::TimeoutArbiter,
    ] {
        for classification in [
            IssueExecutionTerminalClass::ObserveAgain,
            IssueExecutionTerminalClass::QueueRecovery,
        ] {
            let (h, now) = started_timeout(authority);
            let original = h.work().unwrap();
            assert_eq!(original.current_start_authority(), authority);
            {
                let mut state = h.execution.state.lock().unwrap();
                assert!(state.control.allows_automation());
                state.observation = Some(IssueWorkExecutionObservation {
                    phase: IssueExecutionPhase::Terminal,
                    task_ids: vec!["task-1".into()],
                    active_task_id: Some("task-1".into()),
                    execution_ids: vec!["task-1".into()],
                    current_terminal_id: Some("off-terminal".into()),
                    terminals: vec![IssueExecutionTerminalRecord {
                        record_id: "off-terminal".into(),
                        classification,
                    }],
                    ..Default::default()
                });
            }
            let store = h.fixture.store();
            let collection = store
                .get_collection(&h.fixture.workspace)
                .unwrap()
                .expect("the started timeout has a collection");
            assert_eq!(collection.config.mode, IssueCollectionMode::Observe);
            let saved = store
                .set_collection_mode(
                    &h.fixture.workspace,
                    Some(collection.epoch),
                    IssueCollectionMode::Off,
                )
                .unwrap();
            let IssueCollectionModeSaveResult::Saved { state, changed, .. } = saved else {
                panic!("turning collection Off must commit: {saved:?}");
            };
            assert!(changed);
            assert_eq!(state.config.mode, IssueCollectionMode::Off);
            assert_eq!(
                store
                    .get_collection(&h.fixture.workspace)
                    .unwrap()
                    .expect("the Off collection remains stored")
                    .config
                    .mode,
                IssueCollectionMode::Off
            );
            let coordinator = h.coordinator();
            for at in [now + 1, now + 2] {
                track(&coordinator, at, true);
                let state = h.execution.state.lock().unwrap();
                assert!(
                    state.recoveries.is_empty(),
                    "{authority:?}/{classification:?}"
                );
                assert!(state.revoked, "{authority:?}/{classification:?}");
                assert_eq!(state.execution_limits.last().unwrap().1, 0);
                assert_eq!(state.registrations, 1);
                assert_eq!(state.grants.len(), 1);
            }
            let observed = h
                .fixture
                .store()
                .get_issue_execution(&h.fixture.workspace, &original.order.issue)
                .unwrap()
                .unwrap();
            assert_eq!(
                observed.progress.terminal.as_ref().unwrap().classification,
                classification
            );
            assert_eq!(
                observed.progress.terminal.as_ref().unwrap().record_id,
                "off-terminal"
            );
            assert_eq!(h.work().unwrap().order, original.order);
        }
    }
}
