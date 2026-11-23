use super::*;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeLiveSessionDetailDefaults,
};
use crate::domain::planning::TaskActor;

mod durability;
mod fences;
mod session;

fn fixture(label: &str) -> HostFixture {
    let host = HostFixture::new(label);
    let mut detail = ParallelModeAgentSessionDetailSnapshot::assigned_for_lease(
        &host.base.lease,
        ParallelModeLiveSessionDetailDefaults {
            validation_summary: "the host check has not finished",
            authority_refresh_outcome: "official completion has not started",
        },
    );
    detail.thread_id = Some("thread-host-verification".into());
    detail.state_label = "running".into();
    host.base
        .adapter
        .upsert_runtime_session_detail(&host.base.workspace, &detail)
        .unwrap();
    host
}

fn session_detail(f: &HostFixture) -> ParallelModeAgentSessionDetailSnapshot {
    let mut runtime =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&f.base.workspace).unwrap();
    assert_eq!(runtime.session_details.len(), 1);
    runtime.session_details.remove(0)
}

fn observation(outcome: TaskVerificationExecutionOutcome) -> TaskVerificationResult {
    let mut value = executed();
    let TaskVerificationResult::Executed { observation } = &mut value else {
        unreachable!()
    };
    observation.outcome = outcome;
    value
}

fn finish_result(
    f: &HostFixture,
    id: &str,
    result: TaskVerificationResult,
    sealed: bool,
) -> TaskVerificationReceipt {
    let claim = f.claim(id);
    let bind = f.bind(&claim);
    let mut finish = f.finish(&bind);
    finish.result = result;
    if !sealed {
        finish.artifact = None;
    }
    f.base
        .adapter
        .finish_host_verification(&f.base.workspace, &finish)
        .unwrap()
        .value
}

fn fail(f: &HostFixture, id: &str) -> TaskVerificationReceipt {
    finish_result(
        f,
        id,
        observation(TaskVerificationExecutionOutcome::Exited { exit_code: 2 }),
        true,
    )
}

fn three_failures(f: &HostFixture) -> [TaskVerificationReceipt; 3] {
    std::array::from_fn(|index| fail(f, &format!("checker-failure-{index}")))
}

fn request(
    f: &HostFixture,
    receipt: &TaskVerificationReceipt,
) -> TaskVerificationFailureLimitCommit {
    TaskVerificationFailureLimitCommit {
        attempt_id: receipt.attempt.attempt_id.clone(),
        expected_lease: f.base.lease.clone(),
        owner_token: f.owner.clone(),
        at: NOW.into(),
    }
}

fn apply(f: &HostFixture, receipt: &TaskVerificationReceipt) -> TaskVerificationFailureLimitResult {
    f.base
        .adapter
        .apply_host_verification_failure_limit(&f.base.workspace, &request(f, receipt))
        .unwrap()
}

fn task(f: &HostFixture) -> TaskDefinition {
    f.base.snapshot().tasks[0].task.clone()
}

fn planning_revision(f: &HostFixture) -> i64 {
    f.base
        .adapter
        .load_task_authority_snapshot(&f.base.workspace)
        .unwrap()
        .unwrap()
        .planning_revision
}

