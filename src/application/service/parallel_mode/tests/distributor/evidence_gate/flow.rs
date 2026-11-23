use super::*;

#[test]
fn observe_audits_missing_criteria_and_legacy_raw_source_without_blocking_delivery() {
    let fixture = Fixture::new(
        "evidence-observe-legacy",
        TaskEvidenceDeliveryMode::Observe,
        false,
    );
    fixture.enqueue();
    let current = fixture.queue();
    assert!(current.evidence_binding.is_some());
    let frozen = current.effective_source_commit_sha();
    fixture.install_pre_evidence_queue();
    let legacy = fixture.queue();
    assert!(legacy.evidence_binding.is_none());
    assert!(legacy.source_commit_sha.is_empty());
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &fixture.repo.workspace_dir(),
        &legacy,
    )
    .unwrap();
    assert!(fixture.queue().evidence_binding.is_none());
    assert_eq!(fixture.queue().effective_source_commit_sha(), frozen);

    fixture.process();

    let calls = fixture.gate.calls.lock().unwrap();
    for stage in [
        TaskEvidenceDeliveryStage::Push,
        TaskEvidenceDeliveryStage::PullRequest,
        TaskEvidenceDeliveryStage::Integrate,
    ] {
        let call = calls
            .iter()
            .find(|call| call.effect && call.stage == stage)
            .unwrap();
        assert!(call.allowed);
        let audit = call
            .queue
            .evidence_observations
            .iter()
            .find(|audit| audit.stage == stage)
            .unwrap();
        assert_eq!(audit.mode, TaskEvidenceDeliveryMode::Observe);
        assert!(
            audit
                .reasons
                .contains(&TaskDeliveryEvidenceReason::CriteriaMissing)
        );
        assert!(
            audit
                .reasons
                .contains(&TaskDeliveryEvidenceReason::EvidenceStale)
        );
        assert!(audit.source.is_none(), "a legacy fallback is not evidence");
    }
    assert!(
        fixture
            .operations()
            .iter()
            .any(|operation| operation.starts_with("ensure-pr:"))
    );
    assert_eq!(
        fixture.remote_ref(POOL_BASELINE_BRANCH).as_deref(),
        calls
            .iter()
            .find(|call| call.stage == TaskEvidenceDeliveryStage::Integrate)
            .unwrap()
            .queue
            .integration_commit_sha
            .as_deref()
    );
}

#[test]
fn enforce_requires_host_evidence_even_when_work_order_permission_is_legacy() {
    for (accepted, report, reason) in [
        (false, None, TaskDeliveryEvidenceReason::CriteriaMissing),
        (true, None, TaskDeliveryEvidenceReason::EvidenceMissing),
        (
            true,
            Some(TaskEvidenceStatus::Pass),
            TaskDeliveryEvidenceReason::EvidenceMissing,
        ),
        (
            true,
            Some(TaskEvidenceStatus::Fail),
            TaskDeliveryEvidenceReason::CriteriaNotMet,
        ),
        (
            true,
            Some(TaskEvidenceStatus::Blocked),
            TaskDeliveryEvidenceReason::CriteriaNotMet,
        ),
    ] {
        let fixture = Fixture::new(
            "evidence-enforce-reported",
            TaskEvidenceDeliveryMode::Enforce,
            accepted,
        );
        if let Some(status) = report {
            fixture.record_worker(status);
        }
        fixture.enqueue();
        let queue = fixture.queue();
        use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
        use crate::application::port::outbound::planning_work_order_repository_port::{
            PlanningWorkPermitDecision, PlanningWorkPermitStage,
        };
        assert_eq!(
            fixture
                .authority
                .check_work_order_permit_for_task(
                    &fixture.repo.workspace_dir(),
                    TASK,
                    PlanningWorkPermitStage::Push,
                    Some(&queue.effective_source_commit_sha()),
                )
                .unwrap(),
            PlanningWorkPermitDecision::Legacy
        );

        fixture.process();
        fixture.assert_no_source_or_pr();
        fixture.assert_blocked(reason, TaskEvidenceDeliveryStage::Push);
        let before = fixture.queue();
        fixture.process();
        fixture.assert_no_source_or_pr();
        assert_eq!(
            fixture.queue(),
            before,
            "unchanged waits must not churn the durable audit"
        );
    }
}

