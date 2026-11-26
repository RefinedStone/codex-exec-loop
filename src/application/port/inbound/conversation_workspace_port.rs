use anyhow::Result;

use crate::domain::conversation_workspace::{
    ConversationWorkspaceQuery, ConversationWorkspaceRecord, PrepareConversationWorkspaceRequest,
};

pub trait ConversationWorkspacePort: Send + Sync {
    fn prepare(
        &self,
        request: PrepareConversationWorkspaceRequest,
    ) -> Result<ConversationWorkspaceRecord>;
    /// 마지막으로 확정한 준비 상태이다. 연결된 thread나 현재 작업 트리의 clean 여부는 주장하지 않는다.
    fn get(&self, query: ConversationWorkspaceQuery)
    -> Result<Option<ConversationWorkspaceRecord>>;
}
