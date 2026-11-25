use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::domain::issue_automation::autonomy::IssueAutonomySelection;
use crate::domain::issue_automation::collection::{
    IssueCollectionConfig, IssueCollectionItem, IssueCollectionStatus, IssueTrackedSummary,
    StoredIssueCollection,
};
use crate::domain::issue_automation::coordinator::{IssueWorkCheckpoint, IssueWorkOrder};
use crate::domain::issue_automation::decision::{IssueDecisionBasis, IssueDecisionId};
use crate::domain::issue_automation::execution::IssueExecutionRecord;
use crate::domain::issue_automation::source::*;
use crate::domain::issue_automation::{
    IssueAutomationBindingId, IssueAutomationFlags, IssueAutomationSnapshot,
};
use crate::domain::issue_publish_intent::{IssuePublishIntentRecord, IssuePublishIntentState};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider, IssueSharedCopy,
};

mod provider;
mod store;
pub(crate) use provider::{FakeProvider, ReadBehavior, WriteBehavior};
pub(crate) use store::FakeStore;

pub(crate) const WORKSPACE: &str = "/feedback-workspace";
pub(crate) const WORK_ORDER: &str = "feedback-work";
pub(crate) const ACTOR: &str = "U_feedback";
pub(crate) const COMMENT: &str = "IC_feedback";
pub(crate) type Hook = Mutex<Option<Box<dyn FnOnce() + Send>>>;

pub(crate) struct Fixture {
    pub store: Arc<FakeStore>,
    pub provider: Arc<FakeProvider>,
    pub journal: Arc<FakePublication>,
    pub policy: Arc<FakePolicy>,
    pub admission: Arc<crate::application::service::issue_binding_admission::IssueBindingAdmission>,
}
impl Fixture {
    pub fn new(comment: bool, labels: bool) -> Self {
        let store = Arc::new(FakeStore::new(issue()));
        let journal = Arc::new(FakePublication::default());
        let provider = Arc::new(FakeProvider::new(store.clone(), journal.clone()));
        Self {
            store,
            provider,
            journal,
            policy: Arc::new(FakePolicy::new(comment, labels)),
            admission: Arc::new(
                crate::application::service::issue_binding_admission::IssueBindingAdmission::new(),
            ),
        }
    }
    pub fn service(&self) -> IssueFeedbackService {
        IssueFeedbackService::new(
            WORKSPACE.into(),
            self.store.clone(),
            self.provider.clone(),
            self.journal.clone(),
            IssueAutomationPolicyService::new(self.policy.clone()),
            IssuePolicyService::new(Arc::new(DefaultDisclosure)),
        )
        .with_issue_binding_admission(self.admission.clone())
    }
    pub fn sync(&self) -> Result<()> {
        self.service()
            .sync_issue(&issue(), 123, &IssueCollectionCancellation::new())
    }
    pub fn grant(&self) -> Result<bool> {
        self.service()
            .ensure_first_grant(&issue(), WORK_ORDER, &IssueCollectionCancellation::new())
    }
    pub fn writes(&self) -> Vec<IssueFeedbackIntent> {
        self.provider.writes.lock().unwrap().clone()
    }
}
pub(crate) fn issue() -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_feedback".into(),
        issue_id: "I_feedback".into(),
        project_scope: IssueProjectScope {
            workspace_dir: WORKSPACE.into(),
        },
    }
}
fn empty_metadata<T>() -> IssueMetadataList<T> {
    IssueMetadataList {
        items: Vec::new(),
        completeness: IssueMetadataCompleteness::Complete,
    }
}
fn snapshot(issue: IssueReference) -> IssueFeedbackSnapshot {
    let shared = IssueSharedCopy {
        title: "task".into(),
        body: "shared task".into(),
    };
    let hash = IssueFieldHash::of_fields(&shared.title, &shared.body);
    let binding_id = IssueAutomationBindingId::new("feedback-binding").unwrap();
    let source = IssueSourceObservation {
        content_problems: vec![],
        jira: None,
        linear: None,
        repository_visibility: Default::default(),
        repository_name_with_owner: None,
        issue: issue.clone(),
        web: None,
        remote_state: IssueRemoteState::Open,
        creation_provenance: IssueCreationProvenance::Unattributed,
        labels: empty_metadata(),
        assignees: empty_metadata(),
        author: None,
        label_actors: empty_metadata(),
        linked_pull_request: IssuePrPresence::Absent,
        content: IssueSourceContent::Shared(IssueAutomationSnapshot {
            basis: IssueComparisonBasis {
                field_hash: hash.clone(),
                provider_updated_at: "2026-11-21T00:00:00Z".into(),
            },
            shared_copy: shared.clone(),
            policy_version: "a".repeat(64),
        }),
    };
    let work = IssueWorkCheckpoint {
        delivery_assessment: None,
        delivery_revision: 0,
        authorization: None,
        permit_authorization_epoch: 0,
        order: IssueWorkOrder {
            work_order_id: WORK_ORDER.into(),
            issue: issue.clone(),
            field_hash: hash.clone(),
            title: shared.title.clone(),
            handoff: shared,
            acceptance: vec!["works".into()],
            scope: "src".into(),
            branch_hint: None,
            link_reference: None,
            provider_link: false,
            profile: None,
            priority: 0,
            attempt: 1,
        },
        start_authority:
            crate::domain::issue_automation::coordinator::IssueWorkStartAuthority::Human,
        decision_id: Some(IssueDecisionId::new("feedback-start").unwrap()),
        approved_basis: IssueDecisionBasis {
            delivery: None,
            authorization_epoch: 0,
            policy_version: "a".repeat(64),
            field_hash: Some(hash),
            work_order_id: None,
            gate_revision: 1,
        },
        task_ids: vec!["feedback-task".into()],
        permit_source: None,
        permit_revision: 0,
        delivery_granted: false,
        blocked_reason: None,
        revision: 1,
    };
    let mut execution = IssueExecutionRecord::new(WORK_ORDER.into());
    execution.progress.state = IssueExecutionState::Running;
    execution.progress.revision = 1;
    execution.progress.attempts.execution = 1;
    IssueFeedbackSnapshot {
        binding_pause: crate::domain::issue_automation::binding_pause::IssueBindingPauseState {
            binding_id: binding_id.clone(),
            revision: 0,
            paused: false,
            pending: None,
        },
        collection: StoredIssueCollection {
            config: IssueCollectionConfig::default(),
            epoch: 1,
            revision: 1,
            binding_id: Some(binding_id.clone()),
            discovery_cursor: None,
            resync_cursor: None,
            status: IssueCollectionStatus::default(),
        },
        item: IssueCollectionItem {
            binding_run_revision: 0,
            binding_run_blocked: false,
            start_budget_wait: None,
            issue: issue.clone(),
            binding_id,
            tracked: Some(IssueTrackedSummary {
                state: IssueAutomationState::Running,
                flags: IssueAutomationFlags::default(),
            }),
            current: Some(IssueSourceRead::Observed(Box::new(source))),
            observation_revision: Some(1),
            tracked_revision: Some(1),
            triage: None,
            open_decisions: Vec::new(),
            work: None,
        },
        work: Some(work),
        execution: Some(execution),
        autonomy: IssueAutonomySelection::default(),
        authorization_epoch: 0,
        pending_intervention: false,
        close_approval: None,
        state: IssueFeedbackState::new(issue),
        first_grant_entered: false,
        publication_attempts: 0,
    }
}
pub(crate) fn pending_feedback(issue: IssueReference) -> Arc<dyn IssueFeedbackStorePort> {
    let store = FakeStore::new(issue.clone());
    store
        .data
        .lock()
        .unwrap()
        .records
        .push(pending_record(issue));
    Arc::new(store)
}
pub(crate) fn pending_record(issue: IssueReference) -> IssueFeedbackIntentRecord {
    let intent_id = "pending-feedback";
    let content = "shared status";
    let body = format!("{content}\n\n{}", feedback_marker(intent_id));
    IssueFeedbackIntentRecord {
        intent: IssueFeedbackIntent {
            intent_id: intent_id.into(),
            issue,
            actor_id: ACTOR.into(),
            connection: None,
            automation_policy_version: "a".repeat(64),
            disclosure_policy_version: "b".repeat(64),
            desired_revision: 1,
            recorded_at: "test-time".into(),
            operation: IssueFeedbackOperation::CreateComment {
                body_sha256: feedback_body_hash(&body),
                body,
                content_sha256: feedback_body_hash(content),
            },
        },
        state: IssueFeedbackIntentState::NeedsConfirmation,
        revision: 2,
        receipt: None,
        acknowledged_comment_id: None,
        contradictory_response: false,
        closure_acknowledged: false,
    }
}
pub(crate) fn pending_publication(mut owner: IssueReference) -> IssuePublishIntentRecord {
    owner.project_scope.workspace_dir = "/same-runtime-alias".into();
    IssuePublishIntentRecord {
        intent_id: "pending-body".into(),
        issue: owner,
        previous_hash: IssueFieldHash::of_fields("before", "before"),
        approved_hash: IssueFieldHash::of_fields("after", "after"),
        pending_vault_fields: Vec::new(),
        state: IssuePublishIntentState::NeedsConfirmation,
        observed_hash: None,
        restore_link_finalized: false,

        restore_binding_attempt: None,
        write_response_mismatch: false,
        write_attempted: Some(true),
        sequence: 1,
    }
}

