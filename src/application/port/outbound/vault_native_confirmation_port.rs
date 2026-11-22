//! 실제 OS 대화상자의 사용자 확인이다. 외부 입력으로 확인 결과나 목적을 주입하지 않는다.
use std::sync::atomic::AtomicBool;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VaultNativeConfirmationPurpose {
    ViewOriginal,
    RewriteBinding,
}

pub(crate) struct VaultNativeConfirmationRequest {
    pub issue_url: String,
    pub workspace_dir: String,
    pub purpose: VaultNativeConfirmationPurpose,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VaultNativeConfirmationOutcome {
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    Confirmed,
    Cancelled,
    Unsupported,
    Unavailable,
}
pub(crate) trait VaultNativeConfirmationPort: Send + Sync {
    fn confirm(
        &self,
        request: &VaultNativeConfirmationRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> VaultNativeConfirmationOutcome;
}
