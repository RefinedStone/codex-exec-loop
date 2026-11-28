//! 저장소별 이슈 수집 조립이다. 생성 시 원격 호출이나 모델 요청을 하지 않는다.
use std::path::PathBuf;
use std::sync::Arc;

use crate::adapter::outbound::db::SqliteIssueAutomationRestartRegistryAdapter;
use crate::adapter::outbound::db::sqlite_issue_automation_store::SqliteIssueAutomationStoreAdapter;
use crate::adapter::outbound::db::sqlite_planning_authority_adapter::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::issue_automation_policy::FilesystemIssueAutomationPolicySourceAdapter;
use crate::adapter::outbound::filesystem::issue_creation_journal::IssueCreationJournalAdapter;
use crate::adapter::outbound::filesystem::issue_policy::FilesystemIssuePolicySourceAdapter;
use crate::adapter::outbound::filesystem::issue_publish_intent_journal::IssuePublishIntentJournalAdapter;
use crate::adapter::outbound::git::issue_origin::GitIssueCollectionOriginAdapter;
use crate::adapter::outbound::github::issue_board_field::GithubIssueBoardFieldAdapter;
use crate::application::port::outbound::IssueAutomationRestartRegistryPort;
use crate::application::port::outbound::issue_board_field_port::IssueBoardFieldPort;
use crate::application::port::outbound::issue_feedback_store_port::IssueFeedbackStorePort;
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_collection::IssueCollectionService;
use crate::application::service::issue_feedback::IssueFeedbackService;
use crate::application::service::issue_policy::IssuePolicyService;
use crate::application::service::issue_source_provenance::IssueSourceProvenanceService;
use crate::application::service::issue_triage::IssueTriageService;

mod arbiter;
mod jira;

/// All production entry points share the issue authority profile's discovery registry.
/// The adapter resolves the path only on an operation, never while assembling a service.
pub(crate) fn build_restart_registry() -> Arc<dyn IssueAutomationRestartRegistryPort> {
    Arc::new(SqliteIssueAutomationRestartRegistryAdapter::new(
        restart_registry_runtime,
    ))
}

#[cfg(not(test))]
fn restart_registry_runtime() -> anyhow::Result<PathBuf> {
    Ok(SqlitePlanningAuthorityAdapter::authority_home_root()?
        .join("issue-automation")
        .join("runtime"))
}

#[cfg(test)]
fn restart_registry_runtime() -> anyhow::Result<PathBuf> {
    static RUNTIME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    Ok(RUNTIME
        .get_or_init(|| {
            crate::test_utils::platform_safe_temp_dir().join(format!(
                "akra-issue-restart-registry-tests-{}-{:016x}",
                std::process::id(),
                rand::random::<u64>()
            ))
        })
        .clone())
}

pub(crate) fn build(workspace_dir: &str) -> Arc<IssueCollectionService> {
    build_with_restart_registry(workspace_dir, build_restart_registry())
}

pub(crate) fn build_with_restart_registry(
    workspace_dir: &str,
    restart_registry: Arc<dyn IssueAutomationRestartRegistryPort>,
) -> Arc<IssueCollectionService> {
    let admission = Arc::new(
        crate::application::service::issue_binding_admission::IssueBindingAdmission::new(),
    );
    admission.close_all();
    build_with_admission_and_restart_registry(workspace_dir, admission, restart_registry)
}

fn build_with_admission_and_restart_registry(
    workspace_dir: &str,
    admission: Arc<crate::application::service::issue_binding_admission::IssueBindingAdmission>,
    restart_registry: Arc<dyn IssueAutomationRestartRegistryPort>,
) -> Arc<IssueCollectionService> {
    build_with_runtime_services(workspace_dir, admission, restart_registry, (None, None))
}

pub(super) type IssueRuntimeServices = (
    Option<super::provider_credentials::ProviderServices>,
    Option<crate::adapter::outbound::app_server::SharedAppServer>,
);

pub(crate) fn build_with_provider_services(
    workspace_dir: &str,
    admission: Arc<crate::application::service::issue_binding_admission::IssueBindingAdmission>,
    services: IssueRuntimeServices,
) -> Arc<IssueCollectionService> {
    build_with_runtime_services(workspace_dir, admission, build_restart_registry(), services)
}

