//! 원문이 게시될 때 사용한 대표 checkout을 읽기만 하는 경계다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VaultRestoreWorkspaceError {
    Unavailable,
}
pub(crate) trait VaultRestoreWorkspacePort: Send + Sync {
    fn canonical_workspace(&self, workspace: &str) -> Result<String, VaultRestoreWorkspaceError>;
}
