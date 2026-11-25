use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::policy::{
    IssueAutomationGate, IssueAutomationPolicy, IssueAutomationRiskClass,
    IssueAutomationSelectionMode,
};
use crate::domain::issue_automation::source::{
    IssueCreationProvenance, IssueMetadataCompleteness, IssuePrPresence, IssueRemoteState,
    IssueRepositoryVisibility, IssueSourceActorKind, IssueSourceBoardRead, IssueSourceContent,
    IssueSourceContentProblem, IssueSourceObservation, IssueSourceRead, JiraPullRequestLookup,
};

use super::readiness::{budget_exhausted, held_or_manual, label_cap};
use super::{
    IssueReadinessInput, IssueTriageEligibility, IssueTriageError, IssueTriageInput,
    IssueTriageReadiness, IssueTriageReasonCode, inspect_issue_readiness,
};

pub(super) struct Evaluation {
    pub eligibility: IssueTriageEligibility,
    pub readiness: IssueTriageReadiness,
    pub risk: IssueAutomationRiskClass,
    pub effective_mode: IssueAutomationAutonomy,
    pub assumed_mode: IssueAutomationAutonomy,
    pub force_human: bool,
    pub human_gates: Vec<IssueAutomationGate>,
    pub reasons: Vec<IssueTriageReasonCode>,
}

pub(super) fn evaluate(input: &IssueTriageInput<'_>) -> Result<Evaluation, IssueTriageError> {
    let mut result = evaluate_observation(input)?;
    restrict_auto_repository(input.current, &mut result);
    Ok(result)
}

fn restrict_auto_repository(current: Option<&IssueSourceRead>, result: &mut Evaluation) {
    let allowed = matches!(current, Some(IssueSourceRead::Observed(observation))
        if matches!(observation.repository_visibility,
            IssueRepositoryVisibility::Private | IssueRepositoryVisibility::Internal)
            && observation.repository_name_with_owner.is_some()
            && observation.validate_repository_metadata().is_ok());
    if allowed
        || (result.effective_mode != IssueAutomationAutonomy::Auto
            && result.assumed_mode != IssueAutomationAutonomy::Auto)
    {
        return;
    }
    // Preserve existing non-Auto execution modes. The counterfactual report must not imply
    // public/unknown Auto, and it must never fall below the effective Gated mode.
    if result.effective_mode == IssueAutomationAutonomy::Auto {
        result.effective_mode = IssueAutomationAutonomy::Propose;
    }
    result.assumed_mode = result
        .assumed_mode
        .min(IssueAutomationAutonomy::Propose)
        .max(result.effective_mode);
    result
        .reasons
        .push(IssueTriageReasonCode::AutoRepositoryRestricted);
}

fn evaluate_observation(input: &IssueTriageInput<'_>) -> Result<Evaluation, IssueTriageError> {
    let mut result = Evaluation {
        eligibility: IssueTriageEligibility::Unknown,
        readiness: IssueTriageReadiness::Unknown,
        risk: IssueAutomationRiskClass::High,
        effective_mode: input.policy.mode_for_provider(input.issue.provider),
        assumed_mode: input.policy.authority.autonomy_cap,
        force_human: true,
        human_gates: Vec::new(),
        reasons: if budget_exhausted(input.policy) {
            vec![IssueTriageReasonCode::BudgetExhausted]
        } else {
            Vec::new()
        },
    };
    let observation = match input.current {
        None => {
            result.reasons.push(IssueTriageReasonCode::NotYetObserved);
            return Ok(result);
        }
        Some(IssueSourceRead::Unknown { .. }) => {
            result
                .reasons
                .push(IssueTriageReasonCode::SourceUnavailable);
            return Ok(result);
        }
        Some(IssueSourceRead::Observed(observation)) => observation,
    };
    let labels: Vec<_> = observation
        .labels
        .items
        .iter()
        .map(|label| label.name.clone())
        .collect();
    let cap = label_cap(&labels, input.policy);
    result.effective_mode = result.effective_mode.min(cap);
    result.assumed_mode = result.assumed_mode.min(cap);
    result.eligibility = eligibility(observation, input.policy, &mut result.reasons);
    if held_or_manual(&labels, input.policy, input.held) {
        result.reasons.push(IssueTriageReasonCode::HeldOrManual);
    }
    let (disclosure_version, copy) = match &observation.content {
        IssueSourceContent::Withheld { policy_version, .. } => {
            if policy_version != input.validation_policy_version {
                result
                    .reasons
                    .push(IssueTriageReasonCode::DisclosurePolicyStale);
            } else {
                result.readiness = IssueTriageReadiness::NeedsInfo;
                result
                    .reasons
                    .push(IssueTriageReasonCode::DisclosureConfirmationRequired);
            }
            return Ok(result);
        }
        IssueSourceContent::Shared(snapshot) => (&snapshot.policy_version, &snapshot.shared_copy),
    };
    if disclosure_version != input.validation_policy_version {
        result
            .reasons
            .push(IssueTriageReasonCode::DisclosurePolicyStale);
        return Ok(result);
    }
    let readiness = inspect_issue_readiness(IssueReadinessInput {
        provider: observation.issue.provider,
        shared_copy: copy,
        requirements: input.requirements,
        policy: input.policy,
        labels: &labels,
        labels_complete: observation.labels.completeness == IssueMetadataCompleteness::Complete,
        held: input.held,
    })?;
    result.readiness = readiness.readiness;
    result.risk = readiness.risk_class;
    result.effective_mode = readiness.effective_mode;
    result.assumed_mode = readiness.assumed_mode;
    result.force_human = readiness.force_human;
    result.human_gates = readiness.human_gates;
    result.reasons.extend(readiness.reasons);
    for problem in &observation.content_problems {
        result.readiness = IssueTriageReadiness::NeedsInfo;
        result.force_human = true;
        result.reasons.push(match problem {
            IssueSourceContentProblem::MissingRequiredField => {
                IssueTriageReasonCode::RequiredSectionMissing
            }
            IssueSourceContentProblem::UnsupportedAdf
            | IssueSourceContentProblem::UnsupportedRequiredField
            | IssueSourceContentProblem::OversizedContent => {
                IssueTriageReasonCode::UnsupportedContent
            }
        });
    }
    Ok(result)
}

