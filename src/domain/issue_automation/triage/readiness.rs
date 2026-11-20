//! 초안과 실제 판정이 함께 사용하는 준비·위험·예상 관문 계산이다.
//! 안정 이슈 ID, 원격 작성자, 실행 승인이나 장부 판을 만들지 않는다.
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::policy::{
    IssueAutomationGate, IssueAutomationGateAction, IssueAutomationPolicy, IssueAutomationRiskClass,
};
use crate::domain::issue_validation::{
    IssueRequirements, IssueValidationError, IssueValidationErrorKind, IssueValidationField,
    validate_issue,
};
use crate::domain::issue_work::IssueSharedCopy;

use super::criteria::{CriteriaProblem, count_criteria};
use super::paths::{MentionedPaths, matches_glob, mentioned_paths};
use super::{
    IssueTriageError, IssueTriageReadiness, IssueTriageReasonCode, MAX_ISSUE_TRIAGE_BODY_BYTES,
    MAX_ISSUE_TRIAGE_TITLE_BYTES,
};

pub struct IssueReadinessInput<'a> {
    /// 현재 경계에서 공개가 허용된 공유본이다. 원문이나 과거 frozen 공유본으로 대체하지 않는다.
    pub shared_copy: &'a IssueSharedCopy,
    pub requirements: &'a IssueRequirements,
    pub policy: &'a IssueAutomationPolicy,
    /// 초안에서는 사용자가 고른 가정이며 실제 라벨 적용의 근거가 아니다.
    pub labels: &'a [String],
    pub labels_complete: bool,
    pub held: bool,
}

/// 선택·신뢰가 확인된다는 가정 아래의 정책 관문이다. 활성화나 실행 허가가 아니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueExpectedGate {
    pub gate: IssueAutomationGate,
    pub action: IssueAutomationGateAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueReadinessResult {
    pub readiness: IssueTriageReadiness,
    pub missing_sections: Vec<String>,
    pub validation_errors: Vec<IssueValidationError>,
    pub risk_class: IssueAutomationRiskClass,
    pub effective_mode: IssueAutomationAutonomy,
    pub assumed_mode: IssueAutomationAutonomy,
    pub reasons: Vec<IssueTriageReasonCode>,
    pub expected_gates: Vec<IssueExpectedGate>,
    pub(super) force_human: bool,
    pub(super) human_gates: Vec<IssueAutomationGate>,
}

pub fn inspect_issue_readiness(
    input: IssueReadinessInput<'_>,
) -> Result<IssueReadinessResult, IssueTriageError> {
    input
        .policy
        .validate()
        .map_err(|_| IssueTriageError::InvalidPolicy)?;
    let mut result = inspect_content(&input);
    result
        .reasons
        .push(IssueTriageReasonCode::ExecutionGrantRequired);
    result.reasons.push(IssueTriageReasonCode::CloseDisabled);
    match result.effective_mode {
        IssueAutomationAutonomy::Off => result.reasons.push(IssueTriageReasonCode::ModeOff),
        IssueAutomationAutonomy::Observe => {
            result.reasons.push(IssueTriageReasonCode::ObservationOnly)
        }
        _ => {}
    }
    result.reasons.sort();
    result.reasons.dedup();
    result.missing_sections.sort();
    result.missing_sections.dedup();
    result.expected_gates = super::gates::expected(&result, input.policy);
    Ok(result)
}

