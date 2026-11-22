use super::*;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskAuthorityMutationAudit;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkOrderRegistration, PlanningWorkOrderRegistrationResult,
    PlanningWorkOrderRepositoryPort, PlanningWorkPermit, PlanningWorkPermitBasis,
    PlanningWorkPermitDecision, PlanningWorkPermitRepositoryPort, PlanningWorkPermitStage,
};
use crate::domain::planning::{OriginSessionKind, TaskMutationProvenance};

const WORK_ORDER_ID: &str = "work-order:delivery-permit";
const TASK_ID: &str = "task-1";

struct WorkPermitDeliveryFixture {
    repo: TempGitRepo,
    service: ParallelModeService,
    github: FakeGithubAutomationPort,
    lease: ParallelModeSlotLeaseSnapshot,
    basis: PlanningWorkPermitBasis,
}

impl WorkPermitDeliveryFixture {
    fn new(prefix: &str, github: FakeGithubAutomationPort) -> Self {
        Self::with_link_reference(prefix, github, None)
    }

    fn with_link_reference(
        prefix: &str,
        github: FakeGithubAutomationPort,
        reference: Option<&crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkLinkReference>,
    ) -> Self {
        let repo = TempGitRepo::new(prefix);
        register_delivery_work_order_with_link(&repo, reference);
        let initial_basis = PlanningWorkPermitBasis {
            authorization_epoch: 0,
            policy_version: "delivery-policy-v1".to_string(),
            field_hash: "delivery-fields-v1".to_string(),
            gate_revision: 1,
            source_sha: repo.head_sha(),
        };
        set_basis_and_grant(
            &repo.workspace_dir(),
            &initial_basis,
            &[PlanningWorkPermitStage::Dispatch],
            false,
        );
        let service = test_parallel_mode_service_with_autonomous_github(Arc::new(github.clone()));
        let lease = blocked::enqueue_single_commit_ready_result(&service, &repo, prefix);
        let source_sha = run_command(
            "git",
            ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
            None,
        )
        .expect("queued source commit should resolve");
        let basis = PlanningWorkPermitBasis {
            gate_revision: 2,
            source_sha,
            ..initial_basis
        };
        SqlitePlanningAuthorityAdapter::new()
            .set_work_order_permit_basis(&repo.workspace_dir(), WORK_ORDER_ID, &basis)
            .expect("delivery evidence should replace the dispatch evidence");
        Self {
            repo,
            service,
            github,
            lease,
            basis,
        }
    }

    fn grant(&self, stages: &[PlanningWorkPermitStage], requires_human_review: bool) {
        set_basis_and_grant(
            &self.repo.workspace_dir(),
            &self.basis,
            stages,
            requires_human_review,
        );
    }

    fn process(&self) -> Vec<String> {
        self.service
            .process_distributor_queue(&self.repo.workspace_dir())
            .expect("delivery should record a blocked or completed result")
    }

    fn operations(&self) -> Vec<String> {
        self.github
            .operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .clone()
    }

    fn assert_blocked(&self, reason: &str, notices: &[String]) {
        let record =
            load_distributor_queue_records(&test_parallel_runtime(), &self.repo.pool_root())
                .into_iter()
                .next()
                .expect("blocked work-order delivery should retain its queue record");
        assert_eq!(record.queue_state, ParallelModeQueueItemState::Blocked);
        assert!(
            record.integration_note.contains(reason),
            "expected {reason}, got {:?}; notices: {notices:?}",
            record.integration_note
        );
    }

    fn assert_no_integration_push(&self) {
        assert!(
            !self
                .operations()
                .iter()
                .any(|operation| operation.starts_with("push-integration:")),
            "a denied work-order permit must prevent the integration adapter call"
        );
    }

    fn change_source_at(
        &self,
        boundary: FakeGithubAutomationBoundary,
        stage: PlanningWorkPermitStage,
    ) {
        let workspace = self.repo.workspace_dir();
        let basis = PlanningWorkPermitBasis {
            source_sha: self.repo.head_sha(),
            gate_revision: self.basis.gate_revision + 1,
            ..self.basis.clone()
        };
        assert_ne!(basis.source_sha, self.basis.source_sha);
        self.github.set_boundary_hook(boundary, move || {
            set_basis_and_grant(&workspace, &basis, &[stage], false);
        });
    }

