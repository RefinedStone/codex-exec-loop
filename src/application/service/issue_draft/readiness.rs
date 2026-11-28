//! 게시용 미리보기와 별도로 현재 초안의 AI 전달 공유본만 검사한다.
//! 원문, 복원 평문, 안정 이슈 ID, 장부 또는 실행 승인을 만들지 않는다.
use super::*;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::policy::{
    IssueAutomationCloseWhen, IssueAutomationGate, IssueAutomationGateAction,
    IssueAutomationRiskClass, validate_text_value,
};
use crate::domain::issue_automation::triage::{
    IssueReadinessInput, IssueTriageReadiness, IssueTriageReasonCode, inspect_issue_readiness,
};

pub(super) fn inspect(
    policy_service: Option<&IssueAutomationPolicyService>,
    (workspace, provider): (&str, crate::domain::issue_work::IssueProvider),
    conventions: &Conventions,
    text: &IssueDraftText,
    expected_labels: &[String],
) -> IssueDraftReadiness {
    let Some(policy_service) = policy_service else {
        return blocked(IssueDraftReadinessBlocker::AutomationPolicyUnavailable);
    };
    let resolved = match policy_service.resolve(workspace) {
        Ok(policy) => policy,
        Err(error) => {
            let mut result = blocked(IssueDraftReadinessBlocker::PolicyBlocked);
            if let IssueDraftReadiness::CannotAssess { policy_error, .. } = &mut result {
                *policy_error = Some(IssueDraftReadinessPolicyError {
                    scope: error.scope.label(),
                    field: error.field,
                    reason: error.reason,
                });
            }
            return result;
        }
    };
    let Some(requirements) = requirements(conventions) else {
        return blocked(IssueDraftReadinessBlocker::RequirementsUnavailable);
    };
    if expected_labels.len() > 100
        || expected_labels
            .iter()
            .any(|label| validate_text_value(label, "expected_labels").is_err())
    {
        return blocked(IssueDraftReadinessBlocker::InvalidLabels);
    }
    let mut labels = expected_labels.to_vec();
    labels.sort();
    labels.dedup();

    let shared_copy = match super::ai_copy::masked(
        conventions,
        text,
        super::ai_copy::AiCopyPurpose::ReadinessPreview,
    ) {
        Ok(shared_copy) => shared_copy,
        Err(reason) => return blocked(reason),
    };
    let result = match inspect_issue_readiness(IssueReadinessInput {
        provider,
        shared_copy: &shared_copy,
        requirements: &requirements,
        policy: &resolved.policy,
        labels: &labels,
        labels_complete: true,
        held: false,
    }) {
        Ok(result) => result,
        Err(_) => return blocked(IssueDraftReadinessBlocker::InvalidInput),
    };
    IssueDraftReadiness::Assessed {
        assessment: Box::new(IssueDraftReadinessAssessment {
            preview_only: true,
            readiness: result.readiness,
            summary: match result.readiness {
                IssueTriageReadiness::Ready => "준비 조건을 충족합니다.",
                IssueTriageReadiness::NeedsInfo => "실행 계약을 보완해야 합니다.",
                IssueTriageReadiness::Unknown => "현재 내용만으로 준비 상태를 확인할 수 없습니다.",
            },
            missing_sections: result.missing_sections,
            errors: result
                .validation_errors
                .iter()
                .map(validation_error)
                .collect(),
            risk_class: result.risk_class,
            risk_summary: match result.risk_class {
                IssueAutomationRiskClass::Low => "낮음",
                IssueAutomationRiskClass::Normal => "보통",
                IssueAutomationRiskClass::High => "높음",
            },
            effective_mode: result.effective_mode,
            assumed_mode: result.assumed_mode,
            mode_summary: format!(
                "관문 기준: {} · 관찰 보고의 가정 수준: {}",
                mode_summary(result.effective_mode),
                mode_summary(result.assumed_mode),
            ),
            expected_gates: result
                .expected_gates
                .into_iter()
                .map(|gate| IssueDraftExpectedGate {
                    gate: gate.gate,
                    action: gate.action,
                    summary: gate_summary(gate.gate),
                    action_summary: if gate.gate == IssueAutomationGate::Close
                        && resolved.policy.close.when == IssueAutomationCloseWhen::Never
                    {
                        "닫기 꺼짐"
                    } else {
                        action_summary(gate.action)
                    },
                })
                .collect(),
            reasons: result
                .reasons
                .into_iter()
                .map(|code| IssueDraftReadinessReason {
                    code,
                    summary: reason_summary(code),
                })
                .collect(),
            policy_version: resolved.version,
            disclosure_policy_version: conventions.policy_version.clone(),
            shared_copy_hash: IssueFieldHash::of_fields(&shared_copy.title, &shared_copy.body),
        }),
    }
}

pub(super) fn attach(preview: &mut IssueDraftPreview, value: IssueDraftReadiness) {
    let target = match preview {
        IssueDraftPreview::Ready { readiness, .. }
        | IssueDraftPreview::NeedsConfirmation { readiness, .. }
        | IssueDraftPreview::InvalidSpan { readiness, .. } => readiness,
    };
    *target = Some(Box::new(value));
}

