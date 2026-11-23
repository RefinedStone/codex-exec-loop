//! Issue Auto is a scoped permit, not the ordinary process-wide autonomous fallback.
use super::*;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskAuthorityMutationAudit;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkDeliveryAuthority, PlanningWorkDeliveryReview, PlanningWorkLinkReference,
    PlanningWorkOrderRegistration, PlanningWorkOrderRegistrationResult,
    PlanningWorkOrderRepositoryPort, PlanningWorkPermit, PlanningWorkPermitBasis,
    PlanningWorkPermitDecision, PlanningWorkPermitRepositoryPort, PlanningWorkPermitStage,
};
use std::sync::atomic::{AtomicBool, Ordering};

const WORK: &str = "work-order:auto-delivery";
const SOURCE_REPOSITORY: &str = "refinedstone/codex-exec-loop";

struct AutoFixture {
    inner: Fixture,
}

impl AutoFixture {
    fn new(
        label: &str,
        mode: TaskEvidenceDeliveryMode,
        opt_in: bool,
        github: FakeGithubAutomationPort,
    ) -> Self {
        Self {
            inner: Fixture::with_setup(label, mode, github, seed_issue_task, |service| {
                // Even the older, generic public opt-in cannot grant issue Auto on a public repo.
                service.with_test_delivery_safety_policy(true, opt_in)
            }),
        }
    }

    fn prepare(&self) {
        self.inner.verify_host(true);
        self.inner.enqueue();
        let row = self.inner.queue();
        let basis = PlanningWorkPermitBasis {
            authorization_epoch: 0,
            policy_version: "auto-delivery-policy".into(),
            field_hash: "auto-delivery-fields".into(),
            gate_revision: 2,
            source_sha: row.source_commit_sha.clone(),
            delivery_review: Some(PlanningWorkDeliveryReview {
                base_commit_sha: row.source_base_commit_sha.clone(),
                source_commit_sha: row.source_commit_sha,
                gate_revision: 2,
                authority: Some(PlanningWorkDeliveryAuthority::Auto {
                    merge_gate_revision: 2,
                    source_repository: SOURCE_REPOSITORY.into(),
                }),
            }),
        };
        grant(
            &self.inner.repo.workspace_dir(),
            &basis,
            &[
                PlanningWorkPermitStage::Push,
                PlanningWorkPermitStage::PullRequest,
                PlanningWorkPermitStage::Integrate,
            ],
        );
    }

    fn assert_no_integration(&self) {
        assert!(
            self.inner
                .operations()
                .iter()
                .all(|operation| !operation.starts_with("push-integration:")),
            "{:?}",
            self.inner.operations()
        );
        assert_eq!(
            self.inner.remote_ref(POOL_BASELINE_BRANCH).as_deref(),
            Some(
                self.inner
                    .lease
                    .delivery_target
                    .as_ref()
                    .unwrap()
                    .integration_base_commit_sha
                    .as_str()
            )
        );
    }

    fn assert_integrated(&self) {
        let row = self.inner.queue();
        assert!(
            self.inner
                .operations()
                .iter()
                .any(|operation| operation.starts_with("push-integration:")),
            "queue={row:?}; operations={:?}",
            self.inner.operations()
        );
        assert_eq!(
            self.inner.remote_ref(POOL_BASELINE_BRANCH),
            row.integration_commit_sha
        );
        for stage in [
            TaskEvidenceDeliveryStage::Push,
            TaskEvidenceDeliveryStage::PullRequest,
            TaskEvidenceDeliveryStage::Integrate,
        ] {
            let audit = row
                .evidence_observations
                .iter()
                .find(|audit| audit.stage == stage)
                .unwrap();
            assert_eq!(audit.mode, TaskEvidenceDeliveryMode::Enforce);
            assert!(audit.satisfied(), "{audit:?}");
            assert_eq!(
                audit.criteria[0].state,
                TaskDeliveryCriterionState::HostPassed
            );
        }
    }
}

