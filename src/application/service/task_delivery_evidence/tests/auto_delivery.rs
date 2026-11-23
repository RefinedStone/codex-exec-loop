use super::*;
use crate::application::port::outbound::github_automation_port::GithubRepositoryVisibility;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorDeliveryTarget;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkDeliveryAuthority, PlanningWorkDeliveryReview, PlanningWorkPermitBasis,
    PlanningWorkPermitDecision,
};

fn automatic(mode: TaskEvidenceDeliveryMode) -> Fixture {
    let f = Fixture::new(mode);
    f.change(|snapshot| {
        let queue = &mut snapshot.queue;
        queue.delivery_target = Some(PlanningAuthorityDistributorDeliveryTarget::new(
            "origin",
            "owner/repo",
            GithubRepositoryVisibility::Private,
            "main",
        ));
        snapshot.permit = PlanningWorkPermitDecision::GrantedAuto;
        snapshot.permit_basis = Some(PlanningWorkPermitBasis {
            authorization_epoch: 0,
            policy_version: "policy".into(),
            field_hash: "fields".into(),
            gate_revision: 4,
            source_sha: queue.source_commit_sha.clone(),
            delivery_review: Some(PlanningWorkDeliveryReview {
                base_commit_sha: queue.source_base_commit_sha.clone(),
                source_commit_sha: queue.source_commit_sha.clone(),
                gate_revision: 4,
                authority: Some(PlanningWorkDeliveryAuthority::Auto {
                    merge_gate_revision: 2,
                    source_repository: "owner/repo".into(),
                }),
            }),
        });
    });
    f
}

#[test]
fn auto_requires_enforcement_for_effect_and_restart_recheck_even_with_host_success() {
    for mode in [
        TaskEvidenceDeliveryMode::Observe,
        TaskEvidenceDeliveryMode::Enforce,
    ] {
        let f = automatic(mode);
        let result = f.effect().unwrap();
        assert_eq!(result.allowed, mode == TaskEvidenceDeliveryMode::Enforce);
        assert_eq!(f.last().allows_delivery(), result.allowed);
        assert_eq!(
            result.reason,
            (mode == TaskEvidenceDeliveryMode::Observe)
                .then_some(TaskDeliveryEvidenceReason::AutoRequiresEnforce)
        );
    }
    let mut f = automatic(TaskEvidenceDeliveryMode::Enforce);
    assert!(f.effect().unwrap().allowed);
    f.service.mode = Ok(TaskEvidenceDeliveryMode::Observe);
    let queue = f.repository.snapshot.lock().unwrap().queue.clone();
    let recheck = f
        .service
        .evaluate(
            "/repo",
            &queue,
            TaskEvidenceDeliveryStage::Push,
            None,
            &TaskDeliveryEvidenceOperation::Recheck,
        )
        .unwrap();
    assert!(!recheck.allowed);
    assert!(!f.last().allows_delivery());
    assert_eq!(
        recheck.reason,
        Some(TaskDeliveryEvidenceReason::AutoRequiresEnforce)
    );
}

#[test]
fn auto_does_not_upgrade_worker_pass_or_override_current_worker_failure() {
    for status in [
        TaskEvidenceStatus::Pass,
        TaskEvidenceStatus::Fail,
        TaskEvidenceStatus::Blocked,
    ] {
        let f = automatic(TaskEvidenceDeliveryMode::Enforce);
        f.install_report(status);
        if status == TaskEvidenceStatus::Pass {
            f.change(|snapshot| snapshot.task.as_mut().unwrap().host_verifications.clear());
        }
        assert!(!f.effect().unwrap().allowed, "{status:?}");
        assert!(!f.last().satisfied());
    }
}

#[test]
fn auto_requires_current_artifact_environment_and_source_bound_criteria() {
    for case in ["artifact", "environment", "binding", "criteria"] {
        let f = automatic(TaskEvidenceDeliveryMode::Enforce);
        match case {
            "artifact" => f.artifacts.available.store(false, Ordering::SeqCst),
            "environment" => *f.runner.current.lock().unwrap() = Ok("9".repeat(64)),
            "binding" => f.change(|snapshot| snapshot.queue.evidence_binding = None),
            "criteria" => {
                f.change(|snapshot| snapshot.task.as_mut().unwrap().task.acceptance.clear());
                f.synchronize_scope();
            }
            _ => unreachable!(),
        }
        assert!(!f.effect().unwrap().allowed, "{case}");
    }
}

#[test]
fn all_screen_or_unsupported_command_waivers_do_not_authorize_auto_delivery() {
    for channel in [
        TaskAcceptanceChannel::Screen,
        TaskAcceptanceChannel::Command,
    ] {
        let f = automatic(TaskEvidenceDeliveryMode::Enforce);
        f.change(|snapshot| {
            let task = snapshot.task.as_mut().unwrap();
            for criterion in &mut task.task.acceptance {
                criterion.channel = channel;
                criterion.verification_recipe = None;
            }
        });
        f.synchronize_scope();
        f.waive();
        let blocked = f.effect().unwrap();
        assert!(!blocked.allowed);
        assert_eq!(
            blocked.reason,
            Some(TaskDeliveryEvidenceReason::AutoRequiresMachineCriterion)
        );
        assert!(
            f.last()
                .criteria
                .iter()
                .all(|row| row.state == TaskDeliveryCriterionState::Waived)
        );
        f.change(|snapshot| {
            snapshot.permit = PlanningWorkPermitDecision::Granted {
                requires_human_review: true,
            };
            snapshot
                .permit_basis
                .as_mut()
                .unwrap()
                .delivery_review
                .as_mut()
                .unwrap()
                .authority = Some(PlanningWorkDeliveryAuthority::HumanReview);
        });
        assert!(
            f.effect().unwrap().allowed,
            "Human delivery preserves its waiver semantics"
        );
    }
}

