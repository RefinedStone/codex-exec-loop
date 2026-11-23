//! 완료 기준 증거 명령의 입력이다. 경로·hash·tree·actor는 입력으로 받지 않는다.
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};

use crate::domain::planning::evidence::{
    TaskEvidenceStatus, validate_evidence_identifier, validate_evidence_request_id,
    validate_evidence_token,
};

const MAX_SAFE_REVISION: u64 = 9_007_199_254_740_991;
pub const MAX_TASK_EVIDENCE_REQUEST_ID_BYTES: usize = 128;
pub const MAX_TASK_EVIDENCE_NOTE_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareTaskEvidenceRequest {
    pub request_id: String,
    pub task_id: String,
    pub criterion_id: String,
    pub scope_id: String,
    pub expected_acceptance_revision: u64,
}

/// 짧은 예약만 접수한다. 실행 recipe와 실행 사실은 호스트가 결정한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestTaskVerificationRequest {
    pub request_id: String,
    pub task_id: String,
    pub criterion_id: String,
    pub scope_id: String,
    pub expected_acceptance_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordTaskEvidenceRequest {
    pub attempt_id: String,
    pub status: TaskEvidenceStatus,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaiveTaskCriterionRequest {
    pub request_id: String,
    pub task_id: String,
    pub criterion_id: String,
    pub scope_id: String,
    pub expected_acceptance_revision: u64,
    pub reason: String,
}

impl PrepareTaskEvidenceRequest {
    pub fn validate(&self) -> Result<()> {
        validate_evidence_request_id(&self.request_id)?;
        validate_scope_coordinates(
            &self.task_id,
            &self.criterion_id,
            &self.scope_id,
            self.expected_acceptance_revision,
        )
    }
}

impl RequestTaskVerificationRequest {
    pub fn validate(&self) -> Result<()> {
        validate_evidence_request_id(&self.request_id)?;
        validate_scope_coordinates(
            &self.task_id,
            &self.criterion_id,
            &self.scope_id,
            self.expected_acceptance_revision,
        )
    }
}

impl RecordTaskEvidenceRequest {
    pub fn validate(&self) -> Result<()> {
        validate_evidence_token(&self.attempt_id)?;
        ensure!(
            self.note
                .as_ref()
                .is_none_or(|note| note.len() <= MAX_TASK_EVIDENCE_NOTE_BYTES),
            "evidence note exceeds the byte limit"
        );
        Ok(())
    }
}

impl WaiveTaskCriterionRequest {
    pub fn validate(&self) -> Result<()> {
        validate_evidence_request_id(&self.request_id)?;
        validate_scope_coordinates(
            &self.task_id,
            &self.criterion_id,
            &self.scope_id,
            self.expected_acceptance_revision,
        )?;
        ensure!(
            !self.reason.trim().is_empty() && self.reason.len() <= MAX_TASK_EVIDENCE_NOTE_BYTES,
            "waiver reason must be nonblank and within the byte limit"
        );
        Ok(())
    }

    /// domain이 저장하는 정규화된 이유와 같은 digest다. 원본 명령은 broker fingerprint도 결속한다.
    pub fn reason_digest(&self) -> String {
        format!("{:x}", Sha256::digest(self.reason.trim().as_bytes()))
    }
}

fn validate_scope_coordinates(
    task_id: &str,
    criterion_id: &str,
    scope_id: &str,
    acceptance_revision: u64,
) -> Result<()> {
    // E1은 기준 ID의 내부 공백을 허용한다. 조회로 받은 정규화된 ID를 그대로 사용한다.
    validate_evidence_identifier(task_id)?;
    validate_evidence_identifier(criterion_id)?;
    validate_evidence_token(scope_id)?;
    ensure!(
        (1..=MAX_SAFE_REVISION).contains(&acceptance_revision),
        "invalid evidence acceptance revision"
    );
    Ok(())
}
