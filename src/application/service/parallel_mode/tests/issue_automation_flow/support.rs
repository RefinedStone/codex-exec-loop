use super::super::*;
use crate::adapter::outbound::db::SqliteIssueAutomationStoreAdapter;
use crate::adapter::outbound::filesystem::issue_publish_intent_journal::IssuePublishIntentJournalAdapter;
use crate::application::port::inbound::stored_command_port::{
    StoredCommandContext, issue_confirmation::VerifiedHumanConfirmation,
};
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::issue_work_execution_port::IssueWorkExecutionPort;
use crate::application::port::outbound::issue_work_steering_port::UnavailableIssueWorkSteering;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDocumentCommit, PlanningAuthorityPort,
};
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskAuthorityCommitResult;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_collection::{
    IssueCollectionCancellation, IssueCollectionService,
};
use crate::application::service::issue_coordinator::{
    IssueCoordinatorFact, IssueCoordinatorService,
};
use crate::application::service::issue_decision::IssueDecisionService;
use crate::application::service::issue_feedback::IssueFeedbackService;
use crate::application::service::issue_intervention::IssueInterventionService;
use crate::application::service::issue_policy::IssuePolicyService;
use crate::application::service::issue_triage::IssueTriageService;
use crate::composition::issue_work_execution::AkraIssueWorkExecutionAdapter;
use crate::domain::issue_automation::collection::IssueCollectionQuery;
use crate::domain::issue_automation::coordinator::IssueWorkCheckpoint;
use crate::domain::issue_automation::decision::IssueDecisionKind;
use crate::domain::issue_work::IssueReference;
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
    QueueIdleConfig,
};
use anyhow::{Result, anyhow, ensure};

mod feedback;
mod policy;
mod source;
mod worker;

pub(super) use feedback::FlowFeedbackProvider;
use policy::FlowPolicySource;
pub(super) use source::FlowSource;

pub(super) const ISSUE_URL: &str = "https://github.com/RefinedStone/codex-exec-loop/issues/2555";
pub(super) const PULL_REQUEST_URL: &str = "https://github.com/RefinedStone/codex-exec-loop/pull/77";

/// Owns the real repository and remote doubles across service reconstruction.
/// No task or work-order record is inserted by this fixture.
pub(super) struct FlowFixture {
    pub repo: TempGitRepo,
    pub issue: IssueReference,
    pub source: Arc<FlowSource>,
    pub provider: Arc<FlowFeedbackProvider>,
    pub github: Arc<FakeGithubAutomationPort>,
    policy: Arc<FlowPolicySource>,
}

pub(super) struct FlowServices {
    pub workspace: String,
    pub issue: IssueReference,
    pub store: Arc<SqliteIssueAutomationStoreAdapter>,
    pub planning: Arc<SqlitePlanningAuthorityAdapter>,
    pub execution: Arc<dyn IssueWorkExecutionPort>,
    pub coordinator: Arc<IssueCoordinatorService>,
    pub triage: Arc<IssueTriageService>,
    pub decisions: Arc<IssueDecisionService>,
    pub collection: IssueCollectionService,
    pub parallel: ParallelModeService,
    pub feedback: Arc<IssueFeedbackService>,
    pub interventions: Arc<IssueInterventionService>,
    pub publication: Arc<IssuePublishIntentJournalAdapter>,
}

fn runtime_directory(workspace: &str) -> Result<PathBuf> {
    let location = SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
    Ok(PathBuf::from(location.runtime_dir))
}

fn issue_store() -> Arc<SqliteIssueAutomationStoreAdapter> {
    Arc::new(SqliteIssueAutomationStoreAdapter::new(runtime_directory))
}

