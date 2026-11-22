use super::*;

fn observation(phase: IssueExecutionPhase) -> IssueWorkExecutionObservation {
    IssueWorkExecutionObservation {
        phase,
        task_ids: vec!["task-1".into()],
        active_task_id: Some("task-1".into()),
        execution_ids: vec!["task-1".into()],
        ..Default::default()
    }
}

fn terminal(
    phase: IssueExecutionPhase,
    classification: IssueExecutionTerminalClass,
) -> IssueWorkExecutionObservation {
    let record = IssueExecutionTerminalRecord {
        record_id: "validation-terminal-1".into(),
        classification,
    };
    IssueWorkExecutionObservation {
        current_terminal_id: Some(record.record_id.clone()),
        terminals: vec![record],
        ..observation(phase)
    }
}

#[test]
fn repeated_terminal_receipts_do_not_spend_another_execution_attempt() {
    let observed = terminal(
        IssueExecutionPhase::Terminal,
        IssueExecutionTerminalClass::ReworkNeeded,
    );
    let first = IssueExecutionProgress::default()
        .reconcile(&observed, 1, 0, 3)
        .unwrap();
    let repeated = first.reconcile(&observed, 0, 0, 3).unwrap();
    assert_eq!(first, repeated);
    assert_eq!(repeated.attempts.execution, 1);
    assert!(repeated.allows_additional_execution(3));
}

#[test]
fn post_merge_contract_block_never_requests_reimplementation() {
    let mut observed = terminal(
        IssueExecutionPhase::Terminal,
        IssueExecutionTerminalClass::ContractRecovery,
    );
    observed.integration = Some(IssueExecutionIntegration {
        record_id: "integration-1".into(),
        source_sha: "a".repeat(40),
        integration_sha: "b".repeat(40),
        pull_request_url: Some("https://github.com/acme/project/pull/1".into()),
    });
    let current = IssueExecutionProgress {
        attempts: IssueExecutionAttempts {
            execution: 1,
            verification: 2,
            publication: 0,
        },
        ..Default::default()
    };
    let next = current.reconcile(&observed, 0, 0, 1).unwrap();
    assert_eq!(next.state, IssueExecutionState::Done);
    assert_eq!(next.attempts, current.attempts);
    assert!(!next.allows_additional_execution(100));
    let later = next
        .reconcile(
            &terminal(
                IssueExecutionPhase::Terminal,
                IssueExecutionTerminalClass::ReworkNeeded,
            ),
            0,
            0,
            100,
        )
        .unwrap();
    assert_eq!(later.state, IssueExecutionState::Done);
    assert!(!later.allows_additional_execution(100));
}

#[test]
fn verification_recovery_has_a_separate_counter_and_does_not_exhaust_execution_budget() {
    for class in [
        IssueExecutionTerminalClass::ObserveAgain,
        IssueExecutionTerminalClass::QueueRecovery,
    ] {
        let current = IssueExecutionProgress {
            attempts: IssueExecutionAttempts {
                execution: 1,
                verification: 0,
                publication: 4,
            },
            ..Default::default()
        };
        let next = current
            .reconcile(&terminal(IssueExecutionPhase::Terminal, class), 0, 1, 1)
            .unwrap();
        assert_eq!(next.attempts.execution, 1);
        assert_eq!(next.attempts.verification, 1);
        assert_eq!(next.attempts.publication, 4);
        assert!(!next.attempts_exhausted);
        assert!(!next.allows_additional_execution(10));
    }
}