#[test]
fn failed_or_stale_real_host_results_never_publish_source_or_create_a_pull_request() {
    for stale in [false, true] {
        let fixture = Fixture::new(
            "evidence-host-not-current",
            TaskEvidenceDeliveryMode::Enforce,
            true,
        );
        if stale {
            fixture.verify_host(true);
            fs::write(
                Path::new(&fixture.lease.worktree_path).join("feature.txt"),
                "a different tree after the host result\n",
            )
            .unwrap();
        } else {
            fs::write(
                Path::new(&fixture.lease.worktree_path).join("feature.txt"),
                "trailing whitespace  \n",
            )
            .unwrap();
            fixture.verify_host(false);
        }
        fixture.enqueue();
        fixture.process();
        fixture.assert_no_source_or_pr();
        fixture.assert_blocked(
            if stale {
                TaskDeliveryEvidenceReason::EvidenceStale
            } else {
                TaskDeliveryEvidenceReason::CriteriaNotMet
            },
            TaskEvidenceDeliveryStage::Push,
        );
    }
}

#[test]
fn a_real_host_pass_survives_host_commit_and_delivers_only_the_verified_tree() {
    let fixture = Fixture::new(
        "evidence-host-pass",
        TaskEvidenceDeliveryMode::Enforce,
        true,
    );
    fixture.verify_host(true);
    fixture.enqueue();
    let source = fixture.queue();
    assert!(source.evidence_binding.is_some());
    fixture.process();

    let calls = fixture.gate.calls.lock().unwrap();
    for stage in [
        TaskEvidenceDeliveryStage::Push,
        TaskEvidenceDeliveryStage::PullRequest,
        TaskEvidenceDeliveryStage::Integrate,
    ] {
        let call = calls
            .iter()
            .find(|call| call.effect && call.stage == stage)
            .unwrap();
        assert!(call.allowed);
        let audit = call
            .queue
            .evidence_observations
            .iter()
            .find(|audit| audit.stage == stage)
            .unwrap();
        assert!(audit.satisfied());
        assert_eq!(
            audit.criteria[0].state,
            TaskDeliveryCriterionState::HostPassed
        );
        assert_eq!(
            audit.source.as_ref().unwrap().source_commit_sha,
            source.source_commit_sha
        );
    }
    let integration = calls
        .iter()
        .find(|call| call.stage == TaskEvidenceDeliveryStage::Integrate)
        .unwrap();
    let proof = integration
        .queue
        .evidence_observations
        .iter()
        .find(|audit| audit.stage == TaskEvidenceDeliveryStage::Integrate)
        .unwrap()
        .source
        .as_ref()
        .unwrap();
    let result = proof.integration.as_ref().unwrap();
    assert_eq!(result.base_commit_sha, proof.base_commit_sha);
    assert_eq!(result.result_tree_sha, proof.source_tree_sha);
    assert_eq!(
        fixture.remote_ref(POOL_BASELINE_BRANCH).as_deref(),
        Some(result.result_commit_sha.as_str())
    );
    assert!(
        fixture
            .operations()
            .iter()
            .any(|operation| operation.starts_with("push-integration:"))
    );
}