#[test]
fn auto_accepts_mixed_case_frozen_targets_without_changing_their_stored_identity() {
    for repository in ["Owner/Repo", "OWNER/REPO"] {
        let f = automatic(TaskEvidenceDeliveryMode::Enforce);
        f.change(|snapshot| {
            snapshot
                .queue
                .delivery_target
                .as_mut()
                .unwrap()
                .github_repository = repository.into();
        });
        let result = f.effect().unwrap();
        assert!(result.allowed);
        assert_eq!(
            result
                .updated_queue
                .delivery_target
                .as_ref()
                .unwrap()
                .github_repository,
            repository
        );
        assert_eq!(
            f.repository
                .snapshot
                .lock()
                .unwrap()
                .queue
                .delivery_target
                .as_ref()
                .unwrap()
                .github_repository,
            repository
        );
    }
}

#[test]
fn auto_raw_authority_repository_and_frozen_visibility_must_match_before_git_reads() {
    for change in [
        "repository",
        "rename",
        "invalid_target",
        "public",
        "missing",
        "human",
        "base",
    ] {
        let f = automatic(TaskEvidenceDeliveryMode::Enforce);
        f.change(|snapshot| match change {
            "repository" | "rename" | "invalid_target" => {
                snapshot
                    .queue
                    .delivery_target
                    .as_mut()
                    .unwrap()
                    .github_repository = match change {
                    "repository" => "Other/Repo",
                    "rename" => "Owner/Renamed",
                    "invalid_target" => "Owner/Repo ",
                    _ => unreachable!(),
                }
                .into()
            }
            "public" => {
                snapshot
                    .queue
                    .delivery_target
                    .as_mut()
                    .unwrap()
                    .repository_visibility = GithubRepositoryVisibility::Public
            }
            "missing" => snapshot.permit_basis = None,
            "human" => {
                snapshot
                    .permit_basis
                    .as_mut()
                    .unwrap()
                    .delivery_review
                    .as_mut()
                    .unwrap()
                    .authority = Some(PlanningWorkDeliveryAuthority::HumanReview)
            }
            "base" => snapshot.queue.source_base_commit_sha = "9".repeat(40),
            _ => unreachable!(),
        });
        assert!(f.effect().is_err(), "{change}");
        assert_eq!(f.runtime.reads.load(Ordering::SeqCst), 0);
        assert_eq!(f.repository.commits.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn auto_preflight_decision_must_match_before_any_evidence_observation() {
    let f = automatic(TaskEvidenceDeliveryMode::Enforce);
    let queue = f.repository.snapshot.lock().unwrap().queue.clone();
    for preflight_permit in [
        PlanningWorkPermitDecision::Legacy,
        PlanningWorkPermitDecision::Granted {
            requires_human_review: false,
        },
        PlanningWorkPermitDecision::Granted {
            requires_human_review: true,
        },
    ] {
        assert!(
            f.service
                .evaluate(
                    "/repo",
                    &queue,
                    TaskEvidenceDeliveryStage::Push,
                    None,
                    &TaskDeliveryEvidenceOperation::Effect {
                        owner_token: "owner".into(),
                        preflight_permit
                    }
                )
                .is_err()
        );
    }
    assert_eq!(f.runtime.reads.load(Ordering::SeqCst), 0);
    assert_eq!(f.repository.commits.load(Ordering::SeqCst), 0);
}

#[test]
fn unchanged_auto_decision_cannot_hide_a_changed_merge_authority_in_the_final_cas() {
    for at_commit in [false, true] {
        let f = automatic(TaskEvidenceDeliveryMode::Enforce);
        let repository = f.repository.clone();
        let hook: Hook = Box::new(move || {
            let mut snapshot = repository.snapshot.lock().unwrap();
            let authority = snapshot
                .permit_basis
                .as_mut()
                .unwrap()
                .delivery_review
                .as_mut()
                .unwrap()
                .authority
                .as_mut()
                .unwrap();
            let PlanningWorkDeliveryAuthority::Auto {
                merge_gate_revision,
                ..
            } = authority
            else {
                panic!("Auto fixture")
            };
            *merge_gate_revision += 1;
            assert_eq!(snapshot.permit, PlanningWorkPermitDecision::GrantedAuto);
        });
        if at_commit {
            *f.repository.on_commit.lock().unwrap() = Some(hook);
        } else {
            *f.repository.on_second.lock().unwrap() = Some(hook);
        }
        assert!(f.effect().is_err());
        assert!(f.repository.observations.lock().unwrap().is_empty());
    }
}
