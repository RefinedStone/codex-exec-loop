//! 사용자가 명시 확인한 구조화 완료 기준과 승인 기록 참조다.
//! 외부 본문에서 검증 채널이나 실행 절차를 추측하지 않는다.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use super::decision::{
    IssueDecisionAnswerBy, IssueDecisionBasis, IssueDecisionId, IssueDecisionKind,
    IssueDecisionState, StoredIssueDecision, validate_issue_decision_confirmation_reference,
};
use super::policy::IssueAutomationGate;
use crate::domain::issue_work::IssueReference;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueWorkCriterion {
    pub id: String,
    pub scenario: String,
    pub channel: IssueWorkCriterionChannel,
    pub essential: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_recipe: Option<IssueWorkVerificationRecipe>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueWorkCriterionChannel {
    Command,
    Data,
    Http,
    Screen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueWorkVerificationRecipe {
    GitDiffCheckV1,
    RepoGitGrepV1,
}

pub fn normalize_issue_work_criteria(
    criteria: &[IssueWorkCriterion],
) -> Result<Vec<IssueWorkCriterion>, &'static str> {
    if criteria.is_empty() || criteria.len() > 64 {
        return Err("confirmed issue criteria must contain between one and 64 entries");
    }
    let mut ids = BTreeSet::new();
    let mut normalized = Vec::with_capacity(criteria.len());
    for value in criteria {
        let mut value = value.clone();
        value.id = value.id.trim().to_string();
        value.scenario = value.scenario.trim().to_string();
        if value.id.is_empty()
            || value.id.len() > 128
            || value.id.chars().any(char::is_control)
            || value.scenario.is_empty()
            || value.scenario.len() > 4096
            || value
                .scenario
                .chars()
                .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
            || !ids.insert(value.id.clone())
        {
            return Err("invalid or duplicate confirmed issue criterion");
        }
        if value.verification_recipe.is_some()
            && value.channel != IssueWorkCriterionChannel::Command
        {
            return Err("host verification recipes require a command criterion");
        }
        normalized.push(value);
    }
    normalized.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(normalized)
}

pub fn issue_work_criteria_digest(criteria: &[IssueWorkCriterion]) -> String {
    let bytes = serde_json::to_vec(criteria).expect("typed issue criteria serialize");
    format!("{:x}", Sha256::digest(bytes))
}

/// 저장된 승인 참조다. 이 직렬화 값만으로 사용자 확인 권한을 부여하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueWorkCriteriaAdoption {
    pub decision_id: IssueDecisionId,
    pub decision_revision: u64,
    pub basis: IssueDecisionBasis,
    pub confirmation_ref: String,
    pub criteria: Vec<IssueWorkCriterion>,
}

impl IssueWorkCriteriaAdoption {
    pub fn from_stored(stored: &StoredIssueDecision) -> Result<Option<Self>, &'static str> {
        let request = &stored.request;
        let Some(answer) = request.answer() else {
            return Ok(None);
        };
        let Some(criteria) = &answer.criteria else {
            return Ok(None);
        };
        if request.state() != IssueDecisionState::Answered
            || request.definition().kind != IssueDecisionKind::StartApproval
            || request.definition().gate != IssueAutomationGate::Start
            || answer.option != "start"
            || answer.by != IssueDecisionAnswerBy::Human
        {
            return Err("structured issue criteria require a confirmed start decision");
        }
        let receipt = Self {
            decision_id: request.id().clone(),
            decision_revision: stored.revision,
            basis: request.definition().basis.clone(),
            confirmation_ref: answer
                .confirmation
                .clone()
                .ok_or("missing criteria confirmation")?,
            criteria: criteria.clone(),
        };
        receipt.validate()?;
        Ok(Some(receipt))
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        self.basis
            .validate()
            .map_err(|_| "invalid criteria approval basis")?;
        validate_issue_decision_confirmation_reference(&self.confirmation_ref)
            .map_err(|_| "invalid criteria approval reference")?;
        if self.decision_revision == 0
            || self.decision_revision > super::MAX_ISSUE_AUTOMATION_REVISION
            || self.basis.field_hash.is_none()
            || self.basis.work_order_id.is_some()
            || self.basis.delivery.is_some()
            || normalize_issue_work_criteria(&self.criteria)? != self.criteria
        {
            return Err("invalid criteria approval receipt");
        }
        Ok(())
    }

    /// 대상 이슈, 작업, workspace와 전체 불변 승인 내용을 함께 묶는다.
    pub fn receipt_digest(&self, issue: &IssueReference, work_order_id: &str) -> String {
        let bytes = serde_json::to_vec(&(1_u32, issue, work_order_id, self))
            .expect("typed issue criteria receipt serializes");
        format!("{:x}", Sha256::digest(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn criterion() -> IssueWorkCriterion {
        IssueWorkCriterion {
            id: " one ".into(),
            scenario: " shared outcome ".into(),
            channel: IssueWorkCriterionChannel::Command,
            essential: true,
            verification_recipe: Some(IssueWorkVerificationRecipe::GitDiffCheckV1),
        }
    }

    #[test]
    fn explicit_criteria_are_canonical_and_digest_covers_each_user_choice() {
        let normalized = normalize_issue_work_criteria(&[criterion()]).unwrap();
        assert_eq!(normalized[0].id, "one");
        assert_eq!(normalized[0].scenario, "shared outcome");
        let original = issue_work_criteria_digest(&normalized);
        for field in ["id", "scenario", "channel", "essential", "recipe"] {
            let mut changed = normalized.clone();
            match field {
                "id" => changed[0].id = "two".into(),
                "scenario" => changed[0].scenario = "another outcome".into(),
                "channel" => changed[0].channel = IssueWorkCriterionChannel::Data,
                "essential" => changed[0].essential = false,
                "recipe" => changed[0].verification_recipe = None,
                _ => unreachable!(),
            }
            assert_ne!(issue_work_criteria_digest(&changed), original, "{field}");
        }
    }

    #[test]
    fn criteria_reject_empty_duplicate_unknown_channels_and_non_command_recipes() {
        assert!(normalize_issue_work_criteria(&[]).is_err());
        assert!(normalize_issue_work_criteria(&[criterion(), criterion()]).is_err());
        let mut bad = criterion();
        bad.channel = IssueWorkCriterionChannel::Screen;
        assert!(normalize_issue_work_criteria(&[bad]).is_err());
        assert!(
            serde_json::from_str::<IssueWorkCriterion>(
                r#"{"id":"one","scenario":"check","channel":"shell","essential":true}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<IssueWorkCriterion>(r#"{"id":"one","scenario":"check","channel":"command","essential":true,"verified":true}"#).is_err());
    }
}