fn build_with_runtime_services(
    workspace_dir: &str,
    admission: Arc<crate::application::service::issue_binding_admission::IssueBindingAdmission>,
    restart_registry: Arc<dyn IssueAutomationRestartRegistryPort>,
    (providers, app_server): IssueRuntimeServices,
) -> Arc<IssueCollectionService> {
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
    let claims: Arc<dyn crate::application::port::outbound::issue_source_claim_port::IssueSourceClaimPort> = Arc::new(
        crate::adapter::outbound::db::sqlite_issue_source_claims::SqliteIssueSourceClaimsAdapter::new(restart_registry_runtime),
    );
    let credential_reader = providers.as_ref().map(|services| services.credentials.clone() as Arc<dyn crate::application::port::outbound::provider_credentials_port::ProviderCredentialReadPort>);
    // Projects 보드는 `gh` 의 같은 로그인을 쓴다. 보드 쓰기는 피드백 일지를 지난 의도만 보낸다 (#2565).
    let board_fields: Arc<dyn IssueBoardFieldPort> = Arc::new(GithubIssueBoardFieldAdapter::new(
        PathBuf::from(workspace_dir),
    ));
    let feedback = Arc::new(
        IssueFeedbackService::new(
            workspace_dir.to_owned(),
            store.clone(),
            Arc::new(jira::RoutedIssueFeedback::new(
                workspace_dir,
                store.clone(),
                claims.clone(),
                providers.as_ref(),
            )),
            publication.clone(),
            IssueAutomationPolicyService::new(Arc::new(
                FilesystemIssueAutomationPolicySourceAdapter::new(),
            )),
            IssuePolicyService::new(validation_source.clone()),
        )
        .with_issue_binding_admission(admission.clone())
        .with_board_fields(board_fields.clone()),
    );
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
            Arc::new(jira::RoutedIssueSource::new(
                workspace_dir,
                store.clone(),
                claims.clone(),
                providers,
            )),
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
        )
        .with_board_fields(
            board_fields,
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
    let execution = build_execution_with_admission(workspace_dir, admission.clone(), store.clone());
    let binding_pause = Arc::new(
        crate::application::service::issue_binding_pause::IssueBindingPauseService::new(
            workspace_dir.to_owned(),
            store.clone(),
            IssueAutomationPolicyService::new(Arc::new(
                FilesystemIssueAutomationPolicySourceAdapter::new(),
            )),
            execution.clone(),
            admission.clone(),
        ),
    );
    let comment_commands = build_comment_commands(workspace_dir, store.clone(), execution.clone());
    let autonomy = Arc::new(
        crate::application::service::issue_autonomy::IssueAutonomyService::new(
            workspace_dir.to_owned(),
            store.clone(),
            IssueAutomationPolicyService::new(Arc::new(
                FilesystemIssueAutomationPolicySourceAdapter::new(),
            )),
            execution.clone(),
        )
        .with_restart_registry(
            restart_registry,
            super::production::issue_automation_restart_target,
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
        .with_account_quota(super::production::issue_quota_adapter(app_server))
        .with_issue_binding_admission(admission.clone())
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
        .with_connected_source_collection(
            IssueAutomationPolicyService::new(Arc::new(
                FilesystemIssueAutomationPolicySourceAdapter::new(),
            )),
            credential_reader,
            claims,
        )
        .with_triage(triage)
        .with_autonomy(autonomy)
        .with_interactions(interactions)
        .with_binding_pause(binding_pause)
        .with_coordinator(coordinator)
        .with_comment_commands(comment_commands)
        .with_publication_journal(publication),
    )
}

/// GitHub 댓글 명령은 같은 장부의 개입 경로로 `hold` 만 적용한다. 조립은 원격 호출을 하지 않는다.
fn build_comment_commands(
    workspace_dir: &str,
    store: Arc<SqliteIssueAutomationStoreAdapter>,
    execution: Arc<crate::composition::issue_work_execution::AkraIssueWorkExecutionAdapter>,
) -> Arc<crate::application::service::issue_comment_command::IssueCommentCommandService> {
    use crate::application::port::outbound::issue_work_steering_port::UnavailableIssueWorkSteering;
    let interventions = Arc::new(
        crate::application::service::issue_intervention::IssueInterventionService::new(
            workspace_dir.to_owned(),
            store.clone(),
            execution,
            Arc::new(UnavailableIssueWorkSteering),
        ),
    );
    Arc::new(
        crate::application::service::issue_comment_command::IssueCommentCommandService::new(
            workspace_dir.to_owned(),
            store.clone(),
            store,
            Arc::new(
                crate::adapter::outbound::github::issue_feedback::GithubIssueFeedbackAdapter::new(
                    PathBuf::from(workspace_dir),
                ),
            ),
            IssueAutomationPolicyService::new(Arc::new(
                FilesystemIssueAutomationPolicySourceAdapter::new(),
            )),
            interventions,
        ),
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
        )
        .with_execution(build_execution(workspace_dir)),
    )
}

#[cfg(test)]
pub(crate) fn build_for_kernel_test(workspace_dir: &str) -> Arc<IssueCollectionService> {
    let admission = Arc::new(
        crate::application::service::issue_binding_admission::IssueBindingAdmission::new(),
    );
    admission.close_all();
    build_for_kernel_test_with_admission(workspace_dir, admission)
}