fn inspect_content(input: &IssueReadinessInput<'_>) -> IssueReadinessResult {
    let exhausted = budget_exhausted(input.policy);
    let cap = label_cap(input.labels, input.policy);
    let held = held_or_manual(input.labels, input.policy, input.held);
    let mut result = IssueReadinessResult {
        readiness: IssueTriageReadiness::Unknown,
        missing_sections: Vec::new(),
        validation_errors: Vec::new(),
        risk_class: IssueAutomationRiskClass::High,
        effective_mode: input.policy.binding.github.mode.min(cap),
        assumed_mode: input.policy.authority.autonomy_cap.min(cap),
        reasons: Vec::new(),
        expected_gates: Vec::new(),
        force_human: true,
        human_gates: Vec::new(),
    };
    if exhausted {
        result.reasons.push(IssueTriageReasonCode::BudgetExhausted);
    }
    if held {
        result.reasons.push(IssueTriageReasonCode::HeldOrManual);
    }
    let copy = input.shared_copy;
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
        for error in &errors {
            result.reasons.push(match &error.field {
                IssueValidationField::Title => IssueTriageReasonCode::TitleMissing,
                _ => IssueTriageReasonCode::RequiredSectionMissing,
            });
            if matches!(
                error.kind,
                IssueValidationErrorKind::Missing | IssueValidationErrorKind::Empty
            ) {
                match &error.field {
                    IssueValidationField::TemplateField(label)
                    | IssueValidationField::RequiredSection(label) => {
                        result.missing_sections.push(label.clone());
                    }
                    IssueValidationField::Title => {}
                }
            }
        }
        result.validation_errors = errors;
    }
    if let Err(problem) = count_criteria(&copy.body) {
        result.readiness = IssueTriageReadiness::NeedsInfo;
        result.reasons.push(match problem {
            CriteriaProblem::Missing => {
                result.missing_sections.push("완료 기준".to_owned());
                IssueTriageReasonCode::CriteriaMissing
            }
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
    result.risk_class = risk(input.labels, input.labels_complete, &paths, input.policy);
    if masked || unsupported || !input.labels_complete || paths.incomplete {
        result.risk_class = IssueAutomationRiskClass::High;
    }
    if !input.labels_complete {
        result
            .reasons
            .push(IssueTriageReasonCode::MetadataIncomplete);
    }
    if paths.incomplete {
        result
            .reasons
            .push(IssueTriageReasonCode::UnsupportedContent);
    }
    result.force_human = exhausted || held || result.risk_class == IssueAutomationRiskClass::High;
    match path_match(&input.policy.protection.paths, &paths) {
        Some(true) => {
            result.force_human = true;
            result.reasons.push(IssueTriageReasonCode::ProtectedPath);
        }
        None => {
            result.force_human = true;
            result.risk_class = IssueAutomationRiskClass::High;
            result
                .reasons
                .push(IssueTriageReasonCode::UnsupportedContent);
        }
        Some(false) => {}
    }
    for rule in &input.policy.human_required.additional {
        match condition_match(
            &rule.labels_any,
            &rule.paths_any,
            input.labels,
            input.labels_complete,
            &paths,
        ) {
            Some(false) => {}
            Some(true) => {
                result.human_gates.extend(&rule.gates);
                result
                    .reasons
                    .push(IssueTriageReasonCode::PolicyHumanRequired);
            }
            None => {
                result.force_human = true;
                result.risk_class = IssueAutomationRiskClass::High;
                result
                    .reasons
                    .push(IssueTriageReasonCode::PolicyHumanRequired);
            }
        }
    }
    if result.risk_class == IssueAutomationRiskClass::High {
        result.reasons.push(IssueTriageReasonCode::HighRisk);
    }
    result
}

pub(super) fn budget_exhausted(policy: &IssueAutomationPolicy) -> bool {
    policy.budget.max_running == 0
        || policy.budget.max_started_per_day == 0
        || policy.budget.max_attempts == 0
}

pub(super) fn label_cap(
    labels: &[String],
    policy: &IssueAutomationPolicy,
) -> IssueAutomationAutonomy {
    if labels.contains(&policy.labels.manual) {
        IssueAutomationAutonomy::Propose
    } else if labels.contains(&policy.labels.gated) {
        IssueAutomationAutonomy::Gated
    } else {
        IssueAutomationAutonomy::Auto
    }
}

pub(super) fn held_or_manual(
    labels: &[String],
    policy: &IssueAutomationPolicy,
    held: bool,
) -> bool {
    held || labels.contains(&policy.labels.hold) || labels.contains(&policy.labels.manual)
}

fn risk(
    labels: &[String],
    complete: bool,
    paths: &MentionedPaths,
    policy: &IssueAutomationPolicy,
) -> IssueAutomationRiskClass {
    if paths.incomplete || !complete {
        return IssueAutomationRiskClass::High;
    }
    for rule in &policy.classify.rule {
        match condition_match(&rule.labels_any, &rule.paths_any, labels, complete, paths) {
            Some(true) => return rule.class,
            Some(false) => {}
            None => return IssueAutomationRiskClass::High,
        }
    }
    IssueAutomationRiskClass::Normal
}

fn condition_match(
    conditions: &[String],
    patterns: &[String],
    labels: &[String],
    complete: bool,
    paths: &MentionedPaths,
) -> Option<bool> {
    if conditions.iter().any(|label| labels.contains(label)) {
        return Some(true);
    }
    match path_match(patterns, paths) {
        Some(true) => Some(true),
        None => None,
        Some(false) if !conditions.is_empty() && !complete => None,
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
