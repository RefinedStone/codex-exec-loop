//! 저장소별 이슈 수집 조립이다. 생성 시 원격 호출이나 모델 요청을 하지 않는다.
use std::path::PathBuf;
use std::sync::Arc;

use crate::adapter::outbound::db::sqlite_issue_automation_store::SqliteIssueAutomationStoreAdapter;
use crate::adapter::outbound::db::sqlite_planning_authority_adapter::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::issue_automation_policy::FilesystemIssueAutomationPolicySourceAdapter;
use crate::adapter::outbound::filesystem::issue_policy::FilesystemIssuePolicySourceAdapter;
use crate::adapter::outbound::git::issue_origin::GitIssueCollectionOriginAdapter;
use crate::adapter::outbound::github::issue_source::GithubIssueSourceAdapter;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_collection::IssueCollectionService;
use crate::application::service::issue_policy::IssuePolicyService;
use crate::application::service::issue_triage::IssueTriageService;

pub(crate) fn build(workspace_dir: &str) -> Arc<IssueCollectionService> {
    let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(|workspace| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
        Ok(PathBuf::from(location.runtime_dir))
    }));
    let validation_source = Arc::new(FilesystemIssuePolicySourceAdapter::new());
    let triage = Arc::new(IssueTriageService::new(
        workspace_dir.to_string(),
        store.clone(),
        IssueAutomationPolicyService::new(Arc::new(
            FilesystemIssueAutomationPolicySourceAdapter::new(),
        )),
        IssuePolicyService::new(validation_source.clone()),
    ));
    Arc::new(
        IssueCollectionService::new(
            workspace_dir.to_string(),
            store,
            Arc::new(GithubIssueSourceAdapter::new(PathBuf::from(workspace_dir))),
            Arc::new(GitIssueCollectionOriginAdapter::new()),
            IssuePolicyService::new(validation_source),
        )
        .with_triage(triage),
    )
}

#[cfg(test)]
pub(crate) fn build_for_kernel_test(workspace_dir: &str) -> Arc<IssueCollectionService> {
    use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
    use crate::application::port::outbound::issue_collection_origin_port::{
        IssueCollectionOrigin, IssueCollectionOriginPort,
    };
    use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
    use crate::application::port::outbound::issue_source_port::{
        IssueDiscoveryPage, IssueDiscoveryRequest, IssueResyncRequest, IssueSourceCapabilities,
        IssueSourcePort,
    };
    use crate::domain::issue_automation::source::{IssueSourceFailure, IssueSourceRead};

    struct TestOrigin;
    impl IssueCollectionOriginPort for TestOrigin {
        fn github_origin(&self, _: &str) -> anyhow::Result<IssueCollectionOrigin> {
            anyhow::bail!("kernel tests do not read a provider origin")
        }
    }
    struct TestPolicy;
    impl IssuePolicySourcePort for TestPolicy {
        fn load_user_policy_toml(&self) -> anyhow::Result<Option<String>> {
            Ok(None)
        }
        fn load_project_policy_toml(&self, _: &str) -> anyhow::Result<Option<String>> {
            Ok(None)
        }
    }
    struct TestAutomationPolicy;
    impl IssueAutomationPolicySourcePort for TestAutomationPolicy {
        fn load_user_policy_toml(&self) -> anyhow::Result<Option<String>> {
            Ok(None)
        }
        fn load_project_policy_toml(&self, _: &str) -> anyhow::Result<Option<String>> {
            Ok(None)
        }
    }
    struct TestSource;
    impl IssueSourcePort for TestSource {
        fn capabilities(&self) -> IssueSourceCapabilities {
            IssueSourceCapabilities::read_only()
        }
        fn discover(
            &self,
            _: IssueDiscoveryRequest,
        ) -> Result<IssueDiscoveryPage, IssueSourceFailure> {
            panic!("kernel tests must inject an issue source before collecting")
        }
        fn resync(&self, _: IssueResyncRequest) -> IssueSourceRead {
            panic!("kernel tests must inject an issue source before collecting")
        }
    }
    let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(|workspace| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
        Ok(PathBuf::from(location.runtime_dir))
    }));
    let triage = Arc::new(IssueTriageService::new(
        workspace_dir.to_string(),
        store.clone(),
        IssueAutomationPolicyService::new(Arc::new(TestAutomationPolicy)),
        IssuePolicyService::new(Arc::new(TestPolicy)),
    ));
    Arc::new(
        IssueCollectionService::new(
            workspace_dir.to_string(),
            store,
            Arc::new(TestSource),
            Arc::new(TestOrigin),
            IssuePolicyService::new(Arc::new(TestPolicy)),
        )
        .with_triage(triage),
    )
}
