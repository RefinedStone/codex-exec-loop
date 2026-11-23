use crate::domain::issue_automation::policy::{
    IssueAutomationGate, IssueAutomationGateAction, IssueAutomationGateSet, IssueAutomationPolicy,
    IssueAutomationRiskClass,
};
use crate::domain::issue_automation::{IssueAutomationAutonomy, MAX_ISSUE_AUTOMATION_REVISION};

use super::basis::canonical_hash;
use super::evaluate::Evaluation;
use super::{
    IssueObservationDisposition, IssueTriageAssessment, IssueTriageBasis, IssueTriageEligibility,
    IssueTriageError, IssueTriageGate, IssueTriageObservation, IssueTriageReadiness,
    IssueTriageReasonCode, IssueTriageSourceState, PersistedIssueTriageAssessment,
};

const GATES: [IssueAutomationGate; 6] = [
    IssueAutomationGate::Start,
    IssueAutomationGate::Spec,
    IssueAutomationGate::Plan,
    IssueAutomationGate::Delivery,
    IssueAutomationGate::Merge,
    IssueAutomationGate::Close,
];

pub(super) fn expected(
    readiness: &super::IssueReadinessResult,
    policy: &IssueAutomationPolicy,
) -> Vec<super::IssueExpectedGate> {
    let evaluation = Evaluation {
        eligibility: IssueTriageEligibility::Eligible,
        readiness: readiness.readiness,
        risk: readiness.risk_class,
        effective_mode: readiness.effective_mode,
        assumed_mode: readiness.assumed_mode,
        force_human: readiness.force_human,
        human_gates: readiness.human_gates.clone(),
        reasons: Vec::new(),
    };
    let can_prepare = evaluation.effective_mode >= IssueAutomationAutonomy::Propose
        && evaluation.readiness == IssueTriageReadiness::Ready;
    GATES
        .into_iter()
        .map(|gate| super::IssueExpectedGate {
            gate,
            action: if can_prepare && gate != IssueAutomationGate::Close {
                action(gate, evaluation.effective_mode, &evaluation, policy)
            } else {
                IssueAutomationGateAction::Human
            },
        })
        .collect()
}

pub(super) fn calculate(
    basis: &IssueTriageBasis,
    evaluation: &Evaluation,
    policy: &IssueAutomationPolicy,
    previous: Option<&IssueTriageAssessment>,
) -> Result<Vec<IssueTriageGate>, IssueTriageError> {
    let unchanged =
        previous.is_some_and(|previous| previous.input_fingerprint() == canonical_hash(basis));
    let enabled = evaluation.effective_mode >= IssueAutomationAutonomy::Propose
        && evaluation.eligibility == IssueTriageEligibility::Eligible
        && evaluation.readiness == IssueTriageReadiness::Ready;
    let human_start_only = evaluation.effective_mode >= IssueAutomationAutonomy::Propose
        && super::jira_human_start_review(
            basis,
            evaluation.eligibility,
            evaluation.readiness,
            &evaluation.reasons,
        );
    let mut result = Vec::with_capacity(GATES.len());
    for (index, gate) in GATES.into_iter().enumerate() {
        let gate_revision = match previous {
            None => 1,
            Some(previous) if unchanged => previous.gates()[index].gate_revision,
            Some(previous) => previous.gates()[index]
                .gate_revision
                .checked_add(1)
                .filter(|revision| *revision <= MAX_ISSUE_AUTOMATION_REVISION)
                .ok_or(IssueTriageError::RevisionExhausted)?,
        };
        let enabled = (enabled && gate != IssueAutomationGate::Close)
            || (human_start_only && gate == IssueAutomationGate::Start);
        result.push(IssueTriageGate {
            gate,
            action: if enabled && !human_start_only {
                action(gate, evaluation.effective_mode, evaluation, policy)
            } else {
                IssueAutomationGateAction::Human
            },
            enabled,
            gate_revision,
        });
    }
    Ok(result)
}

