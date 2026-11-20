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
        let enabled = enabled && gate != IssueAutomationGate::Close;
        result.push(IssueTriageGate {
            gate,
            action: if enabled {
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
    evaluation: &Evaluation,
    policy: &IssueAutomationPolicy,
) -> IssueTriageObservation {
    use IssueObservationDisposition as Disposition;
    let disposition = match (evaluation.eligibility, evaluation.readiness) {
        (IssueTriageEligibility::Ineligible, _) => Disposition::Excluded,
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
    // E3 실행 허가가 없는 현재 조각은 delivery/merge를 자동으로 통과시키지 않는다.
    // 실제 close도 후속 조각이며 언제나 disabled이다.
    if evaluation.force_human
        || evaluation.human_gates.contains(&gate)
        || matches!(
            gate,
            IssueAutomationGate::Spec
                | IssueAutomationGate::Delivery
                | IssueAutomationGate::Merge
                | IssueAutomationGate::Close
        )
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
        IssueAutomationGate::Spec
        | IssueAutomationGate::Delivery
        | IssueAutomationGate::Merge
        | IssueAutomationGate::Close => Human,
    }
}

pub(super) fn validate_record(
    record: &PersistedIssueTriageAssessment,
) -> Result<(), IssueTriageError> {
    let invalid = || IssueTriageError::InvalidAssessment;
    if record.gates.len() != GATES.len()
        || !record.observation.observation_only
        || record.observation.assumed_mode < record.effective_mode
    {
        return Err(invalid());
    }
    let enabled = record.effective_mode >= IssueAutomationAutonomy::Propose
        && record.eligibility == IssueTriageEligibility::Eligible
        && record.readiness == IssueTriageReadiness::Ready;
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
            || gate.enabled != (enabled && *expected != IssueAutomationGate::Close)
            || ((!gate.enabled || all_human) && gate.action != IssueAutomationGateAction::Human)
        {
            return Err(invalid());
        }
        if matches!(
            expected,
            IssueAutomationGate::Spec
                | IssueAutomationGate::Delivery
                | IssueAutomationGate::Merge
                | IssueAutomationGate::Close
        ) && gate.action != IssueAutomationGateAction::Human
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
            eligible_ready && record.observation.assumed_mode >= IssueAutomationAutonomy::Propose
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
            record.eligibility == IssueTriageEligibility::Eligible
                && record.readiness == IssueTriageReadiness::NeedsInfo
        }
        Disposition::Unknown => {
            record.eligibility == IssueTriageEligibility::Unknown
                || (record.eligibility != IssueTriageEligibility::Ineligible
                    && record.readiness == IssueTriageReadiness::Unknown)
        }
        Disposition::ObservationOnly => {
            eligible_ready && record.observation.assumed_mode <= IssueAutomationAutonomy::Observe
        }
    };
    if valid {
        Ok(())
    } else {
        Err(IssueTriageError::InvalidAssessment)
    }
}
