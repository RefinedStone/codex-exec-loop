use super::{
    ConversationRuntimeSnapshot, ConversationSnapshot, PlanningParallelProjection,
    SessionCatalogSnapshot, StartupSnapshot,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSnapshot {
    pub revision: u64,
    pub startup: StartupSnapshot,
    pub session_catalog: SessionCatalogSnapshot,
    pub conversation: ConversationSnapshot,
    pub conversation_runtime: ConversationRuntimeSnapshot,
    pub planning_parallel: PlanningParallelProjection,
}

impl AppSnapshot {
    pub fn initial() -> Self {
        Self {
            revision: 0,
            startup: StartupSnapshot::Idle,
            session_catalog: SessionCatalogSnapshot::Idle,
            conversation: ConversationSnapshot::Idle,
            conversation_runtime: ConversationRuntimeSnapshot::initial(),
            planning_parallel: PlanningParallelProjection::initial(),
        }
    }
}
