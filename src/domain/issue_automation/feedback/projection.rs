//! 확인된 자기 진행 라벨의 정확한 이벤트만 판정 입력에서 제외한다.
//! 같은 계정의 다른 이벤트나 현재 신원을 확인할 수 없는 관찰은 그대로 보존한다.
use crate::domain::issue_automation::feedback::{
    ISSUE_FEEDBACK_PROGRESS_LABEL, IssueFeedbackEvidence, IssueFeedbackReceipt,
    validate_feedback_id, validate_feedback_issue,
};
use crate::domain::issue_automation::policy::IssueAutomationPolicy;
use crate::domain::issue_automation::source::{
    IssueMetadataCompleteness, IssueSourceFailure, IssueSourceObservation,
};

pub fn suppress_owned_progress_label(
    observation: &mut IssueSourceObservation,
    proof: &IssueFeedbackReceipt,
    policy: &IssueAutomationPolicy,
) -> Result<(), IssueSourceFailure> {
    if !safe_progress_label(policy) {
        return Ok(());
    }
    validate_feedback_issue(&proof.issue).map_err(|_| IssueSourceFailure::IdentityMismatch)?;
    validate_feedback_id(&proof.intent_id).map_err(|_| IssueSourceFailure::IdentityMismatch)?;
    validate_feedback_id(&proof.actor_id).map_err(|_| IssueSourceFailure::IdentityMismatch)?;
    if observation.issue != proof.issue {
        return Err(IssueSourceFailure::IdentityMismatch);
    }
    let IssueFeedbackEvidence::Label {
        label,
        event_id,
        applied: true,
    } = &proof.evidence
    else {
        return Err(IssueSourceFailure::IdentityMismatch);
    };
    validate_feedback_id(&label.id).map_err(|_| IssueSourceFailure::IdentityMismatch)?;
    validate_feedback_id(event_id).map_err(|_| IssueSourceFailure::IdentityMismatch)?;
    if label.name != ISSUE_FEEDBACK_PROGRESS_LABEL {
        return Err(IssueSourceFailure::IdentityMismatch);
    }
    let count = observation
        .labels
        .items
        .iter()
        .filter(|current| *current == label)
        .count();
    if count == 0 {
        return Ok(());
    }
    if count != 1 {
        return Err(IssueSourceFailure::MalformedResponse);
    }
    if observation.labels.completeness != IssueMetadataCompleteness::Complete
        || observation.label_actors.completeness != IssueMetadataCompleteness::Complete
    {
        return Err(IssueSourceFailure::MetadataIncomplete);
    }
    let attributions: Vec<_> = observation
        .label_actors
        .items
        .iter()
        .filter(|current| current.label.id == label.id)
        .collect();
    if attributions.len() > 1 {
        return Err(IssueSourceFailure::MalformedResponse);
    }
    let Some(attribution) = attributions.first() else {
        return Ok(());
    };
    if attribution.label != *label
        || attribution.event_id != *event_id
        || attribution
            .actor
            .as_ref()
            .is_none_or(|actor| actor.id != proof.actor_id)
    {
        return Ok(());
    }
    observation.labels.items.retain(|current| current != label);
    observation
        .label_actors
        .items
        .retain(|current| current.label != *label);
    Ok(())
}