#[cfg(test)]
pub(crate) fn build_for_kernel_test_with_admission(
    workspace_dir: &str,
    admission: Arc<crate::application::service::issue_binding_admission::IssueBindingAdmission>,
) -> Arc<IssueCollectionService> {
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
    let execution = build_execution_with_admission(workspace_dir, admission.clone(), store.clone());
    let binding_pause = Arc::new(
        crate::application::service::issue_binding_pause::IssueBindingPauseService::new(
            workspace_dir.to_owned(),
            store.clone(),
            IssueAutomationPolicyService::new(Arc::new(TestAutomationPolicy)),
            execution.clone(),
            admission,
        ),
    );
    let autonomy = Arc::new(
        crate::application::service::issue_autonomy::IssueAutonomyService::new(
            workspace_dir.to_owned(),
            store.clone(),
            IssueAutomationPolicyService::new(Arc::new(TestAutomationPolicy)),
            execution.clone(),
        ),
    );
    let interactions = Arc::new(
        crate::application::service::issue_interactions::IssueInteractionsService::new(
            workspace_dir.to_owned(),
            store.clone(),
            execution.clone(),
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
        .with_interactions(interactions)
        .with_binding_pause(binding_pause),
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
    build_autonomy_with_restart_registry(workspace_dir, build_restart_registry())
}

pub(crate) fn build_autonomy_with_restart_registry(
    workspace_dir: &str,
    restart_registry: Arc<dyn IssueAutomationRestartRegistryPort>,
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
        )
        .with_restart_registry(
            restart_registry,
            super::production::issue_automation_restart_target,
        ),
    )
}

fn build_execution(
    workspace_dir: &str,
) -> Arc<crate::composition::issue_work_execution::AkraIssueWorkExecutionAdapter> {
    let admission = Arc::new(
        crate::application::service::issue_binding_admission::IssueBindingAdmission::new(),
    );
    admission.close_all();
    let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(|workspace| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
        Ok(PathBuf::from(location.runtime_dir))
    }));
    build_execution_with_admission(workspace_dir, admission, store)
}

