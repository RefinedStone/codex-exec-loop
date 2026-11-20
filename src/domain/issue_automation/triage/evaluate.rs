use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::policy::{
    IssueAutomationGate, IssueAutomationPolicy, IssueAutomationRiskClass,
    IssueAutomationSelectionMode,
};
use crate::domain::issue_automation::source::{
    IssueMetadataCompleteness, IssuePrPresence, IssueRemoteState, IssueSourceContent,
    IssueSourceObservation, IssueSourceRead,
};
use crate::domain::issue_validation::{IssueValidationField, validate_issue};

use super::criteria::{CriteriaProblem, count_criteria};
use super::paths::{MentionedPaths, matches_glob, mentioned_paths};
use super::{
    IssueTriageEligibility, IssueTriageInput, IssueTriageReadiness, IssueTriageReasonCode,
    MAX_ISSUE_TRIAGE_BODY_BYTES, MAX_ISSUE_TRIAGE_TITLE_BYTES,
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

pub(super) fn evaluate(input: &IssueTriageInput<'_>) -> Evaluation {
    // 동적 사용량을 추측하지 않는다. 정책의 0 상한은 현재 입력만으로 소진을 확정할 수 있다.
    let budget_exhausted = input.policy.budget.max_running == 0
        || input.policy.budget.max_started_per_day == 0
        || input.policy.budget.max_attempts == 0;
    let mut result = Evaluation {
        eligibility: IssueTriageEligibility::Unknown,
        readiness: IssueTriageReadiness::Unknown,
        risk: IssueAutomationRiskClass::High,
        effective_mode: input.policy.binding.github.mode,
        assumed_mode: input.policy.authority.autonomy_cap,
        force_human: true,
        human_gates: Vec::new(),
        reasons: if budget_exhausted {
            vec![IssueTriageReasonCode::BudgetExhausted]
        } else {
            Vec::new()
        },
    };
    let observation = match input.current {
        None => {
            result.reasons.push(IssueTriageReasonCode::NotYetObserved);
            return result;
        }
        Some(IssueSourceRead::Unknown { .. }) => {
            result
                .reasons
                .push(IssueTriageReasonCode::SourceUnavailable);
            return result;
        }
        Some(IssueSourceRead::Observed(observation)) => observation,
    };
    let label_cap = label_cap(observation);
    result.effective_mode = result.effective_mode.min(label_cap);
    result.assumed_mode = result.assumed_mode.min(label_cap);
    result.eligibility = eligibility(observation, input.policy, &mut result.reasons);
    let held_or_manual =
        input.held || has_label(observation, "akra:hold") || has_label(observation, "akra:manual");
    if held_or_manual {
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
            return result;
        }
        IssueSourceContent::Shared(snapshot) => (&snapshot.policy_version, &snapshot.shared_copy),
    };
    if disclosure_version != input.validation_policy_version {
        result
            .reasons
            .push(IssueTriageReasonCode::DisclosurePolicyStale);
        return result;
    }
    if copy.title.len() > MAX_ISSUE_TRIAGE_TITLE_BYTES
        || copy.body.len() > MAX_ISSUE_TRIAGE_BODY_BYTES
    {
        result.readiness = IssueTriageReadiness::NeedsInfo;
        result.reasons.push(IssueTriageReasonCode::IssueTooLarge);
        return result;
    }
    result.readiness = IssueTriageReadiness::Ready;
    if let Err(errors) = validate_issue(copy, input.requirements) {
        result.readiness = IssueTriageReadiness::NeedsInfo;
        for error in errors {
            result.reasons.push(match error.field {
                IssueValidationField::Title => IssueTriageReasonCode::TitleMissing,
                _ => IssueTriageReasonCode::RequiredSectionMissing,
            });
        }
    }
    if let Err(problem) = count_criteria(&copy.body) {
        result.readiness = IssueTriageReadiness::NeedsInfo;
        result.reasons.push(match problem {
            CriteriaProblem::Missing => IssueTriageReasonCode::CriteriaMissing,
            CriteriaProblem::NotList => IssueTriageReasonCode::CriteriaNotList,
            CriteriaProblem::TooLarge => IssueTriageReasonCode::IssueTooLarge,
            CriteriaProblem::Unsupported => IssueTriageReasonCode::UnsupportedContent,
        });
    }
    let unsupported = copy
        .title
        .chars()
        .chain(copy.body.chars())
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'));
    if unsupported {
        result.readiness = IssueTriageReadiness::NeedsInfo;
        result
            .reasons
            .push(IssueTriageReasonCode::UnsupportedContent);
    }
    let masked = copy.title.contains("[[akra-mask:")
        || copy.title.contains("[removed]")
        || copy.body.contains("[[akra-mask:")
        || copy.body.contains("[removed]");
    if masked {
        result.reasons.push(IssueTriageReasonCode::MaskedContent);
    }
    let paths = mentioned_paths(&copy.body);
    let metadata_incomplete =
        observation.labels.completeness != IssueMetadataCompleteness::Complete;
    result.risk = risk(observation, &paths, input.policy);
    if masked || unsupported || metadata_incomplete || paths.incomplete {
        result.risk = IssueAutomationRiskClass::High;
    }
    if metadata_incomplete {
        result
            .reasons
            .push(IssueTriageReasonCode::MetadataIncomplete);
    }
    if paths.incomplete {
        result
            .reasons
            .push(IssueTriageReasonCode::UnsupportedContent);
    }
    result.force_human =
        budget_exhausted || held_or_manual || result.risk == IssueAutomationRiskClass::High;
    match path_match(&input.policy.protection.paths, &paths) {
        Some(true) => {
            result.force_human = true;
            result.reasons.push(IssueTriageReasonCode::ProtectedPath);
        }
        None => {
            result.force_human = true;
            result.risk = IssueAutomationRiskClass::High;
            result
                .reasons
                .push(IssueTriageReasonCode::UnsupportedContent);
        }
        Some(false) => {}
    }
    for rule in &input.policy.human_required.additional {
        match condition_match(&rule.labels_any, &rule.paths_any, observation, &paths) {
            Some(false) => {}
            Some(true) => {
                result.human_gates.extend(&rule.gates);
                result
                    .reasons
                    .push(IssueTriageReasonCode::PolicyHumanRequired);
            }
            None => {
                result.force_human = true;
                result.risk = IssueAutomationRiskClass::High;
                result
                    .reasons
                    .push(IssueTriageReasonCode::PolicyHumanRequired);
            }
        }
    }
    if result.risk == IssueAutomationRiskClass::High {
        result.reasons.push(IssueTriageReasonCode::HighRisk);
    }
    result
}