impl FlowFixture {
    pub fn new(prefix: &str) -> Self {
        let repo = TempGitRepo::new(prefix);
        let workspace = repo.workspace_dir();
        let planning = SqlitePlanningAuthorityAdapter::new();
        let initialized = planning
            .commit_planning_authority_documents(
                &workspace,
                PlanningAuthorityDocumentCommit {
                    observed_planning_revision: None,
                    directions: &DirectionCatalogDocument {
                        version: PLANNING_FORMAT_VERSION,
                        queue_idle: QueueIdleConfig::default(),
                        directions: vec![DirectionDefinition {
                            id: "issue-workstream".into(),
                            title: "Issue work".into(),
                            summary: "Implement the approved collected issue.".into(),
                            success_criteria: vec!["The approved result is delivered.".into()],
                            scope_hints: Vec::new(),
                            detail_doc_path: String::new(),
                            state: DirectionState::Active,
                        }],
                    },
                    task_authority: &TaskAuthorityDocument {
                        version: PLANNING_FORMAT_VERSION,
                        tasks: Vec::new(),
                    },
                    queue_projection: &PriorityQueueProjection {
                        next_task: None,
                        active_tasks: Vec::new(),
                        proposed_tasks: Vec::new(),
                        skipped_tasks: Vec::new(),
                    },
                    // Execution observation reads direction/task/result output together.
                    // Initialize the accepted empty document without inventing a result.
                    result_output_markdown: Some("# Result Output\n\n"),
                    active_document_mutations: &[],
                    retired_task_ids: &[],
                    authority_mutation_owner_token: None,
                },
            )
            .expect("real planning authority should initialize without creating a task");
        assert!(matches!(
            initialized,
            PlanningTaskAuthorityCommitResult::Committed { .. }
        ));
        let documents = planning
            .load_planning_authority_documents(&workspace)
            .expect("the real planning document snapshot should be readable")
            .expect("execution observation requires the accepted result output document");
        assert!(documents.task_authority.tasks.is_empty());
        assert_eq!(documents.result_output_markdown, "# Result Output\n\n");

        let policy = Arc::new(FlowPolicySource::new(&repo.root));
        let source = Arc::new(FlowSource::new(&workspace));
        let issue = source.issue();
        let provider = Arc::new(FlowFeedbackProvider::new(issue_store(), source.clone()));
        // The process has opted in. The provider starts without a human PR review.
        let github = Arc::new(FakeGithubAutomationPort::with_review_decision(
            "REVIEW_REQUIRED",
        ));
        Self {
            repo,
            issue,
            source,
            provider,
            github,
            policy,
        }
    }

    pub fn workspace(&self) -> String {
        self.repo.workspace_dir()
    }

    /// Configure the real user policy before collection and supply its matching remote label.
    pub fn require_selection_label(&self, label: &str) -> Result<()> {
        use crate::domain::issue_automation::source::{
            IssueLabelAttribution, IssueSourceLabel, IssueSourceRead,
        };
        self.policy.require_selection_label(label)?;
        let mut read = self.source.read.lock().unwrap();
        let IssueSourceRead::Observed(observed) = &mut *read else {
            anyhow::bail!("selection setup requires an observed remote issue");
        };
        let selected = IssueSourceLabel {
            id: "L_flow_selection".into(),
            name: label.into(),
        };
        observed.labels.items = vec![selected.clone()];
        observed.label_actors.items = vec![IssueLabelAttribution {
            label: selected,
            event_id: "LE_flow_selection".into(),
            actor: observed.author.clone(),
            created_at: "2026-11-21T00:00:00Z".into(),
        }];
        Ok(())
    }

    pub fn services(&self) -> FlowServices {
        self.services_with_execution(|real| real)
    }

