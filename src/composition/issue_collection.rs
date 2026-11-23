//! 저장소별 이슈 수집 조립이다. 생성 시 원격 호출이나 모델 요청을 하지 않는다.
use std::path::PathBuf;
use std::sync::Arc;

use crate::adapter::outbound::db::sqlite_issue_automation_store::SqliteIssueAutomationStoreAdapter;
use crate::adapter::outbound::db::sqlite_planning_authority_adapter::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::issue_automation_policy::FilesystemIssueAutomationPolicySourceAdapter;
use crate::adapter::outbound::filesystem::issue_creation_journal::IssueCreationJournalAdapter;
use crate::adapter::outbound::filesystem::issue_policy::FilesystemIssuePolicySourceAdapter;
use crate::adapter::outbound::filesystem::issue_publish_intent_journal::IssuePublishIntentJournalAdapter;
use crate::adapter::outbound::git::issue_origin::GitIssueCollectionOriginAdapter;
use crate::adapter::outbound::github::issue_feedback::GithubIssueFeedbackAdapter;
use crate::adapter::outbound::github::issue_source::GithubIssueSourceAdapter;
use crate::application::port::outbound::issue_feedback_store_port::IssueFeedbackStorePort;
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_collection::IssueCollectionService;
use crate::application::service::issue_feedback::IssueFeedbackService;
use crate::application::service::issue_policy::IssuePolicyService;
use crate::application::service::issue_source_provenance::IssueSourceProvenanceService;
use crate::application::service::issue_triage::IssueTriageService;

mod arbiter;

pub(crate) fn build(workspace_dir: &str) -> Arc<IssueCollectionService> {
    let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(|workspace| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
        Ok(PathBuf::from(location.runtime_dir))
    }));
    let publication: Arc<dyn IssuePublishIntentJournalPort> =
        Arc::new(IssuePublishIntentJournalAdapter::new(|workspace| {
            let location =
                SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
            Ok(PathBuf::from(location.runtime_dir))
        }));
    let validation_source = Arc::new(FilesystemIssuePolicySourceAdapter::new());
    let feedback = Arc::new(IssueFeedbackService::new(
        workspace_dir.to_owned(),
        store.clone(),
        Arc::new(GithubIssueFeedbackAdapter::new(PathBuf::from(
            workspace_dir,
        ))),
        publication.clone(),
        IssueAutomationPolicyService::new(Arc::new(
            FilesystemIssueAutomationPolicySourceAdapter::new(),
        )),
        IssuePolicyService::new(validation_source.clone()),
    ));
    let triage = Arc::new(IssueTriageService::new(
        workspace_dir.to_string(),
        store.clone(),
        IssueAutomationPolicyService::new(Arc::new(
            FilesystemIssueAutomationPolicySourceAdapter::new(),
        )),
        IssuePolicyService::new(validation_source.clone()),
    ));
    let source = Arc::new(
        IssueSourceProvenanceService::new(
            Arc::new(GithubIssueSourceAdapter::new(PathBuf::from(workspace_dir))),
            Arc::new(
                IssueCreationJournalAdapter::new(|workspace| {
                    let location =
                        SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(
                            workspace,
                        )?;
                    Ok(PathBuf::from(location.runtime_dir))
                })
                .with_workspace_resolver(|cwd| {
                    Ok(super::production::repository_kernel_target(cwd)?.workspace_root)
                }),
            ),
        )
        .with_feedback(
            store.clone(),
            IssueAutomationPolicyService::new(Arc::new(
                FilesystemIssueAutomationPolicySourceAdapter::new(),
            )),
        ),
    );
    let decisions = Arc::new(
        crate::application::service::issue_decision::IssueDecisionService::new(
            workspace_dir.to_owned(),
            store.clone(),
            triage.clone(),
            IssuePolicyService::new(validation_source.clone()),
        ),
    );
    let execution = build_execution(workspace_dir);
    let autonomy = Arc::new(
        crate::application::service::issue_autonomy::IssueAutonomyService::new(
            workspace_dir.to_owned(),
            store.clone(),
            IssueAutomationPolicyService::new(Arc::new(
                FilesystemIssueAutomationPolicySourceAdapter::new(),
            )),
            execution.clone(),
        ),
    );
    let interactions = Arc::new(
        crate::application::service::issue_interactions::IssueInteractionsService::new(
            workspace_dir.to_owned(),
            store.clone(),
            execution.clone(),
            triage.clone(),
            IssuePolicyService::new(validation_source.clone()),
        ),
    );
    let coordinator = Arc::new(
        crate::application::service::issue_coordinator::IssueCoordinatorService::new(
            workspace_dir.to_owned(),
            store.clone(),
            source.clone(),
            triage.clone(),
            decisions,
            IssuePolicyService::new(validation_source.clone()),
            execution,
        )
        .with_arbiter(
            arbiter::model(workspace_dir),
            Arc::new(crate::adapter::outbound::git::issue_assessment_evidence::GitIssueAssessmentEvidenceAdapter::new()),
        )
        .with_feedback(feedback)
        .with_publication_journal(publication.clone()),
    );
    Arc::new(
        IssueCollectionService::new(
            workspace_dir.to_owned(),
            store,
            source,
            Arc::new(GitIssueCollectionOriginAdapter::new()),
            IssuePolicyService::new(validation_source),
        )
        .with_triage(triage)
        .with_autonomy(autonomy)
        .with_interactions(interactions)
        .with_coordinator(coordinator)
        .with_publication_journal(publication),
    )
}

