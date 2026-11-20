use std::collections::BTreeSet;

use super::{
    IssueDecisionAnswer, IssueDecisionAnswerBy, IssueDecisionBasis, IssueDecisionDefinition,
    IssueDecisionError, IssueDecisionKind, IssueDecisionState, MAX_ISSUE_DECISION_EVIDENCE,
    MAX_ISSUE_DECISION_NOTE_BYTES, MAX_ISSUE_DECISION_OPTION_BYTES, MAX_ISSUE_DECISION_OPTIONS,
    MAX_ISSUE_DECISION_TEXT_BYTES, PersistedIssueDecisionRequest,
};
use crate::domain::issue_automation::MAX_ISSUE_AUTOMATION_REVISION;
use crate::domain::issue_automation::policy::IssueAutomationGate;

pub(super) fn identifier(
    value: &str,
    field: &'static str,
    max_bytes: usize,
) -> Result<(), IssueDecisionError> {
    if value.is_empty()
        || value.len() > max_bytes
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(IssueDecisionError::InvalidField { field });
    }
    Ok(())
}

pub(super) fn shared_text(
    value: &str,
    field: &'static str,
    max_bytes: usize,
) -> Result<(), IssueDecisionError> {
    if value.trim().is_empty()
        || value.len() > max_bytes
        || value.chars().any(|character| {
            character.is_control() && character != '\n' && character != '\r' && character != '\t'
        })
    {
        return Err(IssueDecisionError::InvalidField { field });
    }
    Ok(())
}

pub(super) fn basis(basis: &IssueDecisionBasis) -> Result<(), IssueDecisionError> {
    if !sha256(&basis.policy_version)
        || basis.gate_revision == 0
        || basis.gate_revision > MAX_ISSUE_AUTOMATION_REVISION
        || basis
            .field_hash
            .as_ref()
            .is_some_and(|hash| hash.version == 0 || !sha256(&hash.sha256))
    {
        return Err(IssueDecisionError::InvalidBasis);
    }
    if let Some(id) = &basis.work_order_id {
        identifier(id, "basis.work_order_id", 256)?;
    }
    Ok(())
}

pub(super) fn definition(value: &IssueDecisionDefinition) -> Result<(), IssueDecisionError> {
    super::super::validate_reference(value.subject.issue())
        .map_err(|_| IssueDecisionError::InvalidField { field: "subject" })?;
    basis(&value.basis)?;
    shared_text(
        value.question.as_str(),
        "question",
        MAX_ISSUE_DECISION_TEXT_BYTES,
    )?;
    identifier(&value.asked_by, "asked_by", 256)?;
    if let Some(due_at) = &value.due_at {
        timestamp(due_at, "due_at")?;
    }
    if value.evidence.len() > MAX_ISSUE_DECISION_EVIDENCE {
        return Err(IssueDecisionError::InvalidField { field: "evidence" });
    }
    let mut evidence = BTreeSet::new();
    for reference in &value.evidence {
        identifier(reference, "evidence", 1024)?;
        if !evidence.insert(reference) {
            return Err(IssueDecisionError::InvalidField { field: "evidence" });
        }
    }
    match value.kind {
        IssueDecisionKind::StartApproval if value.gate != IssueAutomationGate::Start => {
            return Err(IssueDecisionError::InvalidField { field: "gate" });
        }
        IssueDecisionKind::SpecConfirmation if value.gate != IssueAutomationGate::Spec => {
            return Err(IssueDecisionError::InvalidField { field: "gate" });
        }
        IssueDecisionKind::PlanApproval if value.gate != IssueAutomationGate::Plan => {
            return Err(IssueDecisionError::InvalidField { field: "gate" });
        }
        _ => {}
    }
    options(value)
}

fn options(value: &IssueDecisionDefinition) -> Result<(), IssueDecisionError> {
    if value.options.is_empty() || value.options.len() > MAX_ISSUE_DECISION_OPTIONS {
        return Err(IssueDecisionError::InvalidField { field: "options" });
    }
    let mut ids = BTreeSet::new();
    for option in &value.options {
        identifier(&option.id, "option.id", MAX_ISSUE_DECISION_OPTION_BYTES)?;
        shared_text(option.label.as_str(), "option.label", 256)?;
        shared_text(option.effect.as_str(), "option.effect", 2048)?;
        if !ids.insert(option.id.as_str()) {
            return Err(IssueDecisionError::InvalidField { field: "option.id" });
        }
    }
    if value
        .recommended
        .as_ref()
        .is_some_and(|id| !ids.contains(id.as_str()))
    {
        return Err(IssueDecisionError::InvalidOption);
    }
    Ok(())
}

pub(super) fn record_state(
    record: &PersistedIssueDecisionRequest,
) -> Result<(), IssueDecisionError> {
    match (record.state, &record.answer) {
        (IssueDecisionState::Answered, Some(value)) => answer(value, &record.definition),
        (IssueDecisionState::Answered, None) => Err(IssueDecisionError::InvalidState),
        (_, Some(_)) => Err(IssueDecisionError::InvalidState),
        (_, None) => Ok(()),
    }
}

pub(super) fn answer(
    value: &IssueDecisionAnswer,
    definition: &IssueDecisionDefinition,
) -> Result<(), IssueDecisionError> {
    if !definition
        .options
        .iter()
        .any(|option| option.id == value.option)
    {
        return Err(IssueDecisionError::InvalidOption);
    }
    // 저장된 참조는 검증 완료의 감사 metadata다. 전송 증거를 복원하거나 인증하지 않는다.
    match (value.by, value.confirmation.as_deref()) {
        (IssueDecisionAnswerBy::Agent, None) => {}
        (IssueDecisionAnswerBy::Human, Some(reference)) => {
            super::validate_issue_decision_confirmation_reference(reference)?;
        }
        _ => return Err(IssueDecisionError::UnsupportedAnswerOrigin),
    }
    if let Some(note) = &value.note {
        shared_text(note.as_str(), "answer.note", MAX_ISSUE_DECISION_NOTE_BYTES)?;
    }
    if let Some(caller) = &value.caller {
        shared_text(caller, "answer.caller", 256)?;
    }
    identifier(&value.channel, "answer.channel", 128)?;
    timestamp(&value.at, "answer.at")
}

fn timestamp(value: &str, field: &'static str) -> Result<(), IssueDecisionError> {
    if value.len() > 64 || chrono::DateTime::parse_from_rfc3339(value).is_err() {
        return Err(IssueDecisionError::InvalidField { field });
    }
    Ok(())
}

fn sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
