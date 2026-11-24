use super::*;
use crate::application::port::inbound::task_delivery_evidence_port::{
    TaskDeliveryEvidenceDecision, TaskDeliveryEvidenceGatePort, TaskDeliveryEvidenceIntegration,
};
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelCommandOutput;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::task_delivery_evidence_repository_port::TaskDeliveryEvidenceOperation;
use crate::domain::planning::delivery_evidence::{
    TaskEvidenceDeliveryMode, TaskEvidenceDeliveryStage,
};
use std::sync::atomic::{AtomicBool, Ordering};

struct PreflightRaceGate {
    inner: Arc<dyn TaskDeliveryEvidenceGatePort>,
    workspace: String,
    next_basis: PlanningWorkPermitBasis,
    remove_preflight: bool,
    changed: AtomicBool,
}

impl TaskDeliveryEvidenceGatePort for PreflightRaceGate {
    fn evaluate(
        &self,
        workspace: &str,
        record: &PlanningAuthorityDistributorQueueRecord,
        stage: TaskEvidenceDeliveryStage,
        integration: Option<&TaskDeliveryEvidenceIntegration>,
        operation: &TaskDeliveryEvidenceOperation,
    ) -> Result<TaskDeliveryEvidenceDecision, String> {
        let mut integration = integration.cloned();
        if stage == TaskEvidenceDeliveryStage::Integrate
            && matches!(operation, TaskDeliveryEvidenceOperation::Effect { .. })
            && !self.changed.swap(true, Ordering::SeqCst)
        {
            let proof = integration.as_mut().unwrap();
            assert_eq!(proof.preflight_requires_human_review, Some(false));
            if self.remove_preflight {
                proof.preflight_requires_human_review = None;
            } else {
                set_basis_and_grant(
                    &self.workspace,
                    &self.next_basis,
                    &[PlanningWorkPermitStage::Integrate],
                    true,
                );
            }
        }
        self.inner
            .evaluate(workspace, record, stage, integration.as_ref(), operation)
    }
}

// Keep the real Git failure visible. These read-only probes run only for an
// unexpected delivery outcome; their results never retry the delivery
// or replace a failed source-range check with a fixture answer.
fn unexpected_delivery_diagnostics(fixture: &WorkPermitDeliveryFixture) -> String {
    let workspace = fixture.repo.workspace_dir();
    let snapshot = match SqlitePlanningAuthorityAdapter::new().load_runtime_projections(&workspace)
    {
        Ok(snapshot) => snapshot,
        Err(error) => return format!("authority snapshot unavailable: {error}"),
    };
    let Some(record) = snapshot
        .distributor_queue_records
        .into_iter()
        .find(|record| record.session_key == fixture.lease.session_key())
    else {
        return "the expected distributor queue record is missing".into();
    };
    let runtime = test_parallel_runtime();
    let inventory =
        diagnostic_git_command(&runtime, &workspace, &["worktree", "list", "--porcelain"]);
    let mut report = format!(
        "queue={:?}; note={:?}; frozen base={:?}; frozen tip={:?}\nworktree inventory: {}",
        record.queue_state,
        record.integration_note,
        record.source_base_commit_sha,
        record.effective_source_commit_sha(),
        describe_git_result(&inventory),
    );
    let mut paths = vec![workspace];
    if let Ok(output) = &inventory
        && output.succeeded()
        && let Ok(stdout) = std::str::from_utf8(&output.stdout)
    {
        paths.extend(
            stdout
                .lines()
                .filter_map(|line| line.strip_prefix("worktree "))
                .filter(|path| path.split(['/', '\\']).any(|part| part == ".integration"))
                .map(str::to_owned),
        );
    }
    let base = format!("{}^{{commit}}", record.source_base_commit_sha);
    let tip = format!("{}^{{commit}}", record.effective_source_commit_sha());
    let range = format!(
        "{}..{}",
        record.source_base_commit_sha,
        record.effective_source_commit_sha(),
    );
    for path in paths {
        report.push_str(&format!(
            "\nrepository path={path:?}; UTF-16 length={}",
            path.encode_utf16().count(),
        ));
        for args in [
            vec!["rev-parse", "--verify", base.as_str()],
            vec!["rev-parse", "--verify", tip.as_str()],
            vec!["rev-list", "--count", "--min-parents=2", range.as_str()],
        ] {
            let output = diagnostic_git_command(&runtime, &path, &args);
            report.push_str(&format!(
                "\n  git {args:?}: {}",
                describe_git_result(&output),
            ));
        }
    }
    report
}