fn seed_issue_task(repo: &TempGitRepo, store: &SqlitePlanningAuthorityAdapter) {
    let workspace = repo.workspace_dir();
    seed_task_with_commit(
        &workspace,
        store,
        true,
        |task_authority, queue_projection| {
            // The typed criterion is explicitly Human-owned. Work-order registration does not
            // synthesize or silently approve acceptance criteria from an issue's prose.
            assert_eq!(task_authority.tasks[0].created_by, TaskActor::User);
            store
                .commit_task_authority_snapshot(
                    &workspace,
                    PlanningTaskAuthorityCommit {
                        observed_planning_revision: None,
                        task_authority: &TaskAuthorityDocument {
                            version: TASK_AUTHORITY_FORMAT_VERSION,
                            tasks: vec![],
                        },
                        queue_projection,
                    },
                )
                .unwrap();
            let snapshot = store
                .load_task_authority_snapshot(&workspace)
                .unwrap()
                .unwrap();
            let reference = PlanningWorkLinkReference {
                url: format!("https://github.com/{SOURCE_REPOSITORY}/issues/2560"),
                close_on_merge: false,
            };
            let result = store
                .register_work_order(
                    &workspace,
                    PlanningWorkOrderRegistration {
                        work_order_id: WORK,
                        origin: "issue_automation",
                        link_reference: Some(&reference),
                        initial_task_id: TASK,
                        observed_planning_revision: snapshot.planning_revision,
                        task_authority,
                        queue_projection,
                        audit: PlanningTaskAuthorityMutationAudit {
                            task_ids: &[TASK.into()],
                            legacy_source_turn_id: None,
                            provenance: &task_authority.tasks[0].provenance,
                        },
                    },
                )
                .unwrap();
            assert!(matches!(
                result,
                PlanningWorkOrderRegistrationResult::Registered { .. }
            ));
        },
    );
    grant(
        &workspace,
        &PlanningWorkPermitBasis {
            authorization_epoch: 0,
            policy_version: "auto-delivery-policy".into(),
            field_hash: "auto-delivery-fields".into(),
            gate_revision: 1,
            source_sha: repo.head_sha(),
            delivery_review: None,
        },
        &[PlanningWorkPermitStage::Dispatch],
    );
}

fn grant(workspace: &str, basis: &PlanningWorkPermitBasis, stages: &[PlanningWorkPermitStage]) {
    let store = SqlitePlanningAuthorityAdapter::new();
    store
        .set_work_order_permit_basis(workspace, WORK, basis)
        .unwrap();
    for &stage in stages {
        store
            .grant_work_order_permit(
                workspace,
                &PlanningWorkPermit {
                    work_order_id: WORK.into(),
                    stage,
                    basis: basis.clone(),
                    requires_human_review: false,
                },
            )
            .unwrap();
    }
}

#[test]
fn explicit_auto_with_host_evidence_and_opt_in_omits_only_human_pr_approval() {
    let github = FakeGithubAutomationPort::with_review_decision("REVIEW_REQUIRED");
    *github.inspect_approved_review_commit_shas.lock().unwrap() = Some(vec![]);
    let fixture = AutoFixture::new(
        "auto-delivery-host-pass",
        TaskEvidenceDeliveryMode::Enforce,
        true,
        github,
    );
    fixture.prepare();
    use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
    assert_eq!(
        PlanningAuthorityPort::check_work_order_permit_for_task(
            fixture.inner.authority.as_ref(),
            &fixture.inner.repo.workspace_dir(),
            TASK,
            PlanningWorkPermitStage::Integrate,
            Some(&fixture.inner.queue().source_commit_sha),
        )
        .unwrap(),
        PlanningWorkPermitDecision::GrantedAuto
    );

    fixture.inner.process();

    fixture.assert_integrated();
}

#[test]
fn auto_without_process_opt_in_keeps_the_exact_human_review_path() {
    for approved in [false, true] {
        let github = FakeGithubAutomationPort::with_review_decision(if approved {
            "APPROVED"
        } else {
            "REVIEW_REQUIRED"
        });
        let fixture = AutoFixture::new(
            "auto-delivery-human-fallback",
            TaskEvidenceDeliveryMode::Enforce,
            false,
            github,
        );
        fixture.prepare();

        fixture.inner.process();

        assert!(
            fixture
                .inner
                .operations()
                .iter()
                .any(|op| op.starts_with("ensure-pr:"))
        );
        if approved {
            fixture.assert_integrated();
        } else {
            fixture.assert_no_integration();
            assert!(fixture.inner.queue().integration_note.contains("APPROVED"));
        }
    }
}

#[test]
fn auto_does_not_publish_under_observe_even_with_a_real_host_pass() {
    let fixture = AutoFixture::new(
        "auto-delivery-observe",
        TaskEvidenceDeliveryMode::Observe,
        true,
        FakeGithubAutomationPort::ready(),
    );
    fixture.prepare();

    fixture.inner.process();

    fixture.inner.assert_no_source_or_pr();
    let queue = fixture.inner.queue();
    assert_eq!(queue.queue_state, ParallelModeQueueItemState::Blocked);
    assert_eq!(queue.evidence_wait, Some(TaskEvidenceDeliveryStage::Push));
    assert!(fixture.inner.gate.calls.lock().unwrap().iter().any(|call| {
        call.effect && call.stage == TaskEvidenceDeliveryStage::Push && !call.allowed
    }));
}

#[test]
fn auto_cannot_use_public_opt_in_or_a_different_private_push_repository() {
    for public in [true, false] {
        let github = FakeGithubAutomationPort::with_repository_visibility(if public {
            GithubRepositoryVisibility::Public
        } else {
            GithubRepositoryVisibility::Private
        });
        if !public {
            *github.repository_identity.lock().unwrap() = "OtherOwner/private-fork".into();
        }
        let fixture = AutoFixture::new(
            "auto-delivery-target-binding",
            TaskEvidenceDeliveryMode::Enforce,
            true,
            github,
        );
        fixture.prepare();

        let result = fixture
            .inner
            .service
            .process_distributor_queue(&fixture.inner.repo.workspace_dir());

        fixture.inner.assert_no_source_or_pr();
        assert!(
            result.is_err()
                || fixture.inner.queue().queue_state == ParallelModeQueueItemState::Blocked,
            "public={public}: {result:?}"
        );
    }
}

