//! 보호된 로컬 열람·재연결 세션의 진입 계약이다. 승인 결과나 저장 token을 받지 않는다.
use crate::application::port::outbound::evidence_vault_port::VaultRestoreResponse;
use crate::application::port::outbound::issue_snapshot_port::IssueRawFields;
use crate::domain::issue_work::IssueReference;
use std::fmt;
use std::time::{Duration, Instant};

pub(crate) const MAX_VIEWING_SESSION: Duration = Duration::from_secs(5 * 60);

#[derive(Clone)]
pub(crate) struct VaultRestoreSessionRequest {
    pub issue_url: String,
    pub issue: IssueReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VaultRestoreSessionHold {
    PlatformUnsupported,
    ConfirmationCancelled,
    SourceUnavailable,
    Locked,
    LatestReadFailed,
    NotBound,
    PublishedCopyChanged,
    MarkerMoved,
    MarkerDuplicated,
    MarkerMissing,
    ChangedDuringReview,
    MappingRejected,
    RecordUnavailable,
    WriteOutcomeUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VaultRestoreSessionPrepared {
    pub reconnect: bool,
}

/// 재연결 가능 여부는 이 응답을 만든 세션 단계가 결정한다. 이유만 보고 추론하지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VaultRestoreSessionFailure {
    pub reason: VaultRestoreSessionHold,
    pub can_reconnect: bool,
}

impl VaultRestoreSessionFailure {
    pub(crate) fn terminal(reason: VaultRestoreSessionHold) -> Self {
        Self {
            reason,
            can_reconnect: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VaultRestoreSessionField {
    Title,
    Body,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VaultRestoreSessionPlace {
    pub field: VaultRestoreSessionField,
    /// 게시본 UTF-8의 바이트 위치이다. 이전 자리는 현재 게시본 길이와 무관하다.
    pub position: usize,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct VaultRestoreSessionCandidate {
    pub place: VaultRestoreSessionPlace,
    pub context: String,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct VaultRestoreSessionMarker {
    pub marker: String,
    pub previous: VaultRestoreSessionPlace,
    pub candidates: Vec<VaultRestoreSessionCandidate>,
}

/// private 검토 basis와 grant가 없는 화면용 투영이다. 전용 adapter만 wire로 변환한다.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct VaultRestoreSessionReview {
    /// 정규 판 번호, 콜론, SHA-256 소문자 64자리이다.
    pub reviewed_hash: String,
    pub published: IssueRawFields,
    pub markers: Vec<VaultRestoreSessionMarker>,
}

impl fmt::Debug for VaultRestoreSessionReview {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VaultRestoreSessionReview(<redacted>)")
    }
}

pub(crate) struct VaultRestoreSessionMapping {
    pub marker: String,
    pub place: VaultRestoreSessionPlace,
}

pub(crate) struct VaultRestoreReconnectRequest {
    pub reviewed_hash: String,
    pub mappings: Vec<VaultRestoreSessionMapping>,
}

/// 구현이 대상 검증, 두 native 확인, 원래 검토와 권한, 단회 재연결의 수명을 소유한다.
pub(crate) trait VaultRestoreSessionPort: Send + Sync {
    fn prepare(
        &self,
        request: VaultRestoreSessionRequest,
    ) -> Result<VaultRestoreSessionPrepared, VaultRestoreSessionHold>;
    fn restore(&self) -> Result<VaultRestoreResponse, VaultRestoreSessionFailure>;
    fn review(&self) -> Result<VaultRestoreSessionReview, VaultRestoreSessionHold>;
    fn reconnect(
        &self,
        request: VaultRestoreReconnectRequest,
    ) -> Result<(), VaultRestoreSessionHold>;
    fn cancel(&self);
    /// Consumed도 최종 응답을 보낼 수 있다. 취소와 기한만 열람 수명을 끝낸다.
    fn is_live(&self) -> bool;
    fn deadline(&self) -> Instant;
}