pub(super) fn blocked(reason: IssueDraftReadinessBlocker) -> IssueDraftReadiness {
    use IssueDraftReadinessBlocker::*;
    let (summary, next_summary) = match reason {
        AutomationPolicyUnavailable => (
            "준비검사 서비스를 사용할 수 없습니다.",
            "Gateway 연결을 확인한 뒤 미리보기를 다시 만드세요.",
        ),
        PolicyBlocked => (
            "자동화 정책을 읽거나 해석하지 못해 준비 상태를 검사할 수 없습니다.",
            "이슈 탭의 정책 보기에서 원인을 확인하세요. 게시 승인은 별도로 유지됩니다.",
        ),
        RequirementsUnavailable => (
            "필수 섹션이나 템플릿의 요구 사항을 확인하지 못했습니다.",
            "프로젝트의 이슈 정책과 템플릿을 확인하세요.",
        ),
        DisclosureConfirmationRequired => (
            "AI 전달 공개 정책에서 사용자 확인이 필요한 구간이 있습니다.",
            "해당 내용을 제거하거나 AI 전달 공개 정책을 확인한 뒤 다시 검사하세요.",
        ),
        InvalidSpan => (
            "민감 구간의 범위가 현재 초안과 맞지 않습니다.",
            "민감 구간을 다시 지정하세요.",
        ),
        InvalidLabels => (
            "예상 라벨의 형식이나 개수가 지원 범위를 벗어났습니다.",
            "비어 있지 않은 라벨 이름을 100개 이내로 입력하세요.",
        ),
        FrozenDraft => (
            "기록된 게시본에는 현재 초안의 준비검사를 적용하지 않습니다.",
            "새 초안에서 준비 조건을 확인하세요. 기록된 생성 결과는 계속 확인할 수 있습니다.",
        ),
        InvalidInput => (
            "현재 입력으로 준비 상태를 검사할 수 없습니다.",
            "초안과 정책의 값을 확인한 뒤 다시 검사하세요.",
        ),
    };
    IssueDraftReadiness::CannotAssess {
        reason,
        summary,
        next_summary,
        policy_error: None,
    }
}

fn mode_summary(mode: IssueAutomationAutonomy) -> &'static str {
    match mode {
        IssueAutomationAutonomy::Off => "꺼짐",
        IssueAutomationAutonomy::Observe => "관찰",
        IssueAutomationAutonomy::Propose => "제안",
        IssueAutomationAutonomy::Gated => "반자동",
        IssueAutomationAutonomy::Auto => "자동",
    }
}

fn gate_summary(gate: IssueAutomationGate) -> &'static str {
    match gate {
        IssueAutomationGate::Start => "시작",
        IssueAutomationGate::Spec => "완료 기준",
        IssueAutomationGate::Plan => "계획",
        IssueAutomationGate::Delivery => "전달",
        IssueAutomationGate::Merge => "통합",
        IssueAutomationGate::Close => "이슈 닫기",
    }
}

fn action_summary(action: IssueAutomationGateAction) -> &'static str {
    match action {
        IssueAutomationGateAction::Human => "사람 확인",
        IssueAutomationGateAction::TimeoutDefault => "마감 뒤 권고값",
        IssueAutomationGateAction::TimeoutArbiter => "마감 뒤 중재자",
        IssueAutomationGateAction::Auto => "정책상 자동",
    }
}

fn reason_summary(reason: IssueTriageReasonCode) -> &'static str {
    // Presentation only: the shared evaluator owns every decision and gate.
    match reason.as_str() {
        "title_missing" => "공유본의 제목이 비어 있습니다.",
        "required_section_missing" => "필수 섹션이 없거나 비어 있습니다.",
        "criteria_missing" => "완료 기준이 없습니다.",
        "criteria_not_list" => "완료 기준을 글머리표, 번호 또는 체크 목록으로 작성하세요.",
        "issue_too_large" => "초안이 준비검사 크기 한도를 넘었습니다.",
        "unsupported_content" => "현재 지원하지 않는 내용이 있습니다.",
        "masked_content" => "가린 부분 때문에 준비 상태를 충분히 확인할 수 없습니다.",
        "metadata_incomplete" => "판정에 필요한 조건을 모두 확인하지 못했습니다.",
        "high_risk" => "위험 등급이 높아 사람 확인이 필요합니다.",
        "held_or_manual" => "예상 라벨이 보류 또는 수동 처리를 지정합니다.",
        "protected_path" => "언급한 범위에 보호 경로가 포함되어 있습니다.",
        "policy_human_required" => "정책의 추가 규칙이 사람 확인을 요구합니다.",
        "budget_exhausted" => "예산 상한이 0이어서 자동 시작할 수 없습니다.",
        "mode_off" => "현재 유효 자동화 수준이 꺼져 있습니다.",
        "observation_only" => "현재 정책은 관찰만 수행합니다.",
        "execution_grant_required" => "준비검사는 실행을 승인하지 않습니다.",
        "close_disabled" => "이 검사로 이슈를 닫지 않습니다.",
        _ => "게시 후 현재 이슈의 조건을 다시 확인해야 합니다.",
    }
}