pub(crate) struct FakePolicy {
    pub user: Mutex<String>,
    pub project: Mutex<Option<String>>,
    pub fail: AtomicBool,
}
impl FakePolicy {
    fn new(comment: bool, labels: bool) -> Self {
        Self {
            user: Mutex::new(format!(
                "[feedback]\ncomment = {comment}\nlabels = {labels}\n"
            )),
            project: Mutex::new(None),
            fail: AtomicBool::new(false),
        }
    }
}
impl IssueAutomationPolicySourcePort for FakePolicy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        if self.fail.load(Ordering::SeqCst) {
            bail!("policy unavailable");
        }
        Ok(Some(self.user.lock().unwrap().clone()))
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(self.project.lock().unwrap().clone())
    }
}
pub(super) struct DefaultDisclosure;
impl IssuePolicySourcePort for DefaultDisclosure {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(None)
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}
#[derive(Default)]
pub(crate) struct FakePublication {
    pub held: AtomicBool,
    pub records: Mutex<Vec<IssuePublishIntentRecord>>,
}
struct Lease<'a>(&'a AtomicBool);
impl IssuePublicationLease for Lease<'_> {
    fn verify_identity(&self) -> Result<()> {
        anyhow::ensure!(self.0.load(Ordering::SeqCst), "publication lease released");
        Ok(())
    }
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
impl IssuePublishIntentJournalPort for FakePublication {
    fn acquire_publication_lease(&self, _: &str) -> Result<Box<dyn IssuePublicationLease + '_>> {
        anyhow::ensure!(
            !self.held.swap(true, Ordering::SeqCst),
            "nested publication lease"
        );
        Ok(Box::new(Lease(&self.held)))
    }
    fn save(&self, _: &str, record: &IssuePublishIntentRecord) -> Result<()> {
        self.records.lock().unwrap().push(record.clone());
        Ok(())
    }
    fn load(&self, _: &str) -> Result<Vec<IssuePublishIntentRecord>> {
        Ok(self.records.lock().unwrap().clone())
    }
}
