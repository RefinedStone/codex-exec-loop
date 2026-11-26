//! 새 사용자 대화의 준비 요청을 저장소별 private runtime에 조립한다.
use std::path::PathBuf;
use std::sync::Arc;

use crate::adapter::outbound::db::conversation_workspace_store::ConversationWorkspaceStoreAdapter;
use crate::adapter::outbound::db::sqlite_planning_authority_adapter::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::git::{history::GitHistoryAdapter, workspace::GitWorkspaceAdapter};
use crate::application::port::inbound::conversation_workspace_port::ConversationWorkspacePort;
use crate::application::port::outbound::conversation_workspace_store_port::ConversationWorkspaceStoreLocation;
use crate::application::service::conversation_workspace::ConversationWorkspaceService;

pub(crate) fn build() -> Arc<dyn ConversationWorkspacePort> {
    Arc::new(ConversationWorkspaceService::new(
        Arc::new(GitHistoryAdapter),
        Arc::new(GitWorkspaceAdapter),
        Arc::new(ConversationWorkspaceStoreAdapter::new(|cwd, create| {
            let location = if create {
                Some(SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(cwd)?)
            } else {
                SqlitePlanningAuthorityAdapter::resolve_authority_location_for_read(cwd)?
            };
            Ok(location.map(|location| ConversationWorkspaceStoreLocation {
                repository_dir: location.canonical_repo_root,
                runtime_dir: PathBuf::from(location.runtime_dir),
            }))
        })),
    ))
}
