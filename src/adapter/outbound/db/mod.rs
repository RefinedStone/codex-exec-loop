// DB outbound adapter 영역은 planning authority를 SQLite에 저장하고 읽는 구현을 담는다.
// 이 module declaration이 실제 adapter 파일을 `adapter::outbound::db` namespace에 연결한다.
pub(crate) mod sqlite_delegation_store;
pub mod sqlite_issue_automation_restart_registry;
pub mod sqlite_issue_automation_store;
pub use sqlite_issue_automation_restart_registry::SqliteIssueAutomationRestartRegistryAdapter;
pub mod sqlite_planning_authority_adapter;
pub mod sqlite_pr_review_journal;
pub(crate) mod sqlite_work_context_store;

// re-export는 wiring code가 긴 하위 경로 대신 public adapter 이름만 의존하게 한다.
pub use self::sqlite_planning_authority_adapter::{
    SqlitePlanningAuthorityAdapter, SqliteTelegramGlobalRunnerLeaseAdapter,
};

pub use self::sqlite_issue_automation_store::SqliteIssueAutomationStoreAdapter;
pub(crate) mod extension_state_store;