    fn revoke_at(&self, boundary: FakeGithubAutomationBoundary, stage: PlanningWorkPermitStage) {
        let workspace = self.repo.workspace_dir();
        self.github.set_boundary_hook(boundary, move || {
            assert_eq!(
                SqlitePlanningAuthorityAdapter::new()
                    .revoke_work_order_permits(
                        &workspace,
                        WORK_ORDER_ID,
                        &[stage],
                        "reviewer revoked permission during delivery preflight",
                    )
                    .expect("live permission revocation should persist"),
                1,
                "the test must revoke a previously valid grant"
            );
        });
    }
}

fn register_delivery_work_order(repo: &TempGitRepo) {
    register_delivery_work_order_with_link(repo, None);
}

fn register_delivery_work_order_with_link(
    repo: &TempGitRepo,
    reference: Option<&crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkLinkReference>,
) {
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let workspace = repo.workspace_dir();
    let empty_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };
    adapter
        .commit_task_authority_snapshot(
            &workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &TaskAuthorityDocument {
                    version: 1,
                    tasks: Vec::new(),
                },
                queue_projection: &empty_projection,
            },
        )
        .expect("work-order fixture should initialize task authority");
    let snapshot = adapter
        .load_task_authority_snapshot(&workspace)
        .expect("fixture authority should load")
        .expect("fixture authority should exist");
    let provenance = TaskMutationProvenance::new(OriginSessionKind::System);
    let task = TaskDefinition {
        acceptance: Vec::new(),
        acceptance_revision: 0,
        id: TASK_ID.to_string(),
        direction_id: "direction-1".to_string(),
        direction_relation_note: "stage permit regression fixture".to_string(),
        title: "Task One".to_string(),
        description: "Deliver the exact reviewed worker result.".to_string(),
        status: TaskStatus::Ready,
        base_priority: 50,
        dynamic_priority_delta: 0,
        priority_reason: String::new(),
        depends_on: Vec::new(),
        blocked_by: Vec::new(),
        created_by: TaskActor::System,
        last_updated_by: TaskActor::System,
        source_turn_id: None,
        provenance: provenance.clone(),
        updated_at: "2026-11-21T00:00:00Z".to_string(),
    };
    let queued = PriorityQueueTask {
        rank: 1,
        task_id: task.id.clone(),
        direction_id: task.direction_id.clone(),
        direction_title: "Permit fixture".to_string(),
        task_title: task.title.clone(),
        status: task.status,
        combined_priority: 50,
        updated_at: task.updated_at.clone(),
        rank_reasons: Vec::new(),
    };
    let document = TaskAuthorityDocument {
        version: 1,
        tasks: vec![task],
    };
    let projection = PriorityQueueProjection {
        next_task: Some(queued.clone()),
        active_tasks: vec![queued],
        ..empty_projection
    };
    let registered = adapter
        .register_work_order(
            &workspace,
            PlanningWorkOrderRegistration {
                link_reference: reference,
                work_order_id: WORK_ORDER_ID,
                origin: "synthetic-permit-regression",
                initial_task_id: TASK_ID,
                observed_planning_revision: snapshot.planning_revision,
                task_authority: &document,
                queue_projection: &projection,
                audit: PlanningTaskAuthorityMutationAudit {
                    task_ids: &[TASK_ID.to_string()],
                    legacy_source_turn_id: None,
                    provenance: &provenance,
                },
            },
        )
        .expect("fixture work order should register through the ordinary authority transaction");
    assert!(matches!(
        registered,
        PlanningWorkOrderRegistrationResult::Registered { .. }
    ));
}

fn set_basis_and_grant(
    workspace: &str,
    basis: &PlanningWorkPermitBasis,
    stages: &[PlanningWorkPermitStage],
    requires_human_review: bool,
) {
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .set_work_order_permit_basis(workspace, WORK_ORDER_ID, basis)
        .expect("current permit evidence should persist");
    for &stage in stages {
        adapter
            .grant_work_order_permit(
                workspace,
                &PlanningWorkPermit {
                    work_order_id: WORK_ORDER_ID.to_string(),
                    stage,
                    basis: basis.clone(),
                    requires_human_review,
                },
            )
            .expect("stage permit should match the independently stored basis");
    }
}