pub fn safe_progress_label(policy: &IssueAutomationPolicy) -> bool {
    let target = ISSUE_FEEDBACK_PROGRESS_LABEL;
    let collides = |value: &str| value.eq_ignore_ascii_case(target);
    ![
        &policy.labels.auto,
        &policy.labels.gated,
        &policy.labels.manual,
        &policy.labels.hold,
    ]
    .into_iter()
    .any(|name| collides(name))
        && !policy.select.labels_any.iter().any(|name| collides(name))
        && !policy
            .classify
            .rule
            .iter()
            .any(|rule| rule.labels_any.iter().any(|name| collides(name)))
        && !policy
            .human_required
            .additional
            .iter()
            .any(|rule| rule.labels_any.iter().any(|name| collides(name)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issue_automation::source::{
        IssueContentWithheldReason, IssueCreationProvenance, IssueLabelAttribution,
        IssueMetadataList, IssuePrPresence, IssueRemoteState, IssueSourceActor,
        IssueSourceActorKind, IssueSourceContent, IssueSourceLabel,
    };
    use crate::domain::issue_work::{
        IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider, IssueReference,
    };

    fn complete<T>(items: Vec<T>) -> IssueMetadataList<T> {
        IssueMetadataList {
            items,
            completeness: IssueMetadataCompleteness::Complete,
        }
    }
    fn baseline() -> IssueSourceObservation {
        IssueSourceObservation {
            content_problems: vec![],
            jira: None,
            repository_visibility: Default::default(),
            repository_name_with_owner: None,
            issue: IssueReference {
                provider: IssueProvider::GitHub,
                server_id: "github.com".into(),
                repository_id: "R_stable".into(),
                issue_id: "I_stable".into(),
                project_scope: IssueProjectScope {
                    workspace_dir: "/work/project".into(),
                },
            },
            web: None,
            remote_state: IssueRemoteState::Open,
            creation_provenance: IssueCreationProvenance::Unattributed,
            labels: complete(vec![IssueSourceLabel {
                id: "L_human".into(),
                name: "akra:manual".into(),
            }]),
            assignees: complete(Vec::new()),
            author: None,
            label_actors: complete(Vec::new()),
            linked_pull_request: IssuePrPresence::Unknown,
            content: IssueSourceContent::Withheld {
                basis: IssueComparisonBasis {
                    field_hash: IssueFieldHash::of_fields("private", "private"),
                    provider_updated_at: "2026-11-21T00:00:00Z".into(),
                },
                policy_version: "a".repeat(64),
                reason: IssueContentWithheldReason::UserConfirmationRequired,
            },
        }
    }
    fn own_label() -> (IssueSourceObservation, IssueFeedbackReceipt) {
        let mut observed = baseline();
        let label = IssueSourceLabel {
            id: "L_progress".into(),
            name: ISSUE_FEEDBACK_PROGRESS_LABEL.into(),
        };
        observed.labels.items.push(label.clone());
        observed.label_actors.items.push(IssueLabelAttribution {
            label: label.clone(),
            event_id: "E_own".into(),
            actor: Some(IssueSourceActor {
                id: "A_stable".into(),
                login: "operator".into(),
                kind: IssueSourceActorKind::UserAccount,
            }),
            created_at: "2026-11-21T01:00:00Z".into(),
        });
        let proof = IssueFeedbackReceipt {
            issue: observed.issue.clone(),
            intent_id: "intent-1".into(),
            actor_id: "A_stable".into(),
            evidence: IssueFeedbackEvidence::Label {
                label,
                event_id: "E_own".into(),
                applied: true,
            },
        };
        (observed, proof)
    }

    #[test]
    fn exact_confirmed_effect_restores_the_prior_semantic_observation() {
        let (mut observed, proof) = own_label();
        suppress_owned_progress_label(&mut observed, &proof, &IssueAutomationPolicy::builtin())
            .unwrap();
        assert_eq!(observed, baseline());
    }

    #[test]
    fn a_later_human_event_from_the_same_account_is_preserved() {
        let (mut observed, proof) = own_label();
        observed.label_actors.items[0].event_id = "E_human_later".into();
        let before = observed.clone();
        suppress_owned_progress_label(&mut observed, &proof, &IssueAutomationPolicy::builtin())
            .unwrap();
        assert_eq!(observed, before);
    }

    #[test]
    fn another_actor_or_label_identity_is_not_owned() {
        let (observed, proof) = own_label();
        let mut different_actor = observed.clone();
        different_actor.label_actors.items[0]
            .actor
            .as_mut()
            .unwrap()
            .id = "A_other".into();
        let mut different_label = observed;
        different_label.labels.items[1].id = "L_recreated".into();
        for mut changed in [different_actor, different_label] {
            let before = changed.clone();
            suppress_owned_progress_label(&mut changed, &proof, &IssueAutomationPolicy::builtin())
                .unwrap();
            assert_eq!(changed, before);
        }
    }

    #[test]
    fn incomplete_latest_event_history_cannot_suppress_a_label() {
        let (mut observed, proof) = own_label();
        observed.label_actors.completeness = IssueMetadataCompleteness::Truncated;
        let before = observed.clone();
        assert_eq!(
            suppress_owned_progress_label(&mut observed, &proof, &IssueAutomationPolicy::builtin()),
            Err(IssueSourceFailure::MetadataIncomplete)
        );
        assert_eq!(observed, before);
    }

    #[test]
    fn a_proof_cannot_cross_remote_or_workspace_identity() {
        let (observed, proof) = own_label();
        for coordinate in 0..4 {
            let mut changed = observed.clone();
            match coordinate {
                0 => changed.issue.server_id = "other.example".into(),
                1 => changed.issue.repository_id = "R_other".into(),
                2 => changed.issue.issue_id = "I_other".into(),
                _ => changed.issue.project_scope.workspace_dir = "/work/alias".into(),
            }
            let before = changed.clone();
            assert_eq!(
                suppress_owned_progress_label(
                    &mut changed,
                    &proof,
                    &IssueAutomationPolicy::builtin()
                ),
                Err(IssueSourceFailure::IdentityMismatch)
            );
            assert_eq!(changed, before);
        }
    }

    #[test]
    fn changed_control_or_selection_policy_preserves_the_owned_label() {
        for field in 0..7 {
            let (mut observed, proof) = own_label();
            let before = observed.clone();
            let mut policy = IssueAutomationPolicy::builtin();
            match field {
                0 => policy.labels.hold = ISSUE_FEEDBACK_PROGRESS_LABEL.into(),
                1 => policy.labels.manual = ISSUE_FEEDBACK_PROGRESS_LABEL.into(),
                2 => policy.labels.auto = ISSUE_FEEDBACK_PROGRESS_LABEL.into(),
                3 => policy.labels.gated = ISSUE_FEEDBACK_PROGRESS_LABEL.into(),
                4 => policy
                    .select
                    .labels_any
                    .push(ISSUE_FEEDBACK_PROGRESS_LABEL.into()),
                5 => policy.classify.rule.push(
                    crate::domain::issue_automation::policy::IssueAutomationRiskRule {
                        class:
                            crate::domain::issue_automation::policy::IssueAutomationRiskClass::High,
                        labels_any: vec![ISSUE_FEEDBACK_PROGRESS_LABEL.into()],
                        paths_any: Vec::new(),
                    },
                ),
                _ => policy.human_required.additional.push(
                    crate::domain::issue_automation::policy::IssueAutomationAdditionalHumanRule {
                        labels_any: vec![ISSUE_FEEDBACK_PROGRESS_LABEL.into()],
                        paths_any: Vec::new(),
                        gates: vec![
                            crate::domain::issue_automation::policy::IssueAutomationGate::Start,
                        ],
                    },
                ),
            }
            suppress_owned_progress_label(&mut observed, &proof, &policy).unwrap();
            assert_eq!(observed, before);
        }
    }

    #[test]
    fn a_remove_receipt_does_not_hide_a_new_addition() {
        let (mut observed, mut proof) = own_label();
        if let IssueFeedbackEvidence::Label { applied, .. } = &mut proof.evidence {
            *applied = false;
        }
        let before = observed.clone();
        assert_eq!(
            suppress_owned_progress_label(&mut observed, &proof, &IssueAutomationPolicy::builtin()),
            Err(IssueSourceFailure::IdentityMismatch)
        );
        assert_eq!(observed, before);
    }
}
