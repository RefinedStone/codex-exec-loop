use super::*;
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkLinkReference;

fn queue_record(
    fixture: &WorkPermitDeliveryFixture,
) -> crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorQueueRecord{
    // Windows does not persist the optional filesystem mirror; delivery reads SQLite authority.
    let mut records =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&fixture.repo.workspace_dir())
            .expect("distributor authority should load")
            .distributor_queue_records;
    assert_eq!(
        records.len(),
        1,
        "the work order must have one durable queue record"
    );
    records.pop().unwrap()
}

fn reference(close_on_merge: bool) -> PlanningWorkLinkReference {
    PlanningWorkLinkReference {
        url: "https://github.com/RefinedStone/codex-exec-loop/issues/2553".into(),
        close_on_merge,
    }
}

fn grant_delivery(fixture: &WorkPermitDeliveryFixture) {
    fixture.grant(
        &[
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
            PlanningWorkPermitStage::Integrate,
        ],
        true,
    );
}

#[test]
fn provider_link_rejects_a_nondefault_base_before_task_admission() {
    let repo = TempGitRepo::new("issue-link-base-preflight");
    let github = FakeGithubAutomationPort::ready();
    let service = test_parallel_mode_service_with_github(Arc::new(github.clone()));
    assert_eq!(
        service
            .validate_work_link_reference(&repo.workspace_dir(), &reference(true))
            .unwrap_err(),
        "close.when: provider_link_requires_default_branch",
    );
    service
        .validate_work_link_reference(&repo.workspace_dir(), &reference(false))
        .unwrap();
    assert!(github.operations.lock().unwrap().is_empty());
}

#[test]
fn issue_reference_survives_lineage_and_queue_without_an_implicit_close() {
    let link = reference(false);
    let fixture = WorkPermitDeliveryFixture::with_link_reference(
        "issue-link-reference-delivery",
        FakeGithubAutomationPort::ready(),
        Some(&link),
    );
    let queue = queue_record(&fixture);
    assert_eq!(queue.link_reference.as_ref(), Some(&link));
    let loaded = SqlitePlanningAuthorityAdapter::new()
        .load_work_order_for_task(&fixture.repo.workspace_dir(), TASK_ID)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.link_reference.as_ref(), Some(&link));
    grant_delivery(&fixture);
    fixture.process();
    let bodies = fixture.github.pull_request_bodies.lock().unwrap();
    assert_eq!(bodies.len(), 1);
    assert!(bodies[0].ends_with(&format!("Issue: {}", link.url)));
    assert!(!bodies[0].contains("Closes "));
}

#[test]
fn provider_default_branch_drift_blocks_source_push_and_pr_creation() {
    let github = FakeGithubAutomationPort::ready();
    *github.default_branch.lock().unwrap() = POOL_BASELINE_BRANCH.into();
    let fixture = WorkPermitDeliveryFixture::with_link_reference(
        "issue-link-default-drift",
        github,
        Some(&reference(true)),
    );
    grant_delivery(&fixture);
    let before = remote_heads(&fixture.repo);
    *fixture.github.default_branch.lock().unwrap() = "main".into();
    let notices = fixture.process();
    let record = queue_record(&fixture);
    assert_eq!(record.queue_state, ParallelModeQueueItemState::Blocked);
    assert!(
        record
            .integration_note
            .contains("provider_link_requires_default_branch"),
        "unexpected block reason: {}; notices: {notices:?}",
        record.integration_note,
    );
    assert_eq!(remote_heads(&fixture.repo), before);
    assert!(
        !fixture
            .operations()
            .iter()
            .any(|operation| operation.starts_with("push:") || operation.starts_with("ensure-pr:"))
    );
    assert!(
        fixture
            .github
            .pull_request_bodies
            .lock()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn provider_link_uses_closing_keyword_only_for_the_verified_default_base() {
    let github = FakeGithubAutomationPort::ready();
    *github.default_branch.lock().unwrap() = POOL_BASELINE_BRANCH.into();
    let link = reference(true);
    let fixture = WorkPermitDeliveryFixture::with_link_reference(
        "issue-link-default-delivery",
        github,
        Some(&link),
    );
    grant_delivery(&fixture);
    fixture.process();
    let bodies = fixture.github.pull_request_bodies.lock().unwrap();
    assert_eq!(bodies.len(), 1);
    assert!(bodies[0].ends_with(&format!("Closes {}", link.url)));
}

#[test]
fn unrelated_task_pull_request_body_keeps_the_legacy_format() {
    let fixture =
        WorkPermitDeliveryFixture::new("issue-link-legacy-body", FakeGithubAutomationPort::ready());
    let record = queue_record(&fixture);
    assert!(record.link_reference.is_none());
    let expected = format!(
        "Automated distributor delivery for a supersession result.\n\n- Agent: {}\n- Task ID: {}\n- Branch: `{}`\n- Commit: `{}`\n- Validation: {}\n- Official refresh: {}",
        record.agent_id,
        record.task_id,
        record.effective_source_branch(),
        record.effective_source_commit_sha(),
        record.validation_summary.trim(),
        record.authority_refresh_outcome.trim(),
    );
    grant_delivery(&fixture);
    fixture.process();
    assert_eq!(
        *fixture.github.pull_request_bodies.lock().unwrap(),
        vec![expected]
    );
}