fn remote_heads(repo: &TempGitRepo) -> String {
    let output = crate::test_utils::git_command()
        .arg("-C")
        .arg(repo.root.join("origin.git"))
        .args([
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads",
        ])
        .output()
        .expect("bare remote refs should be readable");
    assert!(
        output.status.success(),
        "bare remote refs should resolve: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("bare remote refs should be UTF-8")
}

fn remote_integration_head(repo: &TempGitRepo) -> String {
    let output = crate::test_utils::git_command()
        .arg("-C")
        .arg(repo.root.join("origin.git"))
        .args(["rev-parse", "refs/heads/prerelease"])
        .output()
        .expect("bare integration ref should be readable");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("integration ref should be UTF-8")
        .trim()
        .to_string()
}

#[test]
fn work_order_without_integrate_permit_blocks_even_with_process_opt_in() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-integrate-missing",
        FakeGithubAutomationPort::ready(),
    );
    fixture.grant(
        &[
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
        ],
        false,
    );
    let before = remote_integration_head(&fixture.repo);

    let notices = fixture.process();

    fixture.assert_blocked("permit_missing", &notices);
    fixture.assert_no_integration_push();
    assert_eq!(remote_integration_head(&fixture.repo), before);
    assert!(
        fixture
            .operations()
            .iter()
            .any(|operation| operation.starts_with("ensure-pr:")),
        "independent push and PR permits must remain usable before the integrate gate"
    );
}

#[test]
fn work_order_human_review_requirement_survives_process_opt_in() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-human-review",
        FakeGithubAutomationPort::with_review_decision("REVIEW_REQUIRED"),
    );
    fixture.grant(
        &[
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
            PlanningWorkPermitStage::Integrate,
        ],
        true,
    );
    let before = remote_integration_head(&fixture.repo);

    let notices = fixture.process();

    fixture.assert_blocked("APPROVED", &notices);
    fixture.assert_no_integration_push();
    assert_eq!(remote_integration_head(&fixture.repo), before);
}

#[test]
fn work_order_human_review_requirement_blocks_direct_delivery() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-direct-human-review",
        FakeGithubAutomationPort::ready(),
    );
    fixture.grant(
        &[
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
            PlanningWorkPermitStage::Integrate,
        ],
        true,
    );
    run_git(
        &fixture.repo.repo_root,
        &["config", "akra.githubPrMode", "disabled"],
    );
    let before = remote_integration_head(&fixture.repo);

    let notices = fixture.process();

    fixture.assert_blocked("review", &notices);
    fixture.assert_no_integration_push();
    assert_eq!(remote_integration_head(&fixture.repo), before);
}

#[test]
fn work_order_push_revoked_during_preflight_leaves_bare_remote_refs_unchanged() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-push-live-revoke",
        FakeGithubAutomationPort::ready(),
    );
    fixture.grant(
        &[
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
            PlanningWorkPermitStage::Integrate,
        ],
        false,
    );
    let before = remote_heads(&fixture.repo);
    assert!(matches!(
        SqlitePlanningAuthorityAdapter::new()
            .check_work_order_permit_for_task(
                &fixture.repo.workspace_dir(),
                TASK_ID,
                PlanningWorkPermitStage::Push,
                Some(&fixture.basis.source_sha),
            )
            .expect("push admission should be readable before revocation"),
        PlanningWorkPermitDecision::Granted { .. }
    ));
    fixture.revoke_at(
        FakeGithubAutomationBoundary::SourcePushPreflight,
        PlanningWorkPermitStage::Push,
    );

    let notices = fixture.process();

    fixture.assert_blocked("permit_missing", &notices);
    assert!(
        fixture.operations().is_empty(),
        "a revoked push must not call any delivery mutation"
    );
    assert_eq!(remote_heads(&fixture.repo), before);
}