fn diagnostic_git_command(
    runtime: &GitParallelModeRuntimeAdapter,
    workspace: &str,
    args: &[&str],
) -> Result<ParallelCommandOutput, String> {
    runtime.ensure_git_execution_safe(Path::new(workspace))?;
    let args = ["-C", workspace]
        .into_iter()
        .chain(args.iter().copied())
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    runtime.run_git_command(&args, None)
}

fn describe_git_result(output: &Result<ParallelCommandOutput, String>) -> String {
    match output {
        Ok(output) => format!(
            "exit={:?}, stdout={:?}, stderr={:?}",
            output.exit_code,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ),
        Err(error) => format!("process/guard error: {error}"),
    }
}

#[test]
fn a_new_human_review_requirement_or_missing_preflight_cannot_reuse_autonomous_pr_readiness() {
    for remove_preflight in [false, true] {
        let fixture = WorkPermitDeliveryFixture::new(
            "evidence-review-condition-race",
            FakeGithubAutomationPort::with_review_decision("REVIEW_REQUIRED"),
        );
        fixture.grant(
            &[
                PlanningWorkPermitStage::Push,
                PlanningWorkPermitStage::PullRequest,
                PlanningWorkPermitStage::Integrate,
            ],
            false,
        );
        let workspace = fixture.repo.workspace_dir();
        let authority = SqlitePlanningAuthorityAdapter::new();
        let identity = PlanningAuthorityPort::resolve_authority_location(&authority, &workspace)
            .unwrap()
            .repository_identity;
        let gate = Arc::new(PreflightRaceGate {
            inner: crate::composition::task_delivery_evidence::build(
                &workspace,
                &identity,
                Ok(TaskEvidenceDeliveryMode::Observe),
            ),
            workspace: workspace.clone(),
            next_basis: PlanningWorkPermitBasis {
                gate_revision: fixture.basis.gate_revision + 1,
                ..fixture.basis.clone()
            },
            remove_preflight,
            changed: AtomicBool::new(false),
        });
        let service =
            test_parallel_mode_service_with_autonomous_github(Arc::new(fixture.github.clone()))
                .with_delivery_evidence(gate.clone());
        let before = remote_integration_head(&fixture.repo);

        let result = service.process_distributor_queue(&workspace);

        assert!(
            gate.changed.load(Ordering::SeqCst),
            "remove_preflight={remove_preflight}: the final preflight race was not reached; delivery={result:?}\n{}",
            unexpected_delivery_diagnostics(&fixture),
        );
        let error = match result {
            Err(error) => error,
            Ok(notices) => panic!(
                "remove_preflight={remove_preflight}: changed review requirements unexpectedly returned {notices:?}\n{}",
                unexpected_delivery_diagnostics(&fixture),
            ),
        };
        assert!(
            error.contains("review requirements changed after the final pull request check"),
            "remove_preflight={remove_preflight}: {error}\n{}",
            unexpected_delivery_diagnostics(&fixture),
        );
        assert_eq!(remote_integration_head(&fixture.repo), before);
        fixture.assert_no_integration_push();
        assert!(
            fixture
                .operations()
                .iter()
                .any(|operation| operation.starts_with("ensure-pr:"))
        );
    }
}

mod source_range;

struct NeverRearmEvidenceWait;

impl TaskDeliveryEvidenceGatePort for NeverRearmEvidenceWait {
    fn evaluate(
        &self,
        _: &str,
        record: &PlanningAuthorityDistributorQueueRecord,
        _: TaskEvidenceDeliveryStage,
        _: Option<&TaskDeliveryEvidenceIntegration>,
        _: &TaskDeliveryEvidenceOperation,
    ) -> Result<TaskDeliveryEvidenceDecision, String> {
        Ok(TaskDeliveryEvidenceDecision {
            updated_queue: record.clone(),
            allowed: false,
            reason: Some(crate::domain::planning::delivery_evidence::TaskDeliveryEvidenceReason::EvidenceMissing),
        })
    }
}

