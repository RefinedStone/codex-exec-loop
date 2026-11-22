use super::*;
use crate::application::port::outbound::issue_automation_store_port::IssueExecutionSaveResult;
use crate::application::port::outbound::issue_feedback_store_port::IssueFeedbackStorePort;
use crate::application::service::issue_collection::IssueDetailSnapshotChanged;
use crate::application::service::issue_feedback::tests::support::{FakeStore, pending_record};
use crate::domain::issue_automation::collection::IssueFeedbackDetailReason;
use crate::domain::issue_automation::feedback::IssueFeedbackIntentState;
use std::sync::atomic::AtomicUsize;

struct InterleavedPolicy {
    reads: AtomicUsize,
    after_read: Box<dyn Fn(usize) + Send + Sync>,
}

impl IssueAutomationPolicySourcePort for InterleavedPolicy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        let read = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
        (self.after_read)(read);
        Ok(Some(AUTO_POLICY.into()))
    }

    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

fn interleave_policy(
    h: &mut Harness,
    after_read: impl Fn(usize) + Send + Sync + 'static,
) -> Arc<InterleavedPolicy> {
    let policy = Arc::new(InterleavedPolicy {
        reads: AtomicUsize::new(0),
        after_read: Box::new(after_read),
    });
    h.triage = Arc::new(IssueTriageService::new(
        h.fixture.workspace.clone(),
        h.fixture.store(),
        IssueAutomationPolicyService::new(policy.clone()),
        IssuePolicyService::new(Arc::new(ValidationPolicy)),
    ));
    policy
}

fn save_interleaved_observation(
    store: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    issue: &IssueReference,
    observation: &IssueWorkExecutionObservation,
) {
    let work = store.get_issue_work(workspace, issue).unwrap().unwrap();
    let execution = store
        .get_issue_execution(workspace, issue)
        .unwrap()
        .unwrap();
    assert!(matches!(
        store
            .apply_issue_execution(
                workspace,
                &work,
                execution.progress.revision,
                observation,
                Some(3),
            )
            .unwrap(),
        IssueExecutionSaveResult::Saved { changed: true, .. }
    ));
}

fn catalog(h: &Harness) -> u64 {
    h.fixture
        .store()
        .read_triage_batch(&h.fixture.workspace, None, 1)
        .unwrap()
        .revision
}

#[test]
fn execution_detail_snapshot_retries_catalog_change_with_current_lineage_and_feedback() {
    let mut h = started();
    stored_review(&h);
    let store = h.fixture.store();
    let workspace = h.fixture.workspace.clone();
    let issue = h.fixture.reference("I_001");
    let integration = IssueExecutionIntegration {
        record_id: "integration-after-first-read".into(),
        source_sha: "a".repeat(40),
        integration_sha: "b".repeat(40),
        pull_request_url: Some("https://github.com/actual/review-target/pull/43".into()),
    };
    let observation = IssueWorkExecutionObservation {
        phase: IssueExecutionPhase::Integrated,
        task_ids: vec!["task-1".into(), "task-2".into()],
        active_task_id: Some("task-2".into()),
        execution_ids: vec!["task-1".into(), "task-2".into()],
        terminals: vec![IssueExecutionTerminalRecord {
            record_id: "contract-after-integration".into(),
            classification: IssueExecutionTerminalClass::ContractRecovery,
        }],
        current_terminal_id: Some("contract-after-integration".into()),
        review_pull_request_url: integration.pull_request_url.clone(),
        integration: Some(integration.clone()),
        ..Default::default()
    };
    let policy = interleave_policy(&mut h, move |read| {
        // Triage reads policy before its snapshot; execution's policy read follows its record read.
        if read == 2 {
            save_interleaved_observation(&store, &workspace, &issue, &observation);
        }
    });
    let calls_before = execution_calls(&h);
    let source_calls_before = h.source.calls.load(Ordering::SeqCst);
    let detail = collection(&h, true)
        .issue(&h.fixture.reference("I_001"))
        .unwrap();
    assert_eq!(policy.reads.load(Ordering::SeqCst), 4);
    assert_eq!(detail.revision, catalog(&h));
    assert_eq!(
        detail
            .item
            .as_ref()
            .unwrap()
            .work
            .as_ref()
            .unwrap()
            .task_ids,
        vec!["task-1", "task-2"]
    );
    let feedback = detail.feedback.as_ref().unwrap();
    assert_eq!(feedback.integration, Some(integration.clone()));
    let execution = ready(detail);
    assert_eq!(execution.state, IssueExecutionState::Done);
    assert_eq!(execution.state_label, "통합 완료");
    assert_eq!(execution.execution_attempts, 2);
    assert_eq!(
        execution.review_pull_request_url,
        integration.pull_request_url
    );
    assert_eq!(
        execution.recovery.unwrap().reason,
        IssueExecutionRecoveryReason::ContractRecovery
    );
    assert_eq!(execution_calls(&h), calls_before);
    assert_eq!(h.source.calls.load(Ordering::SeqCst), source_calls_before);
}