#[test]
fn work_order_push_source_sha_mismatch_prevents_the_remote_push() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-push-source-sha",
        FakeGithubAutomationPort::ready(),
    );
    let wrong_basis = PlanningWorkPermitBasis {
        source_sha: fixture.repo.head_sha(),
        gate_revision: fixture.basis.gate_revision + 1,
        ..fixture.basis.clone()
    };
    assert_ne!(wrong_basis.source_sha, fixture.basis.source_sha);
    set_basis_and_grant(
        &fixture.repo.workspace_dir(),
        &wrong_basis,
        &[PlanningWorkPermitStage::Push],
        false,
    );
    let before = remote_heads(&fixture.repo);

    let notices = fixture.process();

    fixture.assert_blocked("permit_stale", &notices);
    assert!(fixture.operations().is_empty());
    assert_eq!(remote_heads(&fixture.repo), before);
}

#[test]
fn work_order_pull_request_source_sha_mismatch_prevents_pr_creation() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-pr-source-sha",
        FakeGithubAutomationPort::ready(),
    );
    fixture.grant(
        &[
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
        ],
        false,
    );
    fixture.change_source_at(
        FakeGithubAutomationBoundary::SourcePushed,
        PlanningWorkPermitStage::PullRequest,
    );
    let before = remote_integration_head(&fixture.repo);

    let notices = fixture.process();

    fixture.assert_blocked("permit_stale", &notices);
    assert_eq!(
        fixture.operations(),
        vec![format!("push:{}:false", fixture.lease.branch_name)],
        "a new source basis after push must invalidate PR creation for the frozen source"
    );
    assert_eq!(remote_integration_head(&fixture.repo), before);
}

#[test]
fn work_order_integrate_source_sha_mismatch_prevents_target_push() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-integrate-source-sha",
        FakeGithubAutomationPort::ready(),
    );
    fixture.grant(
        &[
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
            PlanningWorkPermitStage::Integrate,
        ],
        false,
    );
    fixture.change_source_at(
        FakeGithubAutomationBoundary::PullRequestEnsured,
        PlanningWorkPermitStage::Integrate,
    );
    let before = remote_integration_head(&fixture.repo);

    let notices = fixture.process();

    fixture.assert_blocked("permit_stale", &notices);
    fixture.assert_no_integration_push();
    assert_eq!(remote_integration_head(&fixture.repo), before);
    assert!(
        fixture
            .operations()
            .iter()
            .any(|operation| operation.starts_with("ensure-pr:")),
        "the integrate mismatch must occur after a permitted source push and PR creation"
    );
}

#[test]
fn work_order_integrate_revoked_during_final_review_prevents_target_push() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-integrate-live-revoke",
        FakeGithubAutomationPort::ready(),
    );
    fixture.grant(
        &[
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
            PlanningWorkPermitStage::Integrate,
        ],
        false,
    );
    fixture.revoke_at(
        FakeGithubAutomationBoundary::IntegrationPushReadiness,
        PlanningWorkPermitStage::Integrate,
    );
    let before = remote_integration_head(&fixture.repo);

    let notices = fixture.process();

    fixture.assert_blocked("permit_missing", &notices);
    fixture.assert_no_integration_push();
    assert_eq!(remote_integration_head(&fixture.repo), before);
    assert!(
        fixture
            .operations()
            .iter()
            .filter(|operation| operation.starts_with("inspect-pr:"))
            .count()
            >= 2,
        "revocation must happen in the final review check, after integration preparation"
    );
}

#[test]
fn work_order_current_permits_and_approved_exact_head_complete_delivery() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-approved-delivery",
        FakeGithubAutomationPort::ready(),
    );
    fixture.grant(
        &[
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
            PlanningWorkPermitStage::Integrate,
        ],
        true,
    );
    let before = remote_integration_head(&fixture.repo);

    let notices = fixture.process();

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor integrated queue head")),
        "valid stage permits and an approved current source must complete delivery: {notices:?}"
    );
    assert!(
        fixture
            .operations()
            .iter()
            .any(|operation| operation == "push-integration:prerelease")
    );
    assert_ne!(remote_integration_head(&fixture.repo), before);
}