pub(super) fn observe(
    basis: &IssueTriageBasis,
    evaluation: &Evaluation,
    policy: &IssueAutomationPolicy,
) -> IssueTriageObservation {
    use IssueObservationDisposition as Disposition;
    let human_start_only = super::jira_human_start_review(
        basis,
        evaluation.eligibility,
        evaluation.readiness,
        &evaluation.reasons,
    );
    let disposition = match (evaluation.eligibility, evaluation.readiness) {
        (IssueTriageEligibility::Ineligible, _) => Disposition::Excluded,
        (_, IssueTriageReadiness::Ready) if human_start_only => {
            if evaluation.assumed_mode >= IssueAutomationAutonomy::Propose {
                Disposition::HumanApproval
            } else {
                Disposition::ObservationOnly
            }
        }
        (_, IssueTriageReadiness::NeedsInfo)
            if evaluation
                .reasons
                .contains(&IssueTriageReasonCode::JiraPullRequestLookupUnsupported) =>
        {
            Disposition::NeedsInfo
        }
        (IssueTriageEligibility::Unknown, _) | (_, IssueTriageReadiness::Unknown) => {
            Disposition::Unknown
        }
        (_, IssueTriageReadiness::NeedsInfo) => Disposition::NeedsInfo,
        (IssueTriageEligibility::Eligible, IssueTriageReadiness::Ready) => {
            if evaluation.assumed_mode <= IssueAutomationAutonomy::Observe {
                Disposition::ObservationOnly
            } else {
                match action(
                    IssueAutomationGate::Start,
                    evaluation.assumed_mode,
                    evaluation,
                    policy,
                ) {
                    IssueAutomationGateAction::Human => Disposition::HumanApproval,
                    IssueAutomationGateAction::Auto => Disposition::AutomaticStart,
                    IssueAutomationGateAction::TimeoutDefault
                    | IssueAutomationGateAction::TimeoutArbiter => Disposition::Deferred,
                }
            }
        }
    };
    IssueTriageObservation {
        observation_only: true,
        assumed_mode: evaluation.assumed_mode,
        disposition,
    }
}

fn action(
    gate: IssueAutomationGate,
    mode: IssueAutomationAutonomy,
    evaluation: &Evaluation,
    policy: &IssueAutomationPolicy,
) -> IssueAutomationGateAction {
    use IssueAutomationGateAction::Human;
    // These actions describe the policy. Actual delivery also requires exact work permits
    // and current enforced host evidence; Spec and Close never gain automatic authority.
    if evaluation.force_human
        || evaluation.human_gates.contains(&gate)
        || matches!(gate, IssueAutomationGate::Spec | IssueAutomationGate::Close)
    {
        return Human;
    }
    let risks = match mode {
        IssueAutomationAutonomy::Off | IssueAutomationAutonomy::Observe => return Human,
        IssueAutomationAutonomy::Propose => &policy.gates.propose,
        IssueAutomationAutonomy::Gated => &policy.gates.gated,
        IssueAutomationAutonomy::Auto => &policy.gates.auto,
    };
    let selected: &IssueAutomationGateSet = match evaluation.risk {
        IssueAutomationRiskClass::Low => &risks.low,
        IssueAutomationRiskClass::Normal => &risks.normal,
        IssueAutomationRiskClass::High => &risks.high,
    };
    match gate {
        IssueAutomationGate::Start => selected.start,
        IssueAutomationGate::Plan => selected.plan,
        IssueAutomationGate::Delivery
            if mode == IssueAutomationAutonomy::Auto
                && selected.delivery == IssueAutomationGateAction::Auto =>
        {
            IssueAutomationGateAction::Auto
        }
        IssueAutomationGate::Merge
            if mode == IssueAutomationAutonomy::Auto
                && selected.merge == IssueAutomationGateAction::Auto =>
        {
            IssueAutomationGateAction::Auto
        }
        _ => Human,
    }
}