#[test]
fn execution_exhaustion_is_sticky_until_explicit_recovery() {
    let current = IssueExecutionProgress {
        attempts: IssueExecutionAttempts {
            execution: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    let failed = current
        .reconcile(
            &terminal(
                IssueExecutionPhase::Terminal,
                IssueExecutionTerminalClass::ReworkNeeded,
            ),
            1,
            0,
            3,
        )
        .unwrap();
    assert_eq!(failed.state, IssueExecutionState::Failed);
    assert!(failed.attempts_exhausted);
    let queued = failed
        .reconcile(&observation(IssueExecutionPhase::Queued), 0, 0, 10)
        .unwrap();
    assert_eq!(queued.state, IssueExecutionState::Failed);
    assert!(!queued.allows_additional_execution(10));
}

#[test]
fn historical_rework_does_not_fail_the_current_running_attempt() {
    let mut observed = terminal(
        IssueExecutionPhase::Running,
        IssueExecutionTerminalClass::ReworkNeeded,
    );
    observed.current_terminal_id = None;
    let current = IssueExecutionProgress {
        attempts: IssueExecutionAttempts {
            execution: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    let next = current.reconcile(&observed, 1, 0, 3).unwrap();
    assert_eq!(next.state, IssueExecutionState::Running);
    assert_eq!(next.attempts.execution, 3);
    assert!(!next.attempts_exhausted);
}

#[test]
fn integration_requires_evidence_and_cannot_change_identity() {
    assert!(
        observation(IssueExecutionPhase::Integrated)
            .validate()
            .is_err()
    );
    let mut observed = observation(IssueExecutionPhase::Integrated);
    observed.integration = Some(IssueExecutionIntegration {
        record_id: "integrated-1".into(),
        source_sha: "a".repeat(40),
        integration_sha: "b".repeat(40),
        pull_request_url: None,
    });
    let first = IssueExecutionProgress::default()
        .reconcile(&observed, 1, 0, 3)
        .unwrap();
    observed.integration.as_mut().unwrap().integration_sha = "c".repeat(40);
    assert!(first.reconcile(&observed, 0, 0, 3).is_err());
}

#[test]
fn an_unrelated_question_or_terminal_identity_is_rejected() {
    let mut observed = observation(IssueExecutionPhase::Running);
    observed.question = Some(IssueExecutionQuestion {
        question_id: "question-1".into(),
        task_id: "unrelated-task".into(),
        session_key: "worker-session-1".into(),
        thread_id: "thread-1".into(),
        slot_id: "slot-1".into(),
        worktree_path: "/work/slot-1".into(),
        branch_name: "work/task-1".into(),
        lease_generation: "generation-1".into(),
        awaiting_task_updated_at: "2026-11-21T00:00:00Z".into(),
    });
    assert!(observed.validate().is_err());
    observed.question = None;
    observed.current_terminal_id = Some("missing".into());
    assert!(observed.validate().is_err());
}

#[test]
fn execution_receipts_must_belong_to_the_observed_lineage() {
    let mut observed = observation(IssueExecutionPhase::Running);
    observed.execution_ids.push("unrelated-task".into());
    assert!(observed.validate().is_err());
}

#[test]
fn a_queued_supplement_does_not_reset_the_work_to_its_initial_queue() {
    let current = IssueExecutionProgress {
        state: IssueExecutionState::InReview,
        attempts: IssueExecutionAttempts {
            execution: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let next = current
        .reconcile(&observation(IssueExecutionPhase::Queued), 0, 0, 3)
        .unwrap();
    assert_eq!(next.state, IssueExecutionState::Running);
    assert_eq!(next.attempts.execution, 1);
}

#[test]
fn unavailable_policy_preserves_facts_without_inventing_budget_exhaustion() {
    let observed = terminal(
        IssueExecutionPhase::Terminal,
        IssueExecutionTerminalClass::ReworkNeeded,
    );
    let next = IssueExecutionProgress::default()
        .reconcile_with_budget(&observed, 1, 0, None)
        .unwrap();
    assert_eq!(next.attempts.execution, 1);
    assert!(!next.attempts_exhausted);
    let mut integrated = observation(IssueExecutionPhase::Integrated);
    integrated.integration = Some(IssueExecutionIntegration {
        record_id: "integration-policy-unavailable".into(),
        source_sha: "a".repeat(40),
        integration_sha: "b".repeat(40),
        pull_request_url: None,
    });
    let done = next.reconcile_with_budget(&integrated, 0, 0, None).unwrap();
    assert_eq!(done.state, IssueExecutionState::Done);
}

#[test]
fn review_link_updates_are_observed_without_spending_another_attempt() {
    let mut observed = observation(IssueExecutionPhase::InReview);
    observed.review_pull_request_url = Some("https://github.com/actual/review/pull/42".into());
    let first = IssueExecutionProgress::default()
        .reconcile(&observed, 1, 0, 3)
        .unwrap();
    assert_eq!(
        first.review_pull_request_url,
        observed.review_pull_request_url
    );
    assert_eq!(first.reconcile(&observed, 0, 0, 3).unwrap(), first);
    observed.review_pull_request_url = Some("https://github.com/actual/review/pull/43".into());
    let successor = first.reconcile(&observed, 0, 0, 3).unwrap();
    assert_eq!(successor.attempts, first.attempts);
    assert_eq!(
        successor.review_pull_request_url,
        observed.review_pull_request_url
    );
    observed.review_pull_request_url = None;
    let unavailable = successor.reconcile(&observed, 0, 0, 3).unwrap();
    assert_eq!(
        unavailable.review_pull_request_url, None,
        "an unobserved current link is not filled from a previous PR"
    );
    assert_eq!(unavailable.attempts, first.attempts);
}

#[test]
fn legacy_execution_json_without_review_link_remains_readable() {
    let record: IssueExecutionRecord = serde_json::from_value(serde_json::json!({
        "work_order_id": "legacy-work",
        "progress": {
            "state": "in_review",
            "attempts": { "execution": 2, "verification": 1, "publication": 0 },
            "terminal": null,
            "integration": null,
            "attempts_exhausted": false,
            "revision": 7
        },
        "question": null
    }))
    .unwrap();
    record.validate().unwrap();
    assert_eq!(record.progress.review_pull_request_url, None);
    assert_eq!(record.progress.attempts.execution, 2);
    let mut value = serde_json::to_value(observation(IssueExecutionPhase::InReview)).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("review_pull_request_url");
    let legacy: IssueWorkExecutionObservation = serde_json::from_value(value).unwrap();
    legacy.validate().unwrap();
    assert_eq!(legacy.review_pull_request_url, None);
}