/// 결정 요청도 수집과 같은 저장소별 SQLite 장부와 현재 정책을 사용한다.
/// 조립은 원격 호출을 하지 않으며, 요청 자동 생성은 이 함수의 책임이 아니다.
pub(crate) fn build_decisions(
    workspace_dir: &str,
) -> Arc<crate::application::service::issue_decision::IssueDecisionService> {
    let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(|workspace| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
        Ok(PathBuf::from(location.runtime_dir))
    }));
    let policy = Arc::new(FilesystemIssuePolicySourceAdapter::new());
    let triage = Arc::new(IssueTriageService::new(
        workspace_dir.to_string(),
        store.clone(),
        IssueAutomationPolicyService::new(Arc::new(
            FilesystemIssueAutomationPolicySourceAdapter::new(),
        )),
        IssuePolicyService::new(policy.clone()),
    ));
    Arc::new(
        crate::application::service::issue_decision::IssueDecisionService::new(
            workspace_dir.to_string(),
            store,
            triage,
            IssuePolicyService::new(policy),
        ),
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
    let autonomy = Arc::new(
        crate::application::service::issue_autonomy::IssueAutonomyService::new(
            workspace_dir.to_owned(),
            store.clone(),
            IssueAutomationPolicyService::new(Arc::new(TestAutomationPolicy)),
            build_execution(workspace_dir),
        ),
    );
    let interactions = Arc::new(
        crate::application::service::issue_interactions::IssueInteractionsService::new(
            workspace_dir.to_owned(),
            store.clone(),
            build_execution(workspace_dir),
            triage.clone(),
            IssuePolicyService::new(Arc::new(TestPolicy)),
        ),
    );
    Arc::new(
        IssueCollectionService::new(
            workspace_dir.to_string(),
            store,
            Arc::new(TestSource),
            Arc::new(TestOrigin),
            IssuePolicyService::new(Arc::new(TestPolicy)),
        )
        .with_triage(triage)
        .with_autonomy(autonomy)
        .with_interactions(interactions),
    )
}

/// 개입 명령은 같은 영속 장부와 실제 kernel worker steering 경계를 사용한다.
pub(crate) fn build_interventions(
    workspace_dir: &str,
    steering: Arc<
        dyn crate::application::port::outbound::issue_work_steering_port::IssueWorkSteeringPort,
    >,
) -> Arc<crate::application::service::issue_intervention::IssueInterventionService> {
    let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(|workspace| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
        Ok(PathBuf::from(location.runtime_dir))
    }));
    let validation_source = Arc::new(FilesystemIssuePolicySourceAdapter::new());
    let triage = Arc::new(IssueTriageService::new(
        workspace_dir.to_owned(),
        store.clone(),
        IssueAutomationPolicyService::new(Arc::new(
            FilesystemIssueAutomationPolicySourceAdapter::new(),
        )),
        IssuePolicyService::new(validation_source.clone()),
    ));
    let execution = build_execution(workspace_dir);
    Arc::new(
        crate::application::service::issue_intervention::IssueInterventionService::new(
            workspace_dir.to_owned(),
            store,
            execution,
            steering,
        )
        .with_triage(triage)
        .with_policy(IssuePolicyService::new(validation_source)),
    )
}

/// Startup recovery constructs local adapters only; it does not fetch or start a worker.
pub(crate) fn build_autonomy(
    workspace_dir: &str,
) -> Arc<crate::application::service::issue_autonomy::IssueAutonomyService> {
    let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(|workspace| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
        Ok(PathBuf::from(location.runtime_dir))
    }));
    Arc::new(
        crate::application::service::issue_autonomy::IssueAutonomyService::new(
            workspace_dir.to_owned(),
            store,
            IssueAutomationPolicyService::new(Arc::new(
                FilesystemIssueAutomationPolicySourceAdapter::new(),
            )),
            build_execution(workspace_dir),
        ),
    )
}

fn build_execution(
    workspace_dir: &str,
) -> Arc<crate::composition::issue_work_execution::AkraIssueWorkExecutionAdapter> {
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let delivery_evidence =
        super::production::task_delivery_evidence_for_workspace(workspace_dir, authority.as_ref());
    let parallel = crate::application::service::parallel_mode::ParallelModeService::new(
        authority.clone(),
        Arc::new(crate::adapter::outbound::github::automation::GithubAutomationAdapter::new()),
        Arc::new(crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter::new()),
    )
    .with_delivery_evidence(delivery_evidence);
    Arc::new(
        crate::composition::issue_work_execution::AkraIssueWorkExecutionAdapter::new(
            authority.clone(),
            authority,
            parallel,
        ),
    )
}

/// 직접 제목·본문 게시도 같은 private issue DB의 미확정 feedback을 확인한다.
pub(crate) fn build_feedback_store() -> Arc<dyn IssueFeedbackStorePort> {
    Arc::new(SqliteIssueAutomationStoreAdapter::new(|workspace| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
        Ok(PathBuf::from(location.runtime_dir))
    }))
}