fn marker(f: &HostFixture, receipt: &TaskVerificationReceipt) -> Option<String> {
    authority_connection(&f.base.workspace)
        .query_row(
            "SELECT failure_limit_application_json FROM task_verification_runs WHERE attempt_id=?1",
            [&receipt.attempt.attempt_id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn automatic_failure_limit_changes_only_the_third_unique_checker_failure() {
    let f = fixture("host-failure-limit-three");
    let initial_revision = planning_revision(&f);
    for count in 1..=3 {
        let receipt = fail(&f, &format!("failure-{count}"));
        let before = f.base.counters();
        let events = f
            .base
            .adapter
            .pending_events(&f.base.workspace, 100)
            .unwrap();
        let result = apply(&f, &receipt);
        assert_eq!(result.failure_count, count);
        if count < 3 {
            assert_eq!(
                result.outcome,
                TaskVerificationFailureLimitOutcome::BelowLimit
            );
            assert!(!result.changed);
            assert_ne!(task(&f).status, TaskStatus::AwaitingUser);
            assert_eq!(planning_revision(&f), initial_revision);
        } else {
            assert_eq!(
                result.outcome,
                TaskVerificationFailureLimitOutcome::AwaitingUser
            );
            assert!(result.changed);
            assert_eq!(result.planning_revision, Some(initial_revision + 1));
            assert_eq!(task(&f).status, TaskStatus::AwaitingUser);
            assert_eq!(task(&f).last_updated_by, TaskActor::System);
            assert!(
                f.base
                    .adapter
                    .worker_task_requires_preservation(&f.base.workspace, &f.base.lease.task_id)
                    .unwrap()
            );
            let waiting = session_detail(&f);
            assert_eq!(waiting.state_label, "awaiting_user");
            assert_eq!(waiting.completion_state_label, "awaiting_user");
            assert!(
                f.base
                    .adapter
                    .preserve_worker_awaiting_user(&f.base.workspace, &f.base.lease)
                    .unwrap()
            );
            assert_eq!(
                session_detail(&f),
                waiting,
                "the caller preservation helper must already be a no-op"
            );
        }
        assert_eq!(
            f.base.counters(),
            before,
            "policy must not alter evidence revisions or quotas"
        );
        assert_eq!(
            f.base
                .adapter
                .pending_events(&f.base.workspace, 100)
                .unwrap(),
            events
        );
        assert_eq!(
            f.base
                .adapter
                .load_host_verification(&f.base.workspace, &receipt.attempt.attempt_id)
                .unwrap()
                .unwrap()
                .receipt
                .as_ref(),
            Some(&receipt)
        );
        assert!(marker(&f, &receipt).is_some());
    }
}

#[test]
fn manual_host_failures_do_not_pause_without_the_explicit_automatic_policy() {
    let f = fixture("host-failure-manual");
    let receipts = three_failures(&f);
    assert_ne!(task(&f).status, TaskStatus::AwaitingUser);
    assert!(receipts.iter().all(|receipt| marker(&f, receipt).is_none()));
    for receipt in &receipts {
        f.base
            .adapter
            .acknowledge_event(&f.base.workspace, &receipt.event_id)
            .unwrap();
    }
    assert_ne!(task(&f).status, TaskStatus::AwaitingUser);
    let result = apply(&f, &receipts[2]);
    assert_eq!(
        result.failure_count, 3,
        "ACK must not erase or recount a terminal fact"
    );
    assert!(result.changed);
}

#[test]
fn automatic_failure_limit_never_counts_infrastructure_or_incomplete_results() {
    let f = fixture("host-failure-classification");
    fail(&f, "known-1");
    fail(&f, "known-2");
    let reported = f.base.prepare("reported-failure");
    let mut reported_commit = f.base.record_commit(&reported);
    reported_commit.status = TaskEvidenceStatus::Fail;
    f.base
        .adapter
        .record_evidence(&f.base.workspace, &reported_commit)
        .unwrap();
    let outcomes = [
        TaskVerificationExecutionOutcome::Exited { exit_code: 0 },
        TaskVerificationExecutionOutcome::Exited { exit_code: 1 },
        TaskVerificationExecutionOutcome::Exited { exit_code: 128 },
        TaskVerificationExecutionOutcome::TimedOut {},
        TaskVerificationExecutionOutcome::Cancelled {},
        TaskVerificationExecutionOutcome::OutputLimit {},
        TaskVerificationExecutionOutcome::Signalled {},
        TaskVerificationExecutionOutcome::IoFailure {},
    ];
    for (index, outcome) in outcomes.into_iter().enumerate() {
        let receipt = finish_result(
            &f,
            &format!("inconclusive-{index}"),
            observation(outcome),
            true,
        );
        assert_eq!(
            apply(&f, &receipt).outcome,
            TaskVerificationFailureLimitOutcome::NotFailed
        );
    }
    for stderr in [false, true] {
        let mut incomplete = observation(TaskVerificationExecutionOutcome::Exited { exit_code: 2 });
        let TaskVerificationResult::Executed { observation } = &mut incomplete else {
            unreachable!()
        };
        if stderr {
            observation.stderr.complete = false;
        } else {
            observation.stdout.complete = false;
        }
        let receipt = finish_result(&f, &format!("incomplete-{stderr}"), incomplete, true);
        assert_eq!(
            apply(&f, &receipt).outcome,
            TaskVerificationFailureLimitOutcome::NotFailed
        );
    }
    for (id, result) in [
        ("interrupted", TaskVerificationResult::Interrupted {}),
        (
            "not-run",
            TaskVerificationResult::NotRun {
                reason: TaskVerificationNotRunReason::Cancelled,
            },
        ),
        (
            "missing-seal",
            observation(TaskVerificationExecutionOutcome::Exited { exit_code: 2 }),
        ),
    ] {
        let receipt = finish_result(&f, id, result, false);
        assert_eq!(
            apply(&f, &receipt).outcome,
            TaskVerificationFailureLimitOutcome::NotFailed
        );
    }
    assert_ne!(task(&f).status, TaskStatus::AwaitingUser);
    let third = fail(&f, "known-3");
    let result = apply(&f, &third);
    assert_eq!(result.failure_count, 3);
    assert_eq!(
        result.outcome,
        TaskVerificationFailureLimitOutcome::AwaitingUser
    );
}

#[test]
fn automatic_failure_limit_is_per_criterion_and_not_the_shared_attempt_ordinal() {
    let f = fixture("host-failure-criterion");
    fail(&f, "first-criterion-1");
    fail(&f, "first-criterion-2");
    for index in 1..=2 {
        let claim = f
            .base
            .adapter
            .claim_host_verification(
                &f.base.workspace,
                &TaskVerificationClaimCommit {
                    criterion_id: "criterion two".into(),
                    ..f.request(&format!("second-criterion-{index}"))
                },
            )
            .unwrap();
        let bind = f.bind(&claim);
        let mut finish = f.finish(&bind);
        finish.result = observation(TaskVerificationExecutionOutcome::Exited { exit_code: 2 });
        let receipt = f
            .base
            .adapter
            .finish_host_verification(&f.base.workspace, &finish)
            .unwrap()
            .value;
        let result = apply(&f, &receipt);
        assert_eq!(result.failure_count, index);
        assert_eq!(
            result.outcome,
            TaskVerificationFailureLimitOutcome::BelowLimit
        );
    }
    let third = fail(&f, "first-criterion-3");
    assert_eq!(apply(&f, &third).failure_count, 3);
    assert_eq!(task(&f).status, TaskStatus::AwaitingUser);
}