#[test]
fn auto_never_falls_back_to_direct_integration_when_prs_are_disabled_or_unavailable() {
    for disabled in [true, false] {
        let mut github = FakeGithubAutomationPort::ready();
        if !disabled {
            github.capabilities.gh_binary.state = ParallelModeCapabilityState::Blocked;
        }
        let fixture = AutoFixture::new(
            "auto-delivery-pr-required",
            TaskEvidenceDeliveryMode::Enforce,
            true,
            github,
        );
        run_git(
            &fixture.inner.repo.repo_root,
            &[
                "config",
                "akra.githubPrMode",
                if disabled { "disabled" } else { "auto" },
            ],
        );
        fixture.prepare();

        fixture.inner.process();

        fixture.assert_no_integration();
        assert!(
            fixture
                .inner
                .operations()
                .iter()
                .any(|op| op.starts_with("push:"))
        );
        assert!(
            fixture
                .inner
                .operations()
                .iter()
                .all(|op| !op.starts_with("ensure-pr:"))
        );
        assert!(
            fixture
                .inner
                .queue()
                .integration_note
                .contains("issue Auto integration")
        );
    }
}

#[test]
fn auto_rechecks_required_checks_and_clean_state_before_the_exact_integration_push() {
    for checks_failed in [true, false] {
        let github = FakeGithubAutomationPort::with_review_decision("REVIEW_REQUIRED");
        *github.pre_push_inspect_overrides.lock().unwrap() =
            Some(FakePullRequestReadinessOverrides {
                required_checks_passed: checks_failed.then_some(false),
                merge_state_status: (!checks_failed).then(|| "DIRTY".into()),
                ..Default::default()
            });
        let fixture = AutoFixture::new(
            "auto-delivery-final-checks",
            TaskEvidenceDeliveryMode::Enforce,
            true,
            github,
        );
        fixture.prepare();

        fixture.inner.process();

        fixture.assert_no_integration();
        assert!(
            fixture
                .inner
                .operations()
                .iter()
                .any(|op| op.starts_with("ensure-pr:"))
        );
        let note = fixture.inner.queue().integration_note;
        assert!(
            note.contains("pre-push pull request gate changed"),
            "{note}"
        );
        assert!(
            note.contains(if checks_failed {
                "required checks"
            } else {
                "CLEAN"
            }),
            "{note}"
        );
    }
}

struct ChangedAuthorityGate {
    inner: Arc<dyn TaskDeliveryEvidenceGatePort>,
    changed: AtomicBool,
}

impl TaskDeliveryEvidenceGatePort for ChangedAuthorityGate {
    fn evaluate(
        &self,
        workspace: &str,
        record: &PlanningAuthorityDistributorQueueRecord,
        stage: TaskEvidenceDeliveryStage,
        integration: Option<&TaskDeliveryEvidenceIntegration>,
        operation: &TaskDeliveryEvidenceOperation,
    ) -> Result<TaskDeliveryEvidenceDecision, String> {
        if let TaskDeliveryEvidenceOperation::Effect {
            preflight_permit, ..
        } = operation
            && stage == TaskEvidenceDeliveryStage::Push
            && !self.changed.swap(true, Ordering::SeqCst)
        {
            assert_eq!(*preflight_permit, PlanningWorkPermitDecision::GrantedAuto);
            let mut basis = SqlitePlanningAuthorityAdapter::new()
                .load_work_order_permit_basis(workspace, WORK)
                .unwrap()
                .unwrap();
            basis.gate_revision += 1;
            let review = basis.delivery_review.as_mut().unwrap();
            review.gate_revision = basis.gate_revision;
            review.authority = Some(PlanningWorkDeliveryAuthority::HumanReview);
            grant(workspace, &basis, &[PlanningWorkPermitStage::Push]);
        }
        self.inner
            .evaluate(workspace, record, stage, integration, operation)
    }
}

#[test]
fn an_authority_change_during_the_final_evidence_gate_cannot_reuse_the_old_preflight() {
    let fixture = AutoFixture::new(
        "auto-delivery-authority-race",
        TaskEvidenceDeliveryMode::Enforce,
        true,
        FakeGithubAutomationPort::ready(),
    );
    fixture.prepare();
    let gate = Arc::new(ChangedAuthorityGate {
        inner: fixture.inner.gate.clone(),
        changed: AtomicBool::new(false),
    });
    let service = fixture
        .inner
        .service
        .clone()
        .with_delivery_evidence(gate.clone());

    let result = service.process_distributor_queue(&fixture.inner.repo.workspace_dir());

    assert!(
        gate.changed.load(Ordering::SeqCst),
        "final boundary was not reached: {result:?}"
    );
    assert!(
        result.is_err(),
        "a changed authority must fail the effect preflight: {result:?}"
    );
    fixture.inner.assert_no_source_or_pr();
}