#[test]
fn execution_detail_snapshot_stops_after_two_changing_catalog_reads() {
    let mut h = started();
    let original = stored_review(&h);
    let store = h.fixture.store();
    let workspace = h.fixture.workspace.clone();
    let issue = h.fixture.reference("I_001");
    let policy = interleave_policy(&mut h, move |read| {
        if read % 2 == 0 {
            let observation = IssueWorkExecutionObservation {
                phase: IssueExecutionPhase::InReview,
                task_ids: vec!["task-1".into()],
                active_task_id: Some("task-1".into()),
                execution_ids: vec!["task-1".into()],
                review_pull_request_url: Some(format!(
                    "https://github.com/actual/review-target/pull/{}",
                    100 + read
                )),
                ..Default::default()
            };
            save_interleaved_observation(&store, &workspace, &issue, &observation);
        }
    });
    let calls_before = execution_calls(&h);
    let source_calls_before = h.source.calls.load(Ordering::SeqCst);
    let error = collection(&h, true)
        .issue(&h.fixture.reference("I_001"))
        .unwrap_err();
    assert!(error.is::<IssueDetailSnapshotChanged>());
    assert_eq!(
        policy.reads.load(Ordering::SeqCst),
        4,
        "only the initial assembly and one full retry are allowed"
    );
    let stored = h
        .fixture
        .store()
        .get_issue_execution(&h.fixture.workspace, &h.fixture.reference("I_001"))
        .unwrap()
        .unwrap();
    assert_eq!(stored.progress.revision, original.progress.revision + 2);
    assert_eq!(execution_calls(&h), calls_before);
    assert_eq!(h.source.calls.load(Ordering::SeqCst), source_calls_before);
}

#[test]
fn execution_detail_snapshot_retries_first_feedback_intent_without_catalog_change() {
    let mut h = started();
    stored_review(&h);
    let issue = h.fixture.reference("I_001");
    let feedback_store = Arc::new(FakeStore::new(issue.clone()));
    feedback_store.data.lock().unwrap().snapshot =
        h.fixture.store().snapshot(&issue).unwrap().unwrap();
    assert_eq!(feedback_store.snapshot_value().state.revision, 0);
    let mut first_intent = pending_record(issue.clone());
    first_intent.state = IssueFeedbackIntentState::Recorded;
    first_intent.revision = 1;
    first_intent.validate().unwrap();
    let changed_feedback = feedback_store.clone();
    *feedback_store.after_snapshot.lock().unwrap() = Some(Box::new(move || {
        let mut data = changed_feedback.data.lock().unwrap();
        data.records.push(first_intent);
        data.snapshot.state.revision = 1;
    }));
    let policy = interleave_policy(&mut h, |_| {});
    let external = Arc::new(NoExternal);
    let coordinator = h
        .coordinator()
        .with_feedback(Arc::new(IssueFeedbackService::new(
            h.fixture.workspace.clone(),
            feedback_store.clone(),
            external.clone(),
            external,
            IssueAutomationPolicyService::new(h.policy.clone()),
            IssuePolicyService::new(Arc::new(ValidationPolicy)),
        )));
    let service = collection(&h, false).with_coordinator(Arc::new(coordinator));
    let catalog_before = catalog(&h);
    let calls_before = execution_calls(&h);
    let source_calls_before = h.source.calls.load(Ordering::SeqCst);
    let detail = service.issue(&issue).unwrap();
    let feedback = detail.feedback.unwrap();
    assert_eq!(feedback.revision, 1);
    assert_eq!(feedback.publication_attempts, 0);
    assert_eq!(
        feedback.reason,
        Some(IssueFeedbackDetailReason::FeedbackUnconfirmed)
    );
    assert_eq!(policy.reads.load(Ordering::SeqCst), 4);
    assert_eq!(catalog(&h), catalog_before);
    assert_eq!(detail.revision, catalog_before);
    assert_ne!(detail.revision, feedback.revision);
    assert_eq!(execution_calls(&h), calls_before);
    assert_eq!(h.source.calls.load(Ordering::SeqCst), source_calls_before);
    let data = feedback_store.data.lock().unwrap();
    assert_eq!(data.records.len(), 1);
    assert_eq!(data.records[0].state, IssueFeedbackIntentState::Recorded);
    assert!(data.events.is_empty());
}