fn integration_wait_record(
    queue_item_id: &str,
    workspace: &str,
) -> PlanningAuthorityDistributorQueueRecord {
    let mut record: PlanningAuthorityDistributorQueueRecord =
        serde_json::from_value(serde_json::json!({
            "queue_item_id": queue_item_id,
            "session_key": format!("session-{queue_item_id}"),
            "agent_id": format!("agent-{queue_item_id}"),
            "task_id": format!("task-{queue_item_id}"),
            "task_title": queue_item_id,
            "branch_name": format!("agent-{queue_item_id}"),
            "worktree_path": workspace,
            "commit_sha": "a".repeat(40),
            "validation_summary": "validated",
            "authority_refresh_outcome": "done",
            "queue_state": "blocked",
            "integration_note": "task evidence waiting: integrate / evidence missing",
            "enqueued_at": "2026-11-23T12:00:00Z",
            "updated_at": "2026-11-23T12:00:00Z"
        }))
        .unwrap();
    record.evidence_wait = Some(TaskEvidenceDeliveryStage::Integrate);
    record
}

#[test]
fn legacy_integration_evidence_wait_keeps_the_shared_worktree_queue_head() {
    let repo = TempGitRepo::new("legacy-integration-evidence-wait");
    let workspace = repo.workspace_dir();
    let runtime = test_parallel_runtime();
    let authority = NoopPlanningAuthorityPort::default().with_canonical_repo_root(&workspace);
    let mut context =
        crate::application::service::parallel_mode::pool::load_pool_runtime_context_with_runtime(
            &runtime, &authority, &workspace,
        )
        .unwrap();

    let mut legacy = integration_wait_record("legacy-a", &workspace);
    legacy.evidence_binding = None;

    let mut next = legacy.clone();
    next.queue_item_id = "legacy-b".into();
    next.session_key = "session-b".into();
    next.agent_id = "agent-b".into();
    next.task_id = "task-b".into();
    next.task_title = "Legacy B".into();
    next.branch_name = "agent-b".into();
    next.commit_sha = "b".repeat(40);
    next.queue_state = crate::domain::parallel_mode::ParallelModeQueueItemState::Queued;
    next.evidence_wait = None;
    next.integration_note = "queued".into();
    context.distributor_queue_records = vec![legacy.clone(), next];

    let selected =
        crate::application::service::parallel_mode::distributor::test_select_permitted_queue_head(
            &authority,
            &runtime,
            &NeverRearmEvidenceWait,
            &mut context,
            false,
        )
        .unwrap();

    assert_eq!(selected, Some(0));
    assert_eq!(context.distributor_queue_records[0], legacy);
}

#[test]
fn work_order_integration_evidence_wait_can_yield_after_a_blocked_recheck() {
    let repo = TempGitRepo::new("work-order-integration-evidence-wait");
    let workspace = repo.workspace_dir();
    let runtime = test_parallel_runtime();
    let authority = NoopPlanningAuthorityPort::default().with_canonical_repo_root(&workspace);
    let mut context =
        crate::application::service::parallel_mode::pool::load_pool_runtime_context_with_runtime(
            &runtime, &authority, &workspace,
        )
        .unwrap();

    fs::create_dir_all(repo.pool_root()).expect("pool root should exist before queue persistence");
    let mut work_order = integration_wait_record("work-order-a", &workspace);
    work_order.evidence_binding = Some(
        crate::domain::planning::delivery_evidence::TaskEvidenceDeliveryBinding {
            scope: crate::domain::planning::evidence::TaskEvidenceScope {
                task_id: work_order.task_id.clone(),
                scope_id: "scope-a".into(),
                acceptance_revision: 1,
                criteria_sha256: "a".repeat(64),
            },
            lease_generation: "lease-a".into(),
        },
    );
    let mut next = integration_wait_record("work-order-b", &workspace);
    next.queue_state = crate::domain::parallel_mode::ParallelModeQueueItemState::Queued;
    next.evidence_wait = None;
    next.evidence_binding = None;
    next.integration_note = "queued".into();
    context.distributor_queue_records = vec![work_order, next];

    let selected =
        crate::application::service::parallel_mode::distributor::test_select_permitted_queue_head(
            &authority,
            &runtime,
            &NeverRearmEvidenceWait,
            &mut context,
            false,
        )
        .unwrap();

    assert_eq!(selected, Some(1));
}
