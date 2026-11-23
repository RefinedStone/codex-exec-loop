use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::db::sqlite_issue_automation_store::{
    SqliteIssueAutomationStoreAdapter, test_support::seed_issue_work,
};
use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};
use std::path::PathBuf;
use std::sync::Arc;

pub(super) fn store() -> Arc<SqliteIssueAutomationStoreAdapter> {
    Arc::new(SqliteIssueAutomationStoreAdapter::new(|workspace| {
        Ok(PathBuf::from(
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?
                .runtime_dir,
        ))
    }))
}

pub(super) fn issue(workspace: &str, issue_id: &str) -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_delivery_fixture".into(),
        issue_id: issue_id.into(),
        project_scope: IssueProjectScope {
            workspace_dir: workspace.into(),
        },
    }
}

pub(super) fn seed(workspace: &str, issue_id: &str, task_id: &str) -> String {
    seed_issue_work(
        store().as_ref(),
        &issue(workspace, issue_id),
        &[task_id.into()],
    )
    .unwrap()
    .order
    .work_order_id
}

pub(super) fn work_id(workspace: &str, issue_id: &str) -> String {
    crate::domain::issue_automation::coordinator::issue_work_order_id(&issue(workspace, issue_id))
}
