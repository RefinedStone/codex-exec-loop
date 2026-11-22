use super::*;

fn review_validation(key: &str, repository: &str, number: u64) -> PrValidationRecord {
    PrValidationRecord::register(
        PrValidationRecordKey::new(key).unwrap(),
        PrValidationTarget::new(repository, number).unwrap(),
        PrValidationTargetShaSnapshot::new(
            PrValidationCommitSha::new("a".repeat(40)).unwrap(),
            PrValidationCommitSha::new("b".repeat(40)).unwrap(),
        ),
    )
    .transition(PrValidationEvent::BeginPreMergeObservation)
    .unwrap()
}

#[test]
fn review_link_uses_exact_active_lineage_target_and_ignores_superseded_records() {
    let mut superseded = evidence(
        review_validation("superseded", "old/repository", 99),
        "current",
    );
    superseded.superseded = true;
    let records = vec![
        evidence(
            review_validation("current-validation", "actual/review-target", 42),
            "current",
        ),
        evidence(
            review_validation("previous-task", "other/repository", 17),
            "previous",
        ),
        superseded,
    ];
    let observed = project(
        &order(&["previous", "current"]),
        &tasks(vec![
            task("previous", TaskStatus::Done),
            task("current", TaskStatus::Done),
        ]),
        &runtime(),
        &records,
        &[],
    )
    .unwrap();
    assert_eq!(observed.phase, IssueExecutionPhase::InReview);
    assert_eq!(observed.active_task_id.as_deref(), Some("current"));
    assert_eq!(
        observed.review_pull_request_url.as_deref(),
        Some("https://github.com/actual/review-target/pull/42")
    );
}

#[test]
fn review_link_is_not_inferred_from_a_queue_number_or_superseded_validation() {
    let mut runtime = runtime();
    runtime
        .distributor_queue_records
        .push(queue(&lease("task-1", 'a'), Some(42)));
    let mut superseded = evidence(
        review_validation("old-validation", "old/repository", 77),
        "task-1",
    );
    superseded.superseded = true;
    for records in [Vec::new(), vec![superseded]] {
        let observed = project(
            &order(&["task-1"]),
            &tasks(vec![task("task-1", TaskStatus::Done)]),
            &runtime,
            &records,
            &[],
        )
        .unwrap();
        assert_eq!(observed.phase, IssueExecutionPhase::InReview);
        assert_eq!(observed.review_pull_request_url, None);
    }
}

#[test]
fn review_link_rejects_validation_from_a_different_work_lineage() {
    let result = project(
        &order(&["task-1"]),
        &tasks(vec![task("task-1", TaskStatus::Done)]),
        &runtime(),
        &[evidence(
            review_validation("foreign", "other/repository", 42),
            "foreign-task",
        )],
        &[],
    );
    assert!(result.is_err());
}

#[test]
fn review_link_follows_the_existing_remediation_correlation() {
    let observed = project(
        &order(&["task-1", "remediation"]),
        &tasks(vec![
            task("task-1", TaskStatus::Done),
            task("remediation", TaskStatus::Done),
        ]),
        &runtime(),
        &[evidence(needs_rework(), "task-1")],
        &[],
    )
    .unwrap();
    assert_eq!(observed.active_task_id.as_deref(), Some("remediation"));
    assert_eq!(
        observed.review_pull_request_url.as_deref(),
        Some("https://github.com/sample/widgets/pull/2551")
    );
}