fn eligibility(
    observation: &IssueSourceObservation,
    policy: &IssueAutomationPolicy,
    reasons: &mut Vec<IssueTriageReasonCode>,
) -> IssueTriageEligibility {
    let mut excluded = false;
    let mut unknown = false;
    use crate::domain::issue_work::IssueProvider;
    let provider_metadata_valid = match observation.issue.provider {
        IssueProvider::GitHub => true,
        IssueProvider::Jira => {
            observation.jira.is_some() && observation.validate_repository_metadata().is_ok()
        }
        IssueProvider::Linear => {
            observation.linear.is_some() && observation.validate_repository_metadata().is_ok()
        }
    };
    if !provider_metadata_valid {
        unknown = true;
    }
    if observation.remote_state == IssueRemoteState::Closed {
        excluded = true;
        reasons.push(IssueTriageReasonCode::IssueClosed);
    }
    match selected(observation, policy) {
        Some(true) => {}
        Some(false) => {
            excluded = true;
            reasons.push(IssueTriageReasonCode::SelectionNotMatched);
        }
        None => unknown = true,
    }
    match board_selected(observation, policy) {
        Some(true) => {}
        Some(false) => {
            excluded = true;
            if !reasons.contains(&IssueTriageReasonCode::SelectionNotMatched) {
                reasons.push(IssueTriageReasonCode::SelectionNotMatched);
            }
        }
        None => {
            unknown = true;
            reasons.push(IssueTriageReasonCode::BoardUnavailable);
        }
    }
    let requires_human_selection = observation.creation_provenance
        != IssueCreationProvenance::Unattributed
        || observation
            .author
            .as_ref()
            .is_none_or(|actor| actor.kind != IssueSourceActorKind::UserAccount);
    let trusted = if requires_human_selection {
        trusted_label(observation, policy, true)
    } else {
        trusted(observation, policy)
    };
    if requires_human_selection && trusted != Some(true) {
        // 작성자 계정이 신뢰 목록에 있어도 에이전트의 자기 선택은 사람의 선택이 아니다.
        excluded = true;
        unknown |= trusted.is_none();
        reasons.push(IssueTriageReasonCode::HumanSelectionRequired);
    } else {
        match trusted {
            Some(true) => {}
            Some(false) => {
                excluded = true;
                reasons.push(IssueTriageReasonCode::UntrustedActor);
            }
            None => unknown = true,
        }
    }
    if !observation.assignees.items.is_empty()
        || observation.linked_pull_request == IssuePrPresence::Present
    {
        excluded = true;
        reasons.push(IssueTriageReasonCode::AlreadyClaimed);
    }
    if observation.assignees.completeness != IssueMetadataCompleteness::Complete {
        unknown = true;
    }
    let provider_lookup_unsupported = match observation.issue.provider {
        IssueProvider::GitHub => false,
        IssueProvider::Jira => observation.jira.as_ref().is_some_and(|jira| {
            jira.pull_request_lookup == JiraPullRequestLookup::Unsupported
                && jira.validate_for(&observation.issue).is_ok()
        }),
        // Linear's attachments depend on an optional integration and cannot prove absence.
        IssueProvider::Linear => observation
            .linear
            .as_ref()
            .is_some_and(|linear| linear.validate_for(&observation.issue).is_ok()),
    };
    let unsupported_pr_lookup = provider_metadata_valid
        && observation.linked_pull_request == IssuePrPresence::Unknown
        && provider_lookup_unsupported;
    if unsupported_pr_lookup
        && let Some(reason) =
            IssueTriageReasonCode::pull_request_lookup_unsupported(observation.issue.provider)
    {
        reasons.push(reason);
    } else if observation.linked_pull_request == IssuePrPresence::Unknown {
        unknown = true;
    }
    if unknown {
        reasons.push(IssueTriageReasonCode::MetadataIncomplete);
    }
    if excluded {
        IssueTriageEligibility::Ineligible
    } else if unknown || unsupported_pr_lookup {
        IssueTriageEligibility::Unknown
    } else {
        IssueTriageEligibility::Eligible
    }
}

