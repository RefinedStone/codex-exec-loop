//! 보호된 로컬 열람 세션의 진입 계약이다. 승인 결과나 직렬화 가능한 평문을 받지 않는다.
use crate::application::port::outbound::evidence_vault_port::VaultRestoreResponse;
use crate::domain::issue_work::IssueReference;
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
    RecordUnavailable,
}

/// 구현이 대상 검증, native 확인, 한 번의 복원, 잠금 해제 권한의 수명을 소유한다.
pub(crate) trait VaultRestoreSessionPort: Send + Sync {
    fn prepare(&self, request: VaultRestoreSessionRequest) -> Result<(), VaultRestoreSessionHold>;
    fn restore(&self) -> Result<VaultRestoreResponse, VaultRestoreSessionHold>;
    fn cancel(&self);
    /// Consumed도 최종 응답을 보낼 수 있다. 취소와 기한만 열람 수명을 끝낸다.
    fn is_live(&self) -> bool;
    fn deadline(&self) -> Instant;
}
