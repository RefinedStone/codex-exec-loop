use crate::domain::issue_automation::criteria::{
    IssueWorkCriterion, IssueWorkCriterionChannel, IssueWorkVerificationRecipe,
    normalize_issue_work_criteria,
};
use crate::domain::issue_automation::decision::{IssueDecisionKind, IssueDecisionRequest};
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_policy::IssuePolicy;
use anyhow::{Result, bail};

pub(super) fn confirmed_criteria(
    request: &IssueDecisionRequest,
    option: &str,
    criteria: Option<&[IssueWorkCriterion]>,
    policy: &IssuePolicy,
) -> Result<Option<Vec<IssueWorkCriterion>>> {
    let Some(criteria) = criteria else {
        return Ok(None);
    };
    let definition = request.definition();
    if definition.kind != IssueDecisionKind::StartApproval
        || definition.gate != IssueAutomationGate::Start
        || option != "start"
        || definition.basis.field_hash.is_none()
        || definition.basis.work_order_id.is_some()
        || definition.basis.delivery.is_some()
    {
        bail!("structured criteria can only be confirmed with the original issue start");
    }
    let normalized = normalize_issue_work_criteria(criteria).map_err(anyhow::Error::msg)?;
    if normalized
        .iter()
        .map(|value| value.scenario.len() + value.id.len())
        .sum::<usize>()
        > 16_384
    {
        bail!("the confirmed criteria exceed the native review limit");
    }
    for criterion in &normalized {
        super::reject_secret_identifier(&criterion.id)?;
        let shared = super::shared_text(&criterion.scenario, policy)?;
        if shared.as_str() != criterion.scenario {
            bail!("confirmed criteria must contain only publication-safe text");
        }
    }
    Ok(Some(normalized))
}

pub(super) fn confirmation_detail(criteria: &[IssueWorkCriterion]) -> String {
    let mut detail = String::from("\n\n작업 완료 기준 (시작 시 최초 작업에 고정):");
    for criterion in criteria {
        let channel = match criterion.channel {
            IssueWorkCriterionChannel::Command => "명령",
            IssueWorkCriterionChannel::Data => "데이터",
            IssueWorkCriterionChannel::Http => "HTTP",
            IssueWorkCriterionChannel::Screen => "화면",
        };
        let recipe = match criterion.verification_recipe {
            Some(IssueWorkVerificationRecipe::GitDiffCheckV1) => "Git diff 공백·충돌 표시 검사",
            None => "지정하지 않음",
        };
        detail.push_str(&format!(
            "\n\n[{}] {}\n채널: {} / 필수: {} / 호스트 검증: {}",
            criterion.id,
            criterion.scenario,
            channel,
            if criterion.essential {
                "예"
            } else {
                "아니오"
            },
            recipe,
        ));
    }
    detail
}