fn eligibility(
    observation: &IssueSourceObservation,
    policy: &IssueAutomationPolicy,
    reasons: &mut Vec<IssueTriageReasonCode>,
) -> IssueTriageEligibility {
    let mut excluded = false;
    let mut unknown = false;
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
    match trusted(observation, policy) {
        Some(true) => {}
        Some(false) => {
            excluded = true;
            reasons.push(IssueTriageReasonCode::UntrustedActor);
        }
        None => unknown = true,
    }
    if !observation.assignees.items.is_empty()
        || observation.linked_pull_request == IssuePrPresence::Present
    {
        excluded = true;
        reasons.push(IssueTriageReasonCode::AlreadyClaimed);
    }
    if observation.assignees.completeness != IssueMetadataCompleteness::Complete
        || observation.linked_pull_request == IssuePrPresence::Unknown
    {
        unknown = true;
    }
    if unknown {
        reasons.push(IssueTriageReasonCode::MetadataIncomplete);
    }
    if excluded {
        IssueTriageEligibility::Ineligible
    } else if unknown {
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

fn trusted(observation: &IssueSourceObservation, policy: &IssueAutomationPolicy) -> Option<bool> {
    if policy.select.trusted_actors.is_empty() {
        return Some(false);
    }
    let actor_trusted = |id: &str| {
        policy.select.trusted_actors.iter().any(|actor| {
            actor.provider == observation.issue.provider
                && actor.server_id == observation.issue.server_id
                && actor.actor_id == id
        })
    };
    if observation
        .author
        .as_ref()
        .is_some_and(|actor| actor_trusted(&actor.id))
    {
        return Some(true);
    }
    if observation.labels.completeness == IssueMetadataCompleteness::Unavailable
        || observation.label_actors.completeness == IssueMetadataCompleteness::Unavailable
    {
        return None;
    }
    let mut ambiguous = false;
    for label in &observation.labels.items {
        if policy.select.mode == IssueAutomationSelectionMode::LabelsAny
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
                    event.actor.as_ref().map(|actor| actor.id.as_str()),
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
            .is_some_and(actor_trusted)
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

fn risk(
    observation: &IssueSourceObservation,
    paths: &MentionedPaths,
    policy: &IssueAutomationPolicy,
) -> IssueAutomationRiskClass {
    if paths.incomplete || observation.labels.completeness != IssueMetadataCompleteness::Complete {
        return IssueAutomationRiskClass::High;
    }
    for rule in &policy.classify.rule {
        match condition_match(&rule.labels_any, &rule.paths_any, observation, paths) {
            Some(true) => return rule.class,
            Some(false) => {}
            None => return IssueAutomationRiskClass::High,
        }
    }
    IssueAutomationRiskClass::Normal
}

fn condition_match(
    labels: &[String],
    patterns: &[String],
    observation: &IssueSourceObservation,
    paths: &MentionedPaths,
) -> Option<bool> {
    if labels.iter().any(|label| has_label(observation, label)) {
        return Some(true);
    }
    match path_match(patterns, paths) {
        Some(true) => Some(true),
        None => None,
        Some(false)
            if !labels.is_empty()
                && observation.labels.completeness != IssueMetadataCompleteness::Complete =>
        {
            None
        }
        Some(false) => Some(false),
    }
}

fn path_match(patterns: &[String], paths: &MentionedPaths) -> Option<bool> {
    let mut unknown = !patterns.is_empty() && paths.incomplete;
    for pattern in patterns {
        for path in &paths.paths {
            match matches_glob(pattern, path) {
                Some(true) => return Some(true),
                None => unknown = true,
                Some(false) => {}
            }
        }
    }
    if unknown { None } else { Some(false) }
}

fn label_cap(observation: &IssueSourceObservation) -> IssueAutomationAutonomy {
    if has_label(observation, "akra:manual") {
        IssueAutomationAutonomy::Propose
    } else if has_label(observation, "akra:gated") {
        IssueAutomationAutonomy::Gated
    } else {
        IssueAutomationAutonomy::Auto
    }
}

fn has_label(observation: &IssueSourceObservation, name: &str) -> bool {
    observation
        .labels
        .items
        .iter()
        .any(|label| label.name == name)
}