fn selected(observation: &IssueSourceObservation, policy: &IssueAutomationPolicy) -> Option<bool> {
    if policy.select.mode == IssueAutomationSelectionMode::AllOpen {
        return Some(true);
    }
    if policy.select.labels_any.is_empty() {
        return Some(false);
    }
    if observation.labels.completeness == IssueMetadataCompleteness::Unavailable {
        return None;
    }
    if observation
        .labels
        .items
        .iter()
        .any(|label| policy.select.labels_any.contains(&label.name))
    {
        return Some(true);
    }
    (observation.labels.completeness == IssueMetadataCompleteness::Complete).then_some(false)
}

/// 보드 열 조건은 라벨 조건과 AND 이다. 읽지 못한 보드와 다른 보드의 관찰은 None 이다 (#2565).
fn board_selected(
    observation: &IssueSourceObservation,
    policy: &IssueAutomationPolicy,
) -> Option<bool> {
    let Some(board) = policy
        .binding
        .github
        .board
        .as_ref()
        .filter(|board| !board.select_option_ids.is_empty())
    else {
        return Some(true);
    };
    if observation.issue.provider != crate::domain::issue_work::IssueProvider::GitHub {
        return Some(true);
    }
    let column = observation.board.as_ref()?;
    if column.project_id != board.project_id || column.field_id != board.field_id {
        return None;
    }
    match &column.read {
        IssueSourceBoardRead::NotOnBoard => Some(false),
        IssueSourceBoardRead::Item { option_id, .. } => Some(
            option_id
                .as_ref()
                .is_some_and(|id| board.select_option_ids.contains(id)),
        ),
        IssueSourceBoardRead::Unavailable => None,
    }
}

fn actor_trusted(
    observation: &IssueSourceObservation,
    policy: &IssueAutomationPolicy,
    id: &str,
) -> bool {
    policy.select.trusted_actors.iter().any(|actor| {
        actor.provider == observation.issue.provider
            && actor.server_id == observation.issue.server_id
            && actor.actor_id == id
    })
}

fn trusted(observation: &IssueSourceObservation, policy: &IssueAutomationPolicy) -> Option<bool> {
    if observation.author.as_ref().is_some_and(|actor| {
        actor.kind == IssueSourceActorKind::UserAccount
            && actor_trusted(observation, policy, &actor.id)
    }) {
        return Some(true);
    }
    trusted_label(observation, policy, false)
}

fn trusted_label(
    observation: &IssueSourceObservation,
    policy: &IssueAutomationPolicy,
    require_selection_label: bool,
) -> Option<bool> {
    if policy.select.trusted_actors.is_empty() {
        return Some(false);
    }
    if observation.labels.completeness == IssueMetadataCompleteness::Unavailable
        || observation.label_actors.completeness == IssueMetadataCompleteness::Unavailable
    {
        return None;
    }
    let mut ambiguous = false;
    for label in &observation.labels.items {
        if (require_selection_label
            || policy.select.mode == IssueAutomationSelectionMode::LabelsAny)
            && !policy.select.labels_any.contains(&label.name)
        {
            continue;
        }
        let mut events: Vec<_> = observation
            .label_actors
            .items
            .iter()
            .filter(|event| event.label.id == label.id && event.label.name == label.name)
            .map(|event| {
                (
                    event.event_id.as_str(),
                    event
                        .actor
                        .as_ref()
                        .map(|actor| (actor.id.as_str(), actor.kind)),
                )
            })
            .collect();
        events.sort();
        events.dedup();
        if events.len() > 1 {
            ambiguous = true;
            continue;
        }
        if events
            .first()
            .and_then(|(_, actor)| *actor)
            .is_some_and(|(id, kind)| {
                kind == IssueSourceActorKind::UserAccount && actor_trusted(observation, policy, id)
            })
        {
            return Some(true);
        }
    }
    if ambiguous
        || observation.labels.completeness != IssueMetadataCompleteness::Complete
        || observation.label_actors.completeness != IssueMetadataCompleteness::Complete
    {
        None
    } else {
        Some(false)
    }
}
