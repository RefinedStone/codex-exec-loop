use super::*;
use crate::domain::issue_automation::execution::*;

fn started() -> Harness {
    let h = Harness::new();
    h.tick(true).unwrap();
    h.answer("start");
    h.tick(true).unwrap();
    h
}

fn running(task: &str) -> IssueWorkExecutionObservation {
    IssueWorkExecutionObservation {
        phase: IssueExecutionPhase::Running,
        task_ids: vec![task.into()],
        active_task_id: Some(task.into()),
        execution_ids: vec![task.into()],
        ..Default::default()
    }
}

fn terminal(classification: IssueExecutionTerminalClass) -> IssueWorkExecutionObservation {
    IssueWorkExecutionObservation {
        phase: IssueExecutionPhase::Terminal,
        terminals: vec![IssueExecutionTerminalRecord {
            record_id: "terminal-1".into(),
            classification,
        }],
        current_terminal_id: Some("terminal-1".into()),
        ..running("task-1")
    }
}

fn integrated(task: &str) -> IssueWorkExecutionObservation {
    IssueWorkExecutionObservation {
        phase: IssueExecutionPhase::Integrated,
        integration: Some(IssueExecutionIntegration {
            record_id: "integration-1".into(),
            source_sha: "a".repeat(40),
            integration_sha: "b".repeat(40),
            pull_request_url: Some("https://github.com/acme/repo/pull/1".into()),
        }),
        ..running(task)
    }
}

fn track(h: &Harness, enabled: bool) -> Result<Vec<IssueCoordinatorFact>> {
    let facts = Mutex::new(Vec::new());
    h.coordinator().track_execution(
        &IssueCollectionCancellation::new(),
        123,
        enabled,
        &|values| facts.lock().unwrap().extend(values),
    )?;
    Ok(facts.into_inner().unwrap())
}

fn progress(h: &Harness) -> IssueExecutionRecord {
    h.fixture
        .store()
        .get_issue_execution(&h.fixture.workspace, &h.fixture.reference("I_001"))
        .unwrap()
        .unwrap()
}

fn recovery_decision(h: &Harness) -> StoredIssueDecision {
    page(&h.triage).items[0]
        .open_decisions
        .iter()
        .find(|decision| decision.request.definition().kind == IssueDecisionKind::FailureRecovery)
        .unwrap()
        .clone()
}

fn answer_recovery(h: &Harness, decision: &StoredIssueDecision, option: &str) {
    use crate::application::port::inbound::stored_command_port::{
        StoredCommandContext, issue_confirmation::VerifiedHumanConfirmation,
    };
    let preview = h
        .decisions
        .prepare_confirmation(decision.request.id().as_str(), option)
        .unwrap();
    let context = StoredCommandContext {
        human_confirmation: Some(
            VerifiedHumanConfirmation::from_verified(
                preview.target,
                format!("desktop-confirmation:{}", "c".repeat(32)),
            )
            .unwrap(),
        ),
        caller: Some("Desktop".into()),
        channel: "gateway".into(),
        at: "2026-11-21T00:00:00Z".into(),
    };
    h.decisions
        .answer(decision.request.id().as_str(), option, None, &context)
        .unwrap();
}

