use std::collections::BTreeSet;

use super::*;
use crate::domain::planning::evidence::{
    MAX_TASK_EVIDENCE_PAGE, MAX_TASK_EVIDENCE_REVISION, validate_evidence_identifier,
    validate_evidence_request_id, validate_evidence_token,
};

fn invalid<T>(value: Result<T, impl std::fmt::Display>) -> Result<T, TaskDeliveryEvidenceError> {
    value.map_err(|_| TaskDeliveryEvidenceError::Invalid)
}

fn oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl TaskEvidenceDeliveryBinding {
    pub fn validate(&self) -> Result<(), TaskDeliveryEvidenceError> {
        invalid(self.scope.validate())?;
        invalid(validate_evidence_token(&self.lease_generation))
    }
}

impl TaskEvidenceDeliverySource {
    pub fn validate(&self) -> Result<(), TaskDeliveryEvidenceError> {
        let length = self.base_commit_sha.len();
        let mut values = vec![
            self.base_commit_sha.as_str(),
            self.base_tree_sha.as_str(),
            self.source_commit_sha.as_str(),
            self.source_tree_sha.as_str(),
        ];
        if let Some(integration) = &self.integration {
            values.extend([
                integration.base_commit_sha.as_str(),
                integration.result_commit_sha.as_str(),
                integration.result_tree_sha.as_str(),
            ]);
        }
        if values
            .iter()
            .any(|value| value.len() != length || !oid(value))
        {
            return Err(TaskDeliveryEvidenceError::Invalid);
        }
        Ok(())
    }
}

impl TaskDeliveryEvidenceObservation {
    pub fn validate(&self) -> Result<(), TaskDeliveryEvidenceError> {
        if self.evidence_revision > MAX_TASK_EVIDENCE_REVISION
            || self.criteria.len() > MAX_TASK_EVIDENCE_PAGE
            || self.reasons.len() > 8
            || chrono::DateTime::parse_from_rfc3339(&self.observed_at).is_err()
        {
            return Err(TaskDeliveryEvidenceError::Invalid);
        }
        if let Some(scope) = &self.scope {
            invalid(scope.validate())?;
        }
        if let Some(source) = &self.source {
            source.validate()?;
            if self.stage != TaskEvidenceDeliveryStage::Integrate && source.integration.is_some() {
                return Err(TaskDeliveryEvidenceError::Invalid);
            }
        }
        let mut identifiers = BTreeSet::new();
        for row in &self.criteria {
            invalid(validate_evidence_identifier(&row.criterion_id))?;
            if !identifiers.insert(&row.criterion_id) {
                return Err(TaskDeliveryEvidenceError::Invalid);
            }
            if let Some(event_id) = &row.receipt_event_id {
                invalid(validate_evidence_request_id(event_id))?;
            }
            if (row.state == TaskDeliveryCriterionState::Blocked) != row.reason.is_some()
                || (row.state != TaskDeliveryCriterionState::Blocked
                    && row.receipt_event_id.is_none())
                || row
                    .reason
                    .is_some_and(|reason| !self.reasons.contains(&reason))
            {
                return Err(TaskDeliveryEvidenceError::Invalid);
            }
        }
        for (index, reason) in self.reasons.iter().enumerate() {
            if self.reasons[..index].contains(reason) {
                return Err(TaskDeliveryEvidenceError::Invalid);
            }
        }
        if self.criteria.is_empty() && self.reasons.is_empty() {
            return Err(TaskDeliveryEvidenceError::Invalid);
        }
        if self.satisfied() {
            if self.scope.is_none() || self.source.is_none() {
                return Err(TaskDeliveryEvidenceError::Invalid);
            }
            if self.stage == TaskEvidenceDeliveryStage::Integrate {
                let source = self.source.as_ref().expect("checked source");
                if source.integration.as_ref().is_none_or(|integration| {
                    integration.base_commit_sha != source.base_commit_sha
                        || integration.result_tree_sha != source.source_tree_sha
                }) {
                    return Err(TaskDeliveryEvidenceError::Invalid);
                }
            }
        }
        Ok(())
    }
}
