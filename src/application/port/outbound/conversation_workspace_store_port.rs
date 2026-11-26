use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::domain::conversation_workspace::ConversationWorkspaceRecord;

pub struct ConversationWorkspaceStoreLocation {
    pub repository_dir: String,
    pub runtime_dir: PathBuf,
}

/// 프로세스 공유 예약은 git 생성과 마지막 상태 저장까지 유지한다. save는 매번 durable commit한다.
pub trait ConversationWorkspaceTransaction {
    fn repository_dir(&self) -> &str;
    fn workspaces_dir(&self) -> &Path;
    fn find(&mut self, request_id: &str) -> Result<Option<ConversationWorkspaceRecord>>;
    fn save(&mut self, record: &ConversationWorkspaceRecord) -> Result<()>;
}

pub trait ConversationWorkspaceStorePort: Send + Sync {
    fn begin(&self, cwd: &str) -> Result<Box<dyn ConversationWorkspaceTransaction + '_>>;
    fn get(&self, cwd: &str, request_id: &str) -> Result<Option<ConversationWorkspaceRecord>>;
}