#[test]
fn collection_off_still_tracks_integration_without_remote_reads_or_registration() {
    let h = started();
    let store = h.fixture.store();
    let collection = store.get_collection(&h.fixture.workspace).unwrap().unwrap();
    store
        .set_collection_mode(
            &h.fixture.workspace,
            Some(collection.epoch),
            IssueCollectionMode::Off,
        )
        .unwrap();
    h.execution.state.lock().unwrap().observation = Some(integrated("task-1"));
    let before_reads = h.source.calls.load(Ordering::SeqCst);
    let limits_before = h.execution.state.lock().unwrap().execution_limits.len();
    let facts = track(&h, false).unwrap();
    assert_eq!(progress(&h).progress.state, IssueExecutionState::Done);
    assert_eq!(h.source.calls.load(Ordering::SeqCst), before_reads);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
    let integrated_facts: Vec<_> = facts
        .iter()
        .filter_map(|fact| match fact {
            IssueCoordinatorFact::Integrated { event_id, .. } => Some(event_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(integrated_facts.len(), 1);
    for fact in facts {
        match fact {
            IssueCoordinatorFact::Integrated { event_id, .. }
            | IssueCoordinatorFact::ExecutionProgressed { event_id, .. }
            | IssueCoordinatorFact::WorkSubmitted { event_id, .. }
            | IssueCoordinatorFact::WorkPermitGranted { event_id, .. } => {
                h.coordinator().acknowledge_event(&event_id).unwrap();
            }
            _ => {}
        }
    }
    let revision = progress(&h).progress.revision;
    assert!(
        track(&h, false)
            .unwrap()
            .iter()
            .all(|fact| !matches!(fact, IssueCoordinatorFact::Integrated { .. }))
    );
    assert_eq!(progress(&h).progress.revision, revision);
    let state = h.execution.state.lock().unwrap();
    assert_eq!(state.execution_limits.last().unwrap().1, 0);
    assert!(
        state.execution_limits[limits_before..]
            .iter()
            .all(|(_, cap)| *cap == 0),
        "an observed integration must never temporarily reopen execution before its issue record is saved"
    );
    assert!(state.recoveries.is_empty());
}

#[test]
fn observe_and_queue_recovery_count_once_without_consuming_another_execution() {
    for class in [
        IssueExecutionTerminalClass::ObserveAgain,
        IssueExecutionTerminalClass::QueueRecovery,
    ] {
        let h = started();
        h.execution.state.lock().unwrap().observation = Some(terminal(class));
        track(&h, true).unwrap();
        track(&h, true).unwrap();
        let progress = progress(&h).progress;
        assert_eq!(
            progress.attempts,
            IssueExecutionAttempts {
                execution: 1,
                verification: 1,
                publication: 0,
            }
        );
        let state = h.execution.state.lock().unwrap();
        assert_eq!(state.registrations, 1);
        assert_eq!(state.recoveries.len(), 1);
    }
}

#[test]
fn post_merge_contract_opens_human_recovery_and_never_reimplements() {
    let h = started();
    h.execution.state.lock().unwrap().observation = Some(IssueWorkExecutionObservation {
        integration: integrated("task-1").integration,
        ..terminal(IssueExecutionTerminalClass::ContractRecovery)
    });
    track(&h, true).unwrap();
    let first = recovery_decision(&h);
    track(&h, true).unwrap();
    assert_eq!(recovery_decision(&h), first);
    let progress = progress(&h).progress;
    assert_eq!(progress.state, IssueExecutionState::Done);
    assert_eq!(
        progress.attempts,
        IssueExecutionAttempts {
            execution: 1,
            verification: 0,
            publication: 0,
        }
    );
    let state = h.execution.state.lock().unwrap();
    assert_eq!(state.registrations, 1);
    assert!(state.recoveries.is_empty());
    assert_eq!(state.execution_limits.last().unwrap().1, 0);
}

#[test]
fn exhausted_execution_is_failed_with_one_recovery_decision() {
    let h = Harness::new();
    h.policy
        .text
        .lock()
        .unwrap()
        .push_str("\n[budget]\nmax_attempts = 1\n");
    h.tick(true).unwrap();
    h.answer("start");
    h.tick(true).unwrap();
    h.execution.state.lock().unwrap().observation =
        Some(terminal(IssueExecutionTerminalClass::ReworkNeeded));
    track(&h, true).unwrap();
    let first = recovery_decision(&h);
    track(&h, true).unwrap();
    assert_eq!(recovery_decision(&h), first);
    let progress = progress(&h).progress;
    assert_eq!(progress.state, IssueExecutionState::Failed);
    assert!(progress.attempts_exhausted);
    assert_eq!(progress.attempts.execution, 1);
    let state = h.execution.state.lock().unwrap();
    assert!(state.revoked);
    assert!(state.recoveries.is_empty());
    assert_eq!(state.registrations, 1);
}

#[test]
fn identity_conflict_holds_once_and_requires_human_recovery() {
    let h = started();
    h.execution.state.lock().unwrap().observation =
        Some(terminal(IssueExecutionTerminalClass::IdentityConflict));
    track(&h, true).unwrap();
    let first = recovery_decision(&h);
    let epoch = h
        .execution
        .state
        .lock()
        .unwrap()
        .control
        .authorization_epoch;
    track(&h, true).unwrap();
    assert_eq!(recovery_decision(&h), first);
    let state = h.execution.state.lock().unwrap();
    assert!(state.control.held);
    assert_eq!(state.control.authorization_epoch, epoch);
    assert_eq!(state.control_commands.len(), 1);
    assert!(state.recoveries.is_empty());
}

#[test]
fn closed_pull_request_offers_reopen_new_pull_request_and_cancel() {
    let h = started();
    h.execution.state.lock().unwrap().observation =
        Some(terminal(IssueExecutionTerminalClass::PullRequestClosed));
    track(&h, true).unwrap();
    let decision = recovery_decision(&h);
    let options: Vec<_> = decision
        .request
        .definition()
        .options
        .iter()
        .map(|option| option.id.as_str())
        .collect();
    assert!(
        options.contains(&"reopen")
            && options.contains(&"new_pull_request")
            && options.contains(&"cancel")
    );
    answer_recovery(&h, &decision, "cancel");
    track(&h, true).unwrap();
    assert!(h.execution.state.lock().unwrap().control.cancelled);
    assert_eq!(
        page(&h.triage).items[0].tracked.as_ref().unwrap().state,
        IssueAutomationState::Cancelled
    );
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
}

#[test]
fn policy_read_failure_preserves_observed_facts_without_false_exhaustion() {
    let h = started();
    h.execution.state.lock().unwrap().observation =
        Some(terminal(IssueExecutionTerminalClass::ReworkNeeded));
    h.policy.unavailable.store(true, Ordering::Release);
    assert!(track(&h, true).is_err());
    let progress = progress(&h).progress;
    assert_eq!(progress.attempts.execution, 1);
    assert!(!progress.attempts_exhausted);
    assert_eq!(progress.state, IssueExecutionState::InReview);
    let state = h.execution.state.lock().unwrap();
    assert!(state.revoked);
    assert_eq!(state.execution_limits.last().unwrap().1, 0);
    assert_eq!(state.registrations, 1);
}

#[test]
fn progress_cas_conflict_discards_old_observation_until_a_fresh_tick() {
    let h = started();
    let work = h.work().unwrap();
    h.execution.state.lock().unwrap().observation = Some(running("task-1"));
    let store = h.fixture.store();
    let workspace = h.fixture.workspace.clone();
    let execution = h.execution.clone();
    h.execution.state.lock().unwrap().after_observation = Some(Box::new(move || {
        let next = integrated("task-1");
        store
            .apply_issue_execution(&workspace, &work, 0, &next, Some(3))
            .unwrap();
        execution.state.lock().unwrap().observation = Some(next);
    }));
    assert!(track(&h, true).is_err());
    let first = progress(&h);
    assert_eq!(first.progress.state, IssueExecutionState::Done);
    track(&h, true).unwrap();
    assert_eq!(progress(&h), first);
    assert_eq!(
        h.execution.state.lock().unwrap().observed_work_orders.len(),
        2
    );
}

#[test]
fn observation_failure_does_not_skip_later_registered_work_pages() {
    let h = started();
    h.fixture.seed(
        (2..=101)
            .map(|index| h.fixture.observation(&format!("I_{index:03}")))
            .collect(),
        &disclosure_version(),
    );
    run(&h.triage);
    let last = super::policy_revalidation::seed_approved_work_checkpoint(
        &h,
        &h.fixture.reference("I_101"),
    );
    let first = h.work().unwrap();
    {
        let mut state = h.execution.state.lock().unwrap();
        state.fail_observation_work_order = Some(first.order.work_order_id);
        state.observations_by_work.insert(
            last.order.work_order_id.clone(),
            integrated("task-restored-last-page"),
        );
    }
    assert!(track(&h, true).is_err());
    let last = h
        .fixture
        .store()
        .get_issue_execution(&h.fixture.workspace, &h.fixture.reference("I_101"))
        .unwrap()
        .unwrap();
    assert_eq!(last.progress.state, IssueExecutionState::Done);
    assert_eq!(
        h.execution.state.lock().unwrap().observed_work_orders.len(),
        2
    );
}

#[test]
fn supplemental_lineage_gets_a_fresh_permit_without_a_second_registration() {
    let h = started();
    let before = h.work().unwrap();
    {
        let mut state = h.execution.state.lock().unwrap();
        state.observation = Some(IssueWorkExecutionObservation {
            phase: IssueExecutionPhase::Queued,
            task_ids: vec!["task-1".into(), "task-2".into()],
            active_task_id: Some("task-2".into()),
            execution_ids: vec!["task-1".into()],
            ..Default::default()
        });
        state.permit_floor = before.permit_revision + 1;
        state.revoked = true;
    }
    h.tick(true).unwrap();
    let after = h.work().unwrap();
    assert_eq!(after.task_ids, ["task-1", "task-2"]);
    assert!(after.permit_revision > before.permit_revision);
    assert_eq!(progress(&h).progress.attempts.execution, 1);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
}

#[test]
fn pre_dispatch_base_change_revalidates_and_uses_fresh_source() {
    let h = started();
    let before = h.work().unwrap();
    h.execution.state.lock().unwrap().dispatch_source = Some("c".repeat(40));
    let reads = h.source.calls.load(Ordering::SeqCst);
    h.tick(true).unwrap();
    let after = h.work().unwrap();
    assert_eq!(after.permit_source, Some("c".repeat(40)));
    assert!(after.permit_revision > before.permit_revision);
    assert_eq!(h.source.calls.load(Ordering::SeqCst), reads + 1);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
}

#[test]
fn failed_execution_observation_blocks_a_new_source_grant_in_the_same_tick() {
    let h = started();
    let work = h.work().unwrap();
    {
        let mut state = h.execution.state.lock().unwrap();
        state.fail_observation_work_order = Some(work.order.work_order_id);
        state.dispatch_source = Some("d".repeat(40));
    }
    let before = h.execution.state.lock().unwrap().grants.clone();
    let reads = h.source.calls.load(Ordering::SeqCst);
    assert!(h.tick(true).is_err());
    assert_eq!(h.execution.state.lock().unwrap().grants, before);
    assert_eq!(h.source.calls.load(Ordering::SeqCst), reads);
}

#[test]
fn unreadable_policy_still_fences_when_execution_observation_fails_or_is_absent() {
    for fails in [false, true] {
        let h = started();
        let work = h.work().unwrap();
        if fails {
            h.execution
                .state
                .lock()
                .unwrap()
                .fail_observation_work_order = Some(work.order.work_order_id.clone());
        }
        h.policy.unavailable.store(true, Ordering::Release);
        assert!(h.tick(true).is_err());
        let state = h.execution.state.lock().unwrap();
        assert_eq!(
            state.execution_limits.last(),
            Some(&(work.order.work_order_id.clone(), 0))
        );
        assert_eq!(state.revoked_work_orders, [work.order.work_order_id]);
    }
}

#[test]
fn an_apply_boundary_error_fences_existing_execution_after_observation() {
    let h = started();
    let work = h.work().unwrap();
    let database = h.fixture.runtime.join("issue-automation.db");
    let changed_schema = Arc::new(AtomicBool::new(false));
    let marker = changed_schema.clone();
    let trigger_database = database.clone();
    let (grants_before, revocations_before) = {
        let mut state = h.execution.state.lock().unwrap();
        state.observation = Some(running("task-1"));
        state.after_observation = Some(Box::new(move || {
            // The earlier progress read succeeds. The next store call is apply_issue_execution;
            // its exact schema guard rejects this injected trigger before any progress INSERT.
            let connection = rusqlite::Connection::open(&trigger_database).unwrap();
            connection
                .execute_batch(
                    "CREATE TRIGGER reject_execution_progress_insert
                     BEFORE INSERT ON issue_execution_progress
                     BEGIN SELECT RAISE(ABORT, 'execution write rejected by fixture'); END;",
                )
                .unwrap();
            marker.store(true, Ordering::Release);
        }));
        (state.grants.len(), state.revoked_work_orders.len())
    };

    let error = track(&h, true).unwrap_err();
    assert!(changed_schema.load(Ordering::Acquire));
    assert!(format!("{error:#}").contains("schema"));
    {
        let state = h.execution.state.lock().unwrap();
        assert_eq!(
            state.execution_limits.last(),
            Some(&(work.order.work_order_id.clone(), 0))
        );
        assert!(state.revoked);
        assert!(
            state.revoked_work_orders[revocations_before..].contains(&work.order.work_order_id)
        );
        assert_eq!(state.grants.len(), grants_before);
    }
    let connection = rusqlite::Connection::open(database).unwrap();
    connection
        .execute_batch("DROP TRIGGER reject_execution_progress_insert;")
        .unwrap();
    drop(connection);
    assert!(
        h.fixture
            .store()
            .get_issue_execution(&h.fixture.workspace, &work.order.issue)
            .unwrap()
            .is_none()
    );
}

#[test]
fn human_recovery_keeps_fresh_source_permits_blocked_through_the_full_tick() {
    for classification in [
        IssueExecutionTerminalClass::PullRequestClosed,
        IssueExecutionTerminalClass::ContractRecovery,
        IssueExecutionTerminalClass::ReworkNeeded,
    ] {
        let h = Harness::new();
        if classification == IssueExecutionTerminalClass::ReworkNeeded {
            h.policy
                .text
                .lock()
                .unwrap()
                .push_str("\n[budget]\nmax_attempts = 1\n");
        }
        h.tick(true).unwrap();
        h.answer("start");
        h.tick(true).unwrap();
        let grants_before = {
            let mut state = h.execution.state.lock().unwrap();
            state.frozen = Some("b".repeat(40));
            state.observation = Some(terminal(classification));
            state.grants.len()
        };

        h.tick(true).unwrap();
        let decision = recovery_decision(&h);
        assert_eq!(
            decision.request.state(),
            crate::domain::issue_automation::decision::IssueDecisionState::Open
        );
        assert!(progress(&h).progress.integration.is_none());
        if classification == IssueExecutionTerminalClass::ReworkNeeded {
            assert!(progress(&h).progress.attempts_exhausted);
        }
        for _ in 0..2 {
            {
                let state = h.execution.state.lock().unwrap();
                assert!(state.revoked, "{classification:?}");
                assert_eq!(state.grants.len(), grants_before, "{classification:?}");
                assert_eq!(
                    state.execution_limits.last().unwrap().1,
                    0,
                    "{classification:?}"
                );
                assert!(state.recoveries.is_empty());
                assert_eq!(state.registrations, 1);
            }
            h.tick(true).unwrap();
            assert_eq!(recovery_decision(&h), decision);
        }
        let state = h.execution.state.lock().unwrap();
        assert!(state.revoked);
        assert_eq!(state.grants.len(), grants_before);
        assert_eq!(state.execution_limits.last().unwrap().1, 0);
    }
}

#[test]
fn observe_and_queue_recovery_remain_enabled_in_the_full_tick() {
    for classification in [
        IssueExecutionTerminalClass::ObserveAgain,
        IssueExecutionTerminalClass::QueueRecovery,
    ] {
        let h = started();
        h.execution.state.lock().unwrap().observation = Some(terminal(classification));
        let grants_before = h.execution.state.lock().unwrap().grants.len();
        h.tick(true).unwrap();
        h.tick(true).unwrap();
        let state = h.execution.state.lock().unwrap();
        assert_eq!(state.recoveries.len(), 1, "{classification:?}");
        assert!(!state.revoked, "{classification:?}");
        assert!(
            state.execution_limits.last().unwrap().1 > 0,
            "{classification:?}"
        );
        assert_eq!(state.grants.len(), grants_before);
        assert_eq!(state.registrations, 1);
        drop(state);
        assert_eq!(progress(&h).progress.attempts.execution, 1);
        assert_eq!(progress(&h).progress.attempts.verification, 1);
    }
}
