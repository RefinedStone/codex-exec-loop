//! Human answers and a confirmed Gated selection remain distinct start authorities.
use crate::domain::issue_automation::coordinator::{
    IssueWorkStartAuthority, human_start, policy_start, ready_for_human_start,
    timeout_arbiter_start, timeout_default_start,
};
use crate::domain::issue_automation::decision::{
    IssueDecisionBasis, IssueDecisionId, IssueDecisionRequest,
};
use crate::domain::issue_automation::triage::IssueTriageAssessment;

#[derive(Clone, Copy)]
pub(super) enum StartAuthorization<'a> {
    Human(&'a IssueDecisionRequest),
    TimeoutDefault(&'a IssueDecisionRequest),
    TimeoutArbiter(&'a IssueDecisionRequest),
    Policy(&'a IssueDecisionBasis),
}

pub(super) struct StartContext<'a> {
    pub authorization: StartAuthorization<'a>,
    pub now_unix_ms: u64,
    pub execution_enabled: bool,
}

impl<'a> StartAuthorization<'a> {
    pub fn basis(self) -> &'a IssueDecisionBasis {
        match self {
            Self::Human(decision)
            | Self::TimeoutDefault(decision)
            | Self::TimeoutArbiter(decision) => &decision.definition().basis,
            Self::Policy(basis) => basis,
        }
    }

    pub fn authority(self) -> IssueWorkStartAuthority {
        match self {
            Self::Human(_) => IssueWorkStartAuthority::Human,
            Self::TimeoutDefault(_) => IssueWorkStartAuthority::TimeoutDefault,
            Self::TimeoutArbiter(_) => IssueWorkStartAuthority::TimeoutArbiter,
            Self::Policy(_) => IssueWorkStartAuthority::Policy,
        }
    }

    pub fn decision_id(self) -> Option<&'a IssueDecisionId> {
        match self {
            Self::Human(decision)
            | Self::TimeoutDefault(decision)
            | Self::TimeoutArbiter(decision) => Some(decision.id()),
            Self::Policy(_) => None,
        }
    }

    pub fn permits(
        self,
        assessment: &IssueTriageAssessment,
        current_basis: &IssueDecisionBasis,
    ) -> bool {
        self.basis() == current_basis
            && match self {
                Self::Human(decision) => {
                    ready_for_human_start(assessment)
                        && human_start(decision)
                        && decision.issue() == &assessment.basis().issue
                }
                Self::Policy(_) => policy_start(assessment),
                Self::TimeoutDefault(decision) => timeout_default_start(assessment, decision),
                Self::TimeoutArbiter(decision) => timeout_arbiter_start(assessment, decision),
            }
    }
}