    /// Only the execution boundary is decorated for a crash/ACK-loss experiment.
    /// The wrapped adapter still owns actual task registration, permits and observations.
    pub fn services_with_execution(
        &self,
        wrap: impl FnOnce(Arc<dyn IssueWorkExecutionPort>) -> Arc<dyn IssueWorkExecutionPort>,
    ) -> FlowServices {
        let workspace = self.workspace();
        let store = issue_store();
        let admission = Arc::new(
            crate::application::service::issue_binding_admission::IssueBindingAdmission::new(),
        );
        let planning = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let parallel = test_parallel_mode_service_with_autonomous_github(self.github.clone())
            .with_issue_binding_admission(admission.clone(), store.clone());
        let real: Arc<dyn IssueWorkExecutionPort> = Arc::new(AkraIssueWorkExecutionAdapter::new(
            planning.clone(),
            planning.clone(),
            parallel.clone(),
        ));
        let execution = wrap(real);
        let disclosure = || IssuePolicyService::new(self.policy.clone());
        let automation = || IssueAutomationPolicyService::new(self.policy.clone());
        let triage = Arc::new(IssueTriageService::new(
            workspace.clone(),
            store.clone(),
            automation(),
            disclosure(),
        ));
        let decisions = Arc::new(IssueDecisionService::new(
            workspace.clone(),
            store.clone(),
            triage.clone(),
            disclosure(),
        ));
        let interventions = Arc::new(
            IssueInterventionService::new(
                workspace.clone(),
                store.clone(),
                execution.clone(),
                Arc::new(UnavailableIssueWorkSteering),
            )
            .with_triage(triage.clone())
            .with_policy(disclosure()),
        );
        let publication = Arc::new(IssuePublishIntentJournalAdapter::new(runtime_directory));
        let feedback = Arc::new(
            IssueFeedbackService::new(
                workspace.clone(),
                store.clone(),
                self.provider.clone(),
                publication.clone(),
                automation(),
                disclosure(),
            )
            .with_issue_binding_admission(admission.clone()),
        );
        let coordinator = Arc::new(
            IssueCoordinatorService::new(
                workspace.clone(),
                store.clone(),
                self.source.clone(),
                triage.clone(),
                decisions.clone(),
                disclosure(),
                execution.clone(),
            )
            .with_issue_binding_admission(admission)
            .with_feedback(feedback.clone())
            .with_publication_journal(publication.clone()),
        );
        let collection = IssueCollectionService::new(
            workspace.clone(),
            store.clone(),
            self.source.clone(),
            self.source.clone(),
            disclosure(),
        )
        .with_triage(triage.clone())
        .with_coordinator(coordinator.clone())
        .with_publication_journal(publication.clone());
        FlowServices {
            workspace,
            issue: self.issue.clone(),
            store,
            planning,
            execution,
            coordinator,
            triage,
            decisions,
            collection,
            parallel,
            feedback,
            interventions,
            publication,
        }
    }
}

impl FlowServices {
    pub fn collect_and_triage(&self) -> Result<()> {
        let cancel = IssueCollectionCancellation::new();
        let outcome = self.collection.collect_once(&cancel, 1)?;
        ensure!(!outcome.discarded, "fixture collection was discarded");
        ensure!(
            outcome.projection.state.status.last_failure.is_none(),
            "fixture discovery failed: {:?}",
            outcome.projection.state.status.last_failure
        );
        self.triage.triage_all(&cancel, &|_| {})?;
        Ok(())
    }

    /// Facts are returned without kernel append or outbox ACK. Recovery tests own both.
    pub fn tick(&self, now_ms: u64) -> Result<Vec<IssueCoordinatorFact>> {
        let cancel = IssueCollectionCancellation::new();
        self.triage.triage_all(&cancel, &|_| {})?;
        let facts = Mutex::new(Vec::new());
        self.coordinator.tick(&cancel, now_ms, true, &|values| {
            facts.lock().unwrap().extend(values);
        })?;
        Ok(facts.into_inner().unwrap())
    }

    pub fn approve_start(&self) -> Result<()> {
        let page = self.triage.issues(&IssueCollectionQuery::default())?;
        let issue = page
            .items
            .iter()
            .find(|item| item.issue == self.issue)
            .ok_or_else(|| anyhow!("collected issue is missing"))?;
        let decisions: Vec<_> = issue
            .open_decisions
            .iter()
            .filter(|stored| stored.request.definition().kind == IssueDecisionKind::StartApproval)
            .collect();
        ensure!(
            decisions.len() == 1,
            "expected one current start approval, got {}",
            decisions.len()
        );
        let id = decisions[0].request.id();
        let preview = self.decisions.prepare_confirmation(id.as_str(), "start")?;
        let context = StoredCommandContext {
            human_confirmation: Some(VerifiedHumanConfirmation::from_verified(
                preview.target,
                format!("desktop-confirmation:{}", "e".repeat(32)),
            )?),
            caller: Some("Desktop".into()),
            channel: "gateway".into(),
            at: "2026-11-21T00:00:00Z".into(),
        };
        self.decisions
            .answer(id.as_str(), "start", None, &context)?;
        Ok(())
    }

    pub fn work(&self) -> Result<Option<IssueWorkCheckpoint>> {
        self.store.get_issue_work(&self.workspace, &self.issue)
    }

    pub fn task_ids(&self) -> Result<Vec<String>> {
        let Some(work) = self.work()? else {
            return Ok(Vec::new());
        };
        self.execution
            .task_ids(&self.workspace, &work.order.work_order_id)
    }
}
