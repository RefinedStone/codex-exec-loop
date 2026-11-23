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