pub(super) fn validate_record(
    record: &PersistedIssueTriageAssessment,
) -> Result<(), IssueTriageError> {
    let invalid = || IssueTriageError::InvalidAssessment;
    if record.gates.len() != GATES.len()
        || !record.observation.observation_only
        || record.observation.assumed_mode < record.effective_mode
        || (record
            .reasons
            .contains(&IssueTriageReasonCode::AutoRepositoryRestricted)
            && (record.effective_mode == IssueAutomationAutonomy::Auto
                || record.observation.assumed_mode == IssueAutomationAutonomy::Auto))
    {
        return Err(invalid());
    }
    let enabled = record.effective_mode >= IssueAutomationAutonomy::Propose
        && record.eligibility == IssueTriageEligibility::Eligible
        && record.readiness == IssueTriageReadiness::Ready;
    let human_start_only = super::jira_human_start_review(
        &record.basis,
        record.eligibility,
        record.readiness,
        &record.reasons,
    );
    if record
        .reasons
        .contains(&IssueTriageReasonCode::JiraPullRequestLookupUnsupported)
        && (record.basis.issue.provider != crate::domain::issue_work::IssueProvider::Jira
            || matches!(
                record.basis.source,
                IssueTriageSourceState::NotYetObserved | IssueTriageSourceState::Unknown
            )
            || record.eligibility == IssueTriageEligibility::Eligible)
    {
        return Err(invalid());
    }
    let all_human = record.risk_class == IssueAutomationRiskClass::High
        || record.reasons.iter().any(|reason| {
            matches!(
                reason,
                IssueTriageReasonCode::HeldOrManual
                    | IssueTriageReasonCode::ProtectedPath
                    | IssueTriageReasonCode::MaskedContent
                    | IssueTriageReasonCode::BudgetExhausted
            )
        });
    for (expected, gate) in GATES.iter().zip(&record.gates) {
        if gate.gate != *expected
            || gate.gate_revision == 0
            || gate.gate_revision > MAX_ISSUE_AUTOMATION_REVISION
            || gate.enabled
                != ((enabled && *expected != IssueAutomationGate::Close)
                    || (human_start_only
                        && record.effective_mode >= IssueAutomationAutonomy::Propose
                        && *expected == IssueAutomationGate::Start))
            || (human_start_only && gate.action != IssueAutomationGateAction::Human)
            || ((!gate.enabled || all_human) && gate.action != IssueAutomationGateAction::Human)
        {
            return Err(invalid());
        }
        if matches!(
            expected,
            IssueAutomationGate::Spec | IssueAutomationGate::Close
        ) && gate.action != IssueAutomationGateAction::Human
        {
            return Err(invalid());
        }
        if matches!(
            expected,
            IssueAutomationGate::Delivery | IssueAutomationGate::Merge
        ) && gate.action != IssueAutomationGateAction::Human
            && (record.effective_mode != IssueAutomationAutonomy::Auto
                || gate.action != IssueAutomationGateAction::Auto)
        {
            return Err(invalid());
        }
        if *expected == IssueAutomationGate::Plan
            && !matches!(
                gate.action,
                IssueAutomationGateAction::Human | IssueAutomationGateAction::TimeoutArbiter
            )
        {
            return Err(invalid());
        }
        if record.effective_mode == IssueAutomationAutonomy::Propose
            && *expected == IssueAutomationGate::Start
            && gate.action != IssueAutomationGateAction::Human
        {
            return Err(invalid());
        }
    }
    if !record
        .reasons
        .contains(&IssueTriageReasonCode::CloseDisabled)
        || !record
            .reasons
            .contains(&IssueTriageReasonCode::ExecutionGrantRequired)
    {
        return Err(invalid());
    }
    match record.basis.source {
        IssueTriageSourceState::NotYetObserved | IssueTriageSourceState::Unknown => {
            if record.eligibility != IssueTriageEligibility::Unknown
                || record.readiness != IssueTriageReadiness::Unknown
                || record.risk_class != IssueAutomationRiskClass::High
            {
                return Err(invalid());
            }
        }
        IssueTriageSourceState::Withheld => {
            if record.readiness == IssueTriageReadiness::Ready
                || record.risk_class != IssueAutomationRiskClass::High
            {
                return Err(invalid());
            }
        }
        IssueTriageSourceState::Shared => {}
    }
    validate_observation(record)
}

fn validate_observation(record: &PersistedIssueTriageAssessment) -> Result<(), IssueTriageError> {
    use IssueObservationDisposition as Disposition;
    let eligible_ready = record.eligibility == IssueTriageEligibility::Eligible
        && record.readiness == IssueTriageReadiness::Ready;
    let human_start_only = super::jira_human_start_review(
        &record.basis,
        record.eligibility,
        record.readiness,
        &record.reasons,
    );
    let valid = match record.observation.disposition {
        Disposition::AutomaticStart => {
            eligible_ready
                && record.observation.assumed_mode >= IssueAutomationAutonomy::Gated
                && record.risk_class != IssueAutomationRiskClass::High
                && !record
                    .reasons
                    .contains(&IssueTriageReasonCode::HeldOrManual)
                && !record
                    .reasons
                    .contains(&IssueTriageReasonCode::ProtectedPath)
                && !record
                    .reasons
                    .contains(&IssueTriageReasonCode::BudgetExhausted)
        }
        Disposition::HumanApproval => {
            (eligible_ready || human_start_only)
                && record.observation.assumed_mode >= IssueAutomationAutonomy::Propose
        }
        Disposition::Deferred => {
            eligible_ready
                && record.observation.assumed_mode >= IssueAutomationAutonomy::Gated
                && record.risk_class != IssueAutomationRiskClass::High
                && !record
                    .reasons
                    .contains(&IssueTriageReasonCode::HeldOrManual)
                && !record
                    .reasons
                    .contains(&IssueTriageReasonCode::ProtectedPath)
                && !record
                    .reasons
                    .contains(&IssueTriageReasonCode::BudgetExhausted)
        }
        Disposition::Excluded => record.eligibility == IssueTriageEligibility::Ineligible,
        Disposition::NeedsInfo => {
            (record.eligibility == IssueTriageEligibility::Eligible
                || (record.eligibility == IssueTriageEligibility::Unknown
                    && record
                        .reasons
                        .contains(&IssueTriageReasonCode::JiraPullRequestLookupUnsupported)))
                && record.readiness == IssueTriageReadiness::NeedsInfo
        }
        Disposition::Unknown => {
            record.eligibility == IssueTriageEligibility::Unknown
                || (record.eligibility != IssueTriageEligibility::Ineligible
                    && record.readiness == IssueTriageReadiness::Unknown)
        }
        Disposition::ObservationOnly => {
            (eligible_ready || human_start_only)
                && record.observation.assumed_mode <= IssueAutomationAutonomy::Observe
        }
    };
    if valid {
        Ok(())
    } else {
        Err(IssueTriageError::InvalidAssessment)
    }
}