#[test]
fn an_exact_human_waiver_rearms_only_the_typed_evidence_wait() {
    let fixture = Fixture::new(
        "evidence-waiver-rearm",
        TaskEvidenceDeliveryMode::Enforce,
        true,
    );
    fixture.enqueue();
    fixture.process();
    fixture.assert_no_source_or_pr();
    fixture.assert_blocked(
        TaskDeliveryEvidenceReason::EvidenceMissing,
        TaskEvidenceDeliveryStage::Push,
    );
    fixture.waive();
    fixture.process();

    let calls = fixture.gate.calls.lock().unwrap();
    assert!(calls.iter().any(|call| !call.effect
        && !call.allowed
        && call.queue.queue_state == ParallelModeQueueItemState::Queued
        && call.queue.evidence_wait.is_none()));
    let integration = calls
        .iter()
        .find(|call| call.effect && call.stage == TaskEvidenceDeliveryStage::Integrate)
        .unwrap();
    assert!(integration.allowed);
    assert_eq!(
        integration
            .queue
            .evidence_observations
            .iter()
            .find(|audit| audit.stage == TaskEvidenceDeliveryStage::Integrate)
            .unwrap()
            .criteria[0]
            .state,
        TaskDeliveryCriterionState::Waived
    );
    assert_eq!(
        fixture.remote_ref(POOL_BASELINE_BRANCH),
        integration.queue.integration_commit_sha
    );
}

#[test]
fn an_operator_block_replaces_evidence_wait_and_is_never_rearmed_by_a_new_waiver() {
    let fixture = Fixture::new(
        "evidence-operator-hold",
        TaskEvidenceDeliveryMode::Enforce,
        true,
    );
    fixture.enqueue();
    fixture.process();
    let mut blocked = fixture.queue();
    blocked.integration_note = "operator hold; manual inspection is required".into();
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &fixture.repo.workspace_dir(),
        &blocked,
    )
    .unwrap();
    assert!(fixture.queue().evidence_wait.is_none());
    fixture.waive();
    let calls = fixture.gate.calls.lock().unwrap().len();
    fixture.process();
    fixture.assert_no_source_or_pr();
    assert_eq!(fixture.gate.calls.lock().unwrap().len(), calls);
    assert_eq!(fixture.queue().integration_note, blocked.integration_note);
}

#[test]
fn changing_acceptance_between_delivery_stages_blocks_the_next_effect() {
    for (boundary, stage) in [
        (
            FakeGithubAutomationBoundary::SourcePushed,
            TaskEvidenceDeliveryStage::PullRequest,
        ),
        (
            FakeGithubAutomationBoundary::PullRequestEnsured,
            TaskEvidenceDeliveryStage::Integrate,
        ),
    ] {
        let fixture = Fixture::new(
            "evidence-stage-change",
            TaskEvidenceDeliveryMode::Enforce,
            true,
        );
        fixture.waive();
        fixture.enqueue();
        let workspace = fixture.repo.workspace_dir();
        fixture
            .github
            .set_boundary_hook(boundary, move || change_acceptance(&workspace));
        fixture.process();
        fixture.assert_blocked(TaskDeliveryEvidenceReason::EvidenceStale, stage);
        assert_eq!(
            fixture.remote_ref(&fixture.lease.branch_name).as_deref(),
            Some(fixture.queue().source_commit_sha.as_str())
        );
        assert!(
            fixture
                .operations()
                .iter()
                .all(|operation| !operation.starts_with("push-integration:"))
        );
        if stage == TaskEvidenceDeliveryStage::PullRequest {
            assert!(
                fixture
                    .operations()
                    .iter()
                    .all(|operation| !operation.starts_with("ensure-pr:"))
            );
        }
    }
}

#[test]
fn integration_head_drift_after_the_gate_does_not_push_an_unverified_oid() {
    let fixture = Fixture::new(
        "evidence-integration-oid-drift",
        TaskEvidenceDeliveryMode::Enforce,
        true,
    );
    fixture.waive();
    fixture.enqueue();
    let pool_root = fixture.repo.pool_root();
    *fixture.gate.after.lock().unwrap() = Some((
        TaskEvidenceDeliveryStage::Integrate,
        Box::new(move |record| {
            let target = record.delivery_target.as_ref().unwrap();
            let path = derive_integration_worktree_path(
                &pool_root,
                &target.push_remote,
                &target.github_repository,
                &target.integration_branch,
            );
            fs::write(
                path.join("UNVERIFIED.txt"),
                "changed after the final evidence read\n",
            )
            .unwrap();
            run_git(&path, &["add", "UNVERIFIED.txt"]);
            run_git(&path, &["commit", "-qm", "unverified concurrent change"]);
        }),
    ));
    fixture.process();
    assert!(
        fixture
            .queue()
            .integration_note
            .contains("changed after host verification")
    );
    assert_eq!(
        fixture.remote_ref(POOL_BASELINE_BRANCH).as_deref(),
        Some(
            fixture
                .lease
                .delivery_target
                .as_ref()
                .unwrap()
                .integration_base_commit_sha
                .as_str()
        )
    );
    assert!(
        fixture
            .operations()
            .iter()
            .all(|operation| !operation.starts_with("push-integration:"))
    );
}