#[test]
fn work_order_new_integrate_grant_resumes_waiting_delivery_without_polling_github() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-resume-current-grant",
        FakeGithubAutomationPort::ready(),
    );
    fixture.grant(
        &[
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
        ],
        false,
    );
    let first = fixture.process();
    fixture.assert_blocked("permit_missing", &first);
    let waiting_operations = fixture.operations();
    let before = remote_integration_head(&fixture.repo);

    let waiting = fixture.process();
    fixture.assert_blocked("permit_missing", &waiting);
    assert_eq!(
        fixture.operations(),
        waiting_operations,
        "a permit wait should read local authority without retrying GitHub delivery"
    );

    fixture.grant(&[PlanningWorkPermitStage::Integrate], false);
    let resumed = fixture.process();

    assert!(
        resumed
            .iter()
            .any(|notice| notice.contains("distributor integrated queue head")),
        "a new current grant should resume the existing queue item: {resumed:?}"
    );
    assert_ne!(remote_integration_head(&fixture.repo), before);
    let records =
        load_distributor_queue_records(&test_parallel_runtime(), &fixture.repo.pool_root());
    assert_eq!(
        records.len(),
        1,
        "resume must reuse the original queue identity"
    );
    assert_eq!(records[0].queue_state, ParallelModeQueueItemState::Done);
}

#[test]
fn work_order_permit_wait_does_not_starve_an_unrelated_task() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-unrelated-task",
        FakeGithubAutomationPort::ready(),
    );
    let first = fixture.process();
    fixture.assert_blocked("permit_missing", &first);
    assert!(fixture.operations().is_empty());
    let before = remote_integration_head(&fixture.repo);

    let unrelated = enqueue_unrelated_delivery(&fixture.service, &fixture.repo);

    let notices = fixture.process();

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor integrated queue head")),
        "a permit-blocked task must yield to unrelated queued delivery: {notices:?}"
    );
    let records =
        load_distributor_queue_records(&test_parallel_runtime(), &fixture.repo.pool_root());
    assert_eq!(records.len(), 2);
    let blocked = records
        .iter()
        .find(|record| record.task_id == TASK_ID)
        .unwrap();
    let completed = records
        .iter()
        .find(|record| record.task_id == "task-2")
        .unwrap();
    assert_eq!(blocked.queue_state, ParallelModeQueueItemState::Blocked);
    assert!(blocked.integration_note.contains("permit_missing"));
    assert_eq!(completed.queue_state, ParallelModeQueueItemState::Done);
    assert_eq!(
        fixture
            .operations()
            .iter()
            .filter(|operation| operation.starts_with("push:"))
            .cloned()
            .collect::<Vec<_>>(),
        vec![format!("push:{}:false", unrelated.branch_name)],
        "only the unrelated task may publish a source branch"
    );
    assert_ne!(remote_integration_head(&fixture.repo), before);
    let output = crate::test_utils::git_command()
        .arg("-C")
        .arg(fixture.repo.root.join("origin.git"))
        .args(["ls-tree", "--name-only", "refs/heads/prerelease"])
        .output()
        .expect("delivered tree should be readable");
    assert!(output.status.success());
    let files = String::from_utf8(output.stdout).expect("delivered tree should be UTF-8");
    assert!(files.lines().any(|file| file == "unrelated.txt"));
    assert!(
        !files.lines().any(|file| file == "feature.txt"),
        "the permit-blocked worker result must not leak into the unrelated integration"
    );
}

fn enqueue_unrelated_delivery(
    service: &ParallelModeService,
    repo: &TempGitRepo,
) -> ParallelModeSlotLeaseSnapshot {
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-2", "Task Two", "agent-2", "task-two"),
        )
        .expect("an ordinary unrelated task should retain legacy dispatch behavior");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("unrelated slot should start");
    repo.commit_file_in_slot(
        Path::new(&lease.worktree_path),
        "unrelated.txt",
        "independent result\n",
        "unrelated worker result",
    );
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-unrelated-permit-wait",
            None,
            Some("Unrelated task completed."),
            Some("fixture validation passed"),
            None,
        )
        .expect("unrelated official completion should persist");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("unrelated official refresh should persist");
    service
        .mark_workspace_commit_ready(&lease.worktree_path, "unrelated delivery is ready")
        .expect("unrelated result should become commit-ready");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("unrelated result should enqueue")
        .expect("unrelated queue identity should be created");
    lease
}

