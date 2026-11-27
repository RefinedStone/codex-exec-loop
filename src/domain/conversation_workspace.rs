//! 새 사용자 대화를 위해 준비한 작업 공간이다. thread 소유권이나 연결 성공을 뜻하지 않는다.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::git_graph::is_git_commit_id;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareConversationWorkspaceRequest {
    pub cwd: String,
    pub request_id: String,
    pub base_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationWorkspaceQuery {
    pub cwd: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationWorkspaceState {
    Preparing,
    Prepared,
    /// 실패·외부 변경·확인 불가 상태다. 기존 공간과 브랜치를 그대로 둔다.
    Preserved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationWorkspaceRecord {
    pub request_id: String,
    pub repository_dir: String,
    pub workspace_dir: String,
    pub branch: String,
    pub base_commit: String,
    pub state: ConversationWorkspaceState,
    pub reason: Option<String>,
}

pub fn validate_request_id(request_id: &str) -> Result<()> {
    ensure!(
        !request_id.is_empty()
            && request_id.len() <= 128
            && request_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "workspace requestId must contain 1..=128 ASCII letters, digits, '-' or '_'"
    );
    Ok(())
}

pub fn request_key(request_id: &str) -> String {
    format!("{:x}", Sha256::digest(request_id.as_bytes()))
}

impl PrepareConversationWorkspaceRequest {
    pub fn validate(&self) -> Result<()> {
        validate_request_id(&self.request_id)?;
        ensure!(
            is_git_commit_id(&self.base_commit),
            "baseCommit must be a full commit SHA"
        );
        ensure!(
            std::path::Path::new(&self.cwd).is_absolute(),
            "cwd must be absolute"
        );
        Ok(())
    }
}

/// A read-only inventory, never a deletion authorization.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationWorkspaceInventoryQuery {
    pub cwd: String,
    #[serde(default)]
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationWorkspaceInventoryEntry {
    pub record: ConversationWorkspaceRecord,
    pub current_head: Option<String>,
    pub reasons: Vec<String>,
    pub cleanup_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationWorkspaceInventoryPage {
    pub items: Vec<ConversationWorkspaceInventoryEntry>,
    pub next_after: Option<String>,
}

pub const WORKSPACE_INVENTORY_LIMIT: usize = 50;