#[test]
fn already_published_exact_result_can_be_cleaned_without_retroactive_evidence_permission() {
    let fixture = Fixture::new(
        "evidence-cleanup-past-effect",
        TaskEvidenceDeliveryMode::Enforce,
        true,
    );
    fixture.waive();
    fixture.enqueue();
    let workspace = fixture.repo.workspace_dir();
    fixture.github.set_boundary_hook(
        FakeGithubAutomationBoundary::PullRequestEnsured,
        move || change_acceptance(&workspace),
    );
    fixture.process();
    fixture.assert_blocked(
        TaskDeliveryEvidenceReason::EvidenceStale,
        TaskEvidenceDeliveryStage::Integrate,
    );
    let row = fixture.queue();
    let result = row.integration_commit_sha.as_ref().unwrap();
    run_git(
        &fixture.repo.repo_root,
        &[
            "push",
            "origin",
            &format!("{result}:refs/heads/{POOL_BASELINE_BRANCH}"),
        ],
    );
    let call_count = fixture.gate.calls.lock().unwrap().len();

    fixture.process();

    assert_eq!(
        fixture.gate.calls.lock().unwrap().len(),
        call_count,
        "remote proof of an old effect is cleanup, not a new delivery authorization"
    );
    assert_eq!(
        fixture.remote_ref(POOL_BASELINE_BRANCH).as_ref(),
        Some(result)
    );
    assert!(
        fixture
            .operations()
            .iter()
            .all(|operation| !operation.starts_with("push-integration:"))
    );
    assert_eq!(
        fixture.queue().queue_state,
        ParallelModeQueueItemState::Done
    );
}

#[test]
fn successful_evidence_mirroring_occurs_after_its_effect_and_cannot_bypass_the_next_gate() {
    let fixture = Fixture::new(
        "evidence-mirror-race",
        TaskEvidenceDeliveryMode::Enforce,
        true,
    );
    fixture.waive();
    fixture.enqueue();
    let source = fixture.queue().source_commit_sha;
    let workspace = fixture.repo.workspace_dir();
    let remote = fixture.repo.root.join("origin.git");
    let branch = fixture.lease.branch_name.clone();
    let observed = Arc::new(Mutex::new(None));
    let captured = observed.clone();
    let _hook = crate::application::service::parallel_mode::distributor::install_before_evidence_mirror_hook(
        TaskEvidenceDeliveryStage::Push,
        move |_| {
            let already_pushed = run_command("git", ["-C", remote.to_str().unwrap(),
                "rev-parse", "--verify", &format!("refs/heads/{branch}")], None);
            *captured.lock().unwrap() = Some(already_pushed);
            change_acceptance(&workspace);
        },
    );

    fixture.process();

    assert_eq!(
        *observed.lock().unwrap(),
        Some(Some(source)),
        "the successful audit must not introduce a mutable mirror boundary before source push"
    );
    fixture.assert_blocked(
        TaskDeliveryEvidenceReason::EvidenceStale,
        TaskEvidenceDeliveryStage::PullRequest,
    );
    assert!(
        fixture
            .operations()
            .iter()
            .all(|operation| !operation.starts_with("ensure-pr:")
                && !operation.starts_with("push-integration:"))
    );
}