fn detached_integration_worktree_for_head(repo: &TempGitRepo, commit_sha: &str) -> PathBuf {
    let output = crate::test_utils::git_command()
        .arg("-C")
        .arg(&repo.repo_root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .expect("registered worktrees should be readable");
    assert!(output.status.success());
    let worktrees = String::from_utf8(output.stdout).expect("worktree paths should be UTF-8");
    worktrees
        .split("\n\n")
        .find_map(|entry| {
            let has_head = entry
                .lines()
                .any(|line| line == format!("HEAD {commit_sha}"));
            let detached = entry.lines().any(|line| line == "detached");
            (has_head && detached)
                .then(|| {
                    entry
                        .lines()
                        .find_map(|line| line.strip_prefix("worktree "))
                })
                .flatten()
                .map(PathBuf::from)
        })
        .expect("the prepared integration commit should remain in its detached worktree")
}

#[test]
fn work_order_revoked_after_cherry_pick_preserves_its_result_and_allows_unrelated_integration() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-prepared-result-isolation",
        FakeGithubAutomationPort::ready(),
    );
    fixture.grant(
        &[
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
            PlanningWorkPermitStage::Integrate,
        ],
        false,
    );
    fixture.revoke_at(
        FakeGithubAutomationBoundary::IntegrationPushReadiness,
        PlanningWorkPermitStage::Integrate,
    );
    let before = remote_integration_head(&fixture.repo);
    let blocked = fixture.process();
    fixture.assert_blocked("permit_missing", &blocked);
    fixture.assert_no_integration_push();
    assert_eq!(remote_integration_head(&fixture.repo), before);
    let paused =
        load_distributor_queue_records(&test_parallel_runtime(), &fixture.repo.pool_root())
            .into_iter()
            .find(|record| record.task_id == TASK_ID)
            .expect("revoked task should retain its queue evidence");
    let prepared_sha = paused
        .integration_commit_sha
        .clone()
        .expect("the source must already have been cherry-picked before final revocation");
    let paused_worktree = detached_integration_worktree_for_head(&fixture.repo, &prepared_sha);

    // A separate fake PR registry gives the unrelated delivery its own PR and still uses
    // the same durable authority and real bare remote.
    let github = GitBackedGithubAutomationPort::ready();
    let service = test_parallel_mode_service_with_autonomous_github(Arc::new(github));
    enqueue_unrelated_delivery(&service, &fixture.repo);
    let completed = service
        .process_distributor_queue(&fixture.repo.workspace_dir())
        .expect("unrelated delivery should run while the prepared result awaits a new permit");

    assert!(
        completed
            .iter()
            .any(|notice| notice.contains("distributor integrated queue head")),
        "a paused prepared integration must not reserve another task's worktree: {completed:?}"
    );
    let records =
        load_distributor_queue_records(&test_parallel_runtime(), &fixture.repo.pool_root());
    let waiting = records
        .iter()
        .find(|record| record.task_id == TASK_ID)
        .unwrap();
    let unrelated = records
        .iter()
        .find(|record| record.task_id == "task-2")
        .unwrap();
    assert_eq!(waiting.queue_state, ParallelModeQueueItemState::Blocked);
    assert!(waiting.integration_note.contains("permit_missing"));
    assert_eq!(
        waiting.integration_commit_sha.as_deref(),
        Some(prepared_sha.as_str())
    );
    assert_eq!(unrelated.queue_state, ParallelModeQueueItemState::Done);
    assert_eq!(
        run_command(
            "git",
            ["-C", paused_worktree.to_str().unwrap(), "rev-parse", "HEAD"],
            None,
        )
        .as_deref(),
        Some(prepared_sha.as_str()),
        "another task's integration must not reset the revoked task's local result"
    );
    let output = crate::test_utils::git_command()
        .arg("-C")
        .arg(fixture.repo.root.join("origin.git"))
        .args(["ls-tree", "--name-only", "refs/heads/prerelease"])
        .output()
        .expect("delivered tree should be readable");
    assert!(output.status.success());
    let files = String::from_utf8(output.stdout).expect("delivered tree should be UTF-8");
    assert!(files.lines().any(|file| file == "unrelated.txt"));
    assert!(!files.lines().any(|file| file == "feature.txt"));
}