fn build_execution_with_admission(
    workspace_dir: &str,
    admission: Arc<crate::application::service::issue_binding_admission::IssueBindingAdmission>,
    store: Arc<dyn crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort>,
) -> Arc<crate::composition::issue_work_execution::AkraIssueWorkExecutionAdapter> {
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let delivery_evidence =
        super::production::task_delivery_evidence_for_workspace(workspace_dir, authority.as_ref());
    let parallel = crate::application::service::parallel_mode::ParallelModeService::new(
        authority.clone(),
        Arc::new(crate::adapter::outbound::github::automation::GithubAutomationAdapter::new()),
        Arc::new(crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter::new()),
    )
    .with_delivery_evidence(delivery_evidence)
    .with_issue_binding_admission(admission, store.clone());
    Arc::new(
        crate::composition::issue_work_execution::AkraIssueWorkExecutionAdapter::new(
            authority.clone(),
            authority,
            parallel,
        )
        .with_criteria_store(store),
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

#[cfg(test)]
mod restart_registry_tests {
    use super::*;
    use crate::application::port::outbound::IssueAutomationRestartEntry;
    use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
    use crate::domain::issue_automation::autonomy::{IssueAutonomyIntent, IssueAutonomyLevel};

    struct Fixture {
        root: PathBuf,
        authority_project: PathBuf,
        workspace: String,
        registry_runtime: PathBuf,
        registry: Arc<SqliteIssueAutomationRestartRegistryAdapter>,
        entry: IssueAutomationRestartEntry,
    }

    impl Fixture {
        fn new() -> Self {
            let root = crate::test_utils::platform_safe_temp_dir().join(format!(
                "akra-restart-composition-{}-{:016x}",
                std::process::id(),
                rand::random::<u64>()
            ));
            crate::private_directory::ensure_private_directory(&root).unwrap();
            let checkout = root.join("repository");
            crate::private_directory::ensure_private_directory(&checkout).unwrap();
            let output = crate::test_utils::git_command()
                .arg("-C")
                .arg(&checkout)
                .args(["init", "-b", "prerelease"])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let target = crate::composition::production::repository_kernel_target(
                checkout.to_str().unwrap(),
            )
            .unwrap();
            let location = SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(
                &target.workspace_root,
            )
            .unwrap();
            let runtime = PathBuf::from(location.runtime_dir);
            let authority_project = runtime.parent().unwrap().to_path_buf();
            let store = SqliteIssueAutomationStoreAdapter::new(move |_| Ok(runtime.clone()));
            let intent = IssueAutonomyIntent {
                command_id: "completed-lower".into(),
                level: IssueAutonomyLevel::Off,
                expected_revision: 0,
                policy_version: "a".repeat(64),
            };
            store
                .begin_issue_autonomy(&target.workspace_root, &intent)
                .unwrap();
            store
                .finish_issue_autonomy(&target.workspace_root, &intent)
                .unwrap();
            let entry = crate::composition::production::issue_automation_restart_target(
                &target.workspace_root,
            )
            .unwrap()
            .unwrap();
            let registry_runtime = root.join("restart-runtime");
            let registry = Arc::new(SqliteIssueAutomationRestartRegistryAdapter::with_runtime(
                registry_runtime.clone(),
            ));
            Self {
                root,
                authority_project,
                workspace: target.workspace_root,
                registry_runtime,
                registry,
                entry,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
            let _ = std::fs::remove_dir_all(&self.authority_project);
        }
    }

    #[test]
    fn production_collection_and_startup_autonomy_reconcile_the_injected_registry() {
        let fixture = Fixture::new();
        fixture.registry.register(&fixture.entry).unwrap();
        let collection = build_with_restart_registry(&fixture.workspace, fixture.registry.clone());
        collection
            .autonomy_service()
            .unwrap()
            .recover_before_runtime()
            .unwrap();
        assert!(fixture.registry.list().unwrap().is_empty());

        fixture.registry.register(&fixture.entry).unwrap();
        build_autonomy_with_restart_registry(&fixture.workspace, fixture.registry.clone())
            .recover_before_runtime()
            .unwrap();
        assert!(fixture.registry.list().unwrap().is_empty());
    }

    #[test]
    fn production_registry_assembly_keeps_binding_pause_on_the_shared_admission() {
        use crate::application::service::issue_binding_admission::IssueBindingAdmission;
        use crate::domain::issue_automation::{IssueAutomationBinding, IssueAutomationBindingId};
        use crate::domain::issue_work::{IssueProjectScope, IssueProvider};

        let fixture = Fixture::new();
        fixture.registry.register(&fixture.entry).unwrap();
        let admission = Arc::new(IssueBindingAdmission::new());
        let collection = build_with_admission_and_restart_registry(
            &fixture.workspace,
            admission.clone(),
            fixture.registry.clone(),
        );
        collection
            .autonomy_service()
            .unwrap()
            .recover_before_runtime()
            .unwrap();
        assert!(fixture.registry.list().unwrap().is_empty());

        let store = collection.automation_store();
        let first = IssueAutomationBindingId::new("restart-binding-first").unwrap();
        let second = IssueAutomationBindingId::new("restart-binding-second").unwrap();
        for (id, repository) in [(&first, "repository-first"), (&second, "repository-second")] {
            let binding = IssueAutomationBinding::new(
                id.clone(),
                IssueProvider::GitHub,
                "github.com",
                repository,
                IssueProjectScope {
                    workspace_dir: fixture.workspace.clone(),
                },
            )
            .unwrap();
            store.insert_binding(&fixture.workspace, &binding).unwrap();
        }
        let pause = collection.binding_pause_service().unwrap();
        pause.recover_before_runtime().unwrap();
        let generation = admission.capture(&first).unwrap();
        let guard = admission.acquire(&first, generation).unwrap();
        let accepted = pause
            .set(
                "restart-pause-first".into(),
                first.clone(),
                true,
                generation,
                None,
            )
            .unwrap();
        assert!(accepted.receipt.is_none());
        assert!(admission.capture(&first).is_err());
        assert_eq!(admission.capture(&second).unwrap(), 0);
        assert!(!pause.poll().unwrap());
        assert!(
            store
                .get_issue_binding_pause_command(&fixture.workspace, "restart-pause-first")
                .unwrap()
                .unwrap()
                .receipt
                .is_none(),
            "the collection must not complete Pause while the shared owner still has a guard"
        );
        drop(guard);
        assert!(pause.poll().unwrap());
        let stored = store
            .issue_binding_pause_state(&fixture.workspace, &first)
            .unwrap();
        assert!(stored.paused);
        assert!(stored.pending.is_none());
        assert_eq!(stored, pause.state(&first).unwrap());
        assert_eq!(admission.capture(&second).unwrap(), 0);
    }

    #[test]
    fn production_assembly_and_state_queries_do_not_create_restart_discovery() {
        let fixture = Fixture::new();
        let collection = build_with_restart_registry(&fixture.workspace, fixture.registry.clone());
        let startup =
            build_autonomy_with_restart_registry(&fixture.workspace, fixture.registry.clone());
        let _ = collection.autonomy_state();
        let _ = startup.state();
        let _ = startup.prepare_confirmation("read-only-preview", IssueAutonomyLevel::Off);
        assert!(!fixture.registry_runtime.exists());
        assert!(fixture.registry.list().unwrap().is_empty());
        assert!(!fixture.registry_runtime.exists());
    }
}