#[test]
fn work_order_stale_permit_rearm_cannot_resurrect_a_completed_queue_record() {
    let fixture =
        WorkPermitDeliveryFixture::new("work-permit-rearm-cas", FakeGithubAutomationPort::ready());
    let notices = fixture.process();
    fixture.assert_blocked("permit_missing", &notices);
    let observed =
        load_distributor_queue_records(&test_parallel_runtime(), &fixture.repo.pool_root())
            .into_iter()
            .next()
            .expect("waiting queue record should exist");
    let mut completed = observed.clone();
    completed.queue_state = ParallelModeQueueItemState::Done;
    completed.integration_note = "another owner already completed this delivery".to_string();
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &fixture.repo.workspace_dir(),
        &completed,
    )
    .expect("a newer completed transition should persist");
    let mut stale_rearm = observed.clone();
    stale_rearm.queue_state = ParallelModeQueueItemState::Queued;
    stale_rearm.integration_note = "stale permission observer attempted to rearm".to_string();

    let swapped =
        SqlitePlanningAuthorityAdapter::compare_and_swap_runtime_distributor_queue_record(
            &fixture.repo.workspace_dir(),
            &observed,
            &stale_rearm,
        )
        .expect("a lost compare-and-swap is a normal concurrent outcome");

    assert!(
        !swapped,
        "the stale blocked snapshot must lose to the newer Done row"
    );
    let current =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&fixture.repo.workspace_dir())
            .expect("current queue authority should load")
            .distributor_queue_records
            .into_iter()
            .next()
            .expect("completed queue record should remain");
    assert_eq!(current.queue_state, ParallelModeQueueItemState::Done);
    assert_eq!(current.integration_note, completed.integration_note);
}

#[test]
fn work_order_dispatch_plan_uses_its_last_idle_slot_for_the_next_permitted_task() {
    let repo = TempGitRepo::new("work-permit-dispatch-capacity");
    register_delivery_work_order(&repo);
    let basis = PlanningWorkPermitBasis {
        authorization_epoch: 0,
        policy_version: "delivery-policy-v1".to_string(),
        field_hash: "delivery-fields-v1".to_string(),
        gate_revision: 1,
        source_sha: repo.head_sha(),
    };
    set_basis_and_grant(
        &repo.workspace_dir(),
        &basis,
        &[PlanningWorkPermitStage::Dispatch],
        false,
    );
    let authority = SqlitePlanningAuthorityAdapter::new();
    assert_eq!(
        authority
            .revoke_work_order_permits(
                &repo.workspace_dir(),
                WORK_ORDER_ID,
                &[PlanningWorkPermitStage::Dispatch],
                "hold the first task before dispatch",
            )
            .expect("dispatch permission should revoke"),
        1
    );
    repo.create_detached_slot(1);
    for slot in [2, 3] {
        let path = repo.create_agent_slot(slot, &format!("occupied-{slot}"));
        repo.commit_file_in_slot(
            &path,
            &format!("occupied-{slot}.txt"),
            "independent occupied worktree\n",
            "preserve occupied worker output",
        );
    }
    let mut projection = authority
        .load_task_authority_snapshot(&repo.workspace_dir())
        .expect("planning authority should load")
        .expect("planning authority should exist")
        .queue_projection;
    let mut unrelated = projection.active_tasks[0].clone();
    unrelated.task_id = "task-2".to_string();
    unrelated.task_title = "Unrelated legacy task".to_string();
    unrelated.rank = 2;
    projection.active_tasks.push(unrelated);
    let planning = PlanningRuntimeProjection::ready_with_queue_projection(
        "prompt".to_string(),
        projection.queue_summary(),
        None,
        projection.next_task.clone(),
        projection,
    );
    let service = test_parallel_mode_service();

    let plan = service
        .build_dispatch_plan(&repo.workspace_dir(), &planning, usize::MAX)
        .expect("a held work order must not block dispatch planning");

    assert_eq!(plan.idle_slot_count, 1);
    assert_eq!(
        plan.candidates
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-2"],
        "permission filtering must happen before applying the idle-slot limit"
    );
}

mod control_cleanup;
mod link_reference;
mod rearm_conflict;
mod shared_integration;
