use super::*;
use crate::application::port::outbound::issue_collection_origin_port::{
    IssueCollectionOrigin, IssueCollectionOriginPort,
};
use crate::application::port::outbound::issue_feedback_port::*;
use crate::application::port::outbound::issue_publish_intent_journal_port::{
    IssuePublicationLease, IssuePublishIntentJournalPort,
};
use crate::application::service::issue_collection::IssueCollectionService;
use crate::domain::issue_automation::collection::IssueCollectionDetail;
use crate::domain::issue_automation::execution::*;
use crate::domain::issue_automation::feedback::IssueFeedbackIntent;
use crate::domain::issue_publish_intent::IssuePublishIntentRecord;

struct NoExternal;
impl IssueCollectionOriginPort for NoExternal {
    fn github_origin(&self, _: &str) -> Result<IssueCollectionOrigin> {
        panic!("detail queries do not resolve a provider origin")
    }
}
impl IssueFeedbackPort for NoExternal {
    fn actor_id(&self, _: &IssueReference) -> Result<String> {
        panic!("detail queries do not read a provider actor")
    }
    fn read_label(&self, _: &IssueReference, _: &str) -> Result<IssueFeedbackLabelSnapshot> {
        panic!("detail queries do not read provider labels")
    }
    fn read_closure(&self, _: &IssueReference) -> Result<IssueFeedbackClosureSnapshot> {
        panic!("detail queries do not recheck provider closure")
    }
    fn write(&self, _: &IssueFeedbackIntent) -> Result<IssueFeedbackWriteResult> {
        panic!("detail queries do not publish or close issues")
    }
    fn reconcile(&self, _: &IssueFeedbackIntent) -> Result<IssueFeedbackReadResult> {
        panic!("detail queries do not reconcile feedback")
    }
}
impl IssuePublishIntentJournalPort for NoExternal {
    fn acquire_publication_lease(&self, _: &str) -> Result<Box<dyn IssuePublicationLease + '_>> {
        panic!("detail queries do not enter the publication workflow")
    }
    fn save(&self, _: &str, _: &IssuePublishIntentRecord) -> Result<()> {
        panic!("detail queries do not change publication intents")
    }
    fn load(&self, _: &str) -> Result<Vec<IssuePublishIntentRecord>> {
        panic!("detail queries do not start publication recovery")
    }
}

fn collection(h: &Harness, with_feedback: bool) -> IssueCollectionService {
    let external = Arc::new(NoExternal);
    let mut coordinator = h.coordinator();
    if with_feedback {
        coordinator = coordinator.with_feedback(Arc::new(IssueFeedbackService::new(
            h.fixture.workspace.clone(),
            h.fixture.store(),
            external.clone(),
            external.clone(),
            IssueAutomationPolicyService::new(h.policy.clone()),
            IssuePolicyService::new(Arc::new(ValidationPolicy)),
        )));
    }
    IssueCollectionService::new(
        h.fixture.workspace.clone(),
        h.fixture.store(),
        h.source.clone(),
        external,
        IssuePolicyService::new(Arc::new(ValidationPolicy)),
    )
    .with_triage(h.triage.clone())
    .with_coordinator(Arc::new(coordinator))
}

fn started() -> Harness {
    let h = Harness::new();
    h.tick(true).unwrap();
    h.answer("start");
    h.tick(true).unwrap();
    h
}

fn stored_review(h: &Harness) -> IssueExecutionRecord {
    let work = h.work().unwrap();
    let observed = IssueWorkExecutionObservation {
        phase: IssueExecutionPhase::InReview,
        task_ids: work.task_ids.clone(),
        active_task_id: Some("task-1".into()),
        execution_ids: vec!["task-1".into()],
        review_pull_request_url: Some("https://github.com/actual/review-target/pull/42".into()),
        ..Default::default()
    };
    h.fixture
        .store()
        .apply_issue_execution(&h.fixture.workspace, &work, 0, &observed, Some(3))
        .unwrap();
    h.fixture
        .store()
        .get_issue_execution(&h.fixture.workspace, &work.order.issue)
        .unwrap()
        .unwrap()
}

fn ready(detail: IssueCollectionDetail) -> IssueExecutionReadyDetail {
    match detail.execution {
        Some(IssueExecutionDetail::Ready(ready)) => *ready,
        other => panic!("expected ready execution detail, got {other:?}"),
    }
}

fn execution_calls(h: &Harness) -> (usize, usize, usize, usize, usize, usize, usize) {
    let state = h.execution.state.lock().unwrap();
    (
        state.observed_work_orders.len(),
        state.registrations,
        state.grants.len(),
        state.revoked_work_orders.len(),
        state.execution_limits.len(),
        state.recoveries.len(),
        state.control_commands.len(),
    )
}

#[test]
fn execution_detail_queries_do_not_observe_publish_recover_or_mutate_policy() {
    let h = started();
    let record = stored_review(&h);
    let store = h.fixture.store();
    let work_before = h.work();
    let collection_before = store.get_collection(&h.fixture.workspace).unwrap();
    let events_before = store
        .pending_issue_work_events(&h.fixture.workspace)
        .unwrap();
    let calls_before = execution_calls(&h);
    let remote_reads_before = h.source.calls.load(Ordering::SeqCst);
    let policy_before = h.policy.text.lock().unwrap().clone();
    let service = collection(&h, true);
    for _ in 0..3 {
        let view = ready(service.issue(&h.fixture.reference("I_001")).unwrap());
        assert_eq!(view.state, IssueExecutionState::InReview);
        assert_eq!(view.state_label, "리뷰 중");
        assert_eq!(view.execution_attempts, 1);
        assert_eq!(view.execution_attempt_limit, Some(3));
        assert_eq!(
            view.review_pull_request_url,
            record.progress.review_pull_request_url
        );
    }
    assert_eq!(execution_calls(&h), calls_before);
    assert_eq!(h.source.calls.load(Ordering::SeqCst), remote_reads_before);
    assert_eq!(*h.policy.text.lock().unwrap(), policy_before);
    assert_eq!(h.work(), work_before);
    assert_eq!(
        store.get_collection(&h.fixture.workspace).unwrap(),
        collection_before
    );
    assert_eq!(
        store
            .pending_issue_work_events(&h.fixture.workspace)
            .unwrap(),
        events_before
    );
    assert_eq!(
        store
            .get_issue_execution(&h.fixture.workspace, &h.fixture.reference("I_001"))
            .unwrap(),
        Some(record)
    );
}

#[test]
fn execution_detail_reads_current_limit_and_preserves_unknown_policy_without_fencing() {
    let h = started();
    let record = stored_review(&h);
    *h.policy.text.lock().unwrap() = format!("{AUTO_POLICY}\n[budget]\nmax_attempts = 7\n");
    let service = collection(&h, false);
    let calls_before = execution_calls(&h);
    let view = ready(service.issue(&h.fixture.reference("I_001")).unwrap());
    assert_eq!(view.execution_attempt_limit, Some(7));
    h.policy.unavailable.store(true, Ordering::SeqCst);
    let unknown = ready(service.issue(&h.fixture.reference("I_001")).unwrap());
    assert_eq!(unknown.execution_attempt_limit, None);
    assert_eq!(
        unknown.execution_attempts,
        record.progress.attempts.execution
    );
    assert_eq!(unknown.state, record.progress.state);
    assert_eq!(
        execution_calls(&h),
        calls_before,
        "display reads do not fence or restart execution"
    );
    assert_eq!(
        h.fixture
            .store()
            .get_issue_execution(&h.fixture.workspace, &h.fixture.reference("I_001"))
            .unwrap(),
        Some(record)
    );
}

#[test]
fn execution_detail_missing_record_is_distinct_from_corruption_and_feedback_read_failure() {
    let h = started();
    let service = collection(&h, false);
    assert!(
        service
            .issue(&h.fixture.reference("I_001"))
            .unwrap()
            .execution
            .is_none()
    );
    let record = stored_review(&h);
    let database =
        rusqlite::Connection::open(h.fixture.runtime.join("issue-automation.db")).unwrap();
    assert_eq!(database.execute(
        "UPDATE issue_execution_progress SET record='{}' WHERE workspace_dir=?1 AND work_order_id=?2",
        rusqlite::params![&h.fixture.workspace, &record.work_order_id],
    ).unwrap(), 1);
    drop(database);
    let failed = service.issue(&h.fixture.reference("I_001")).unwrap();
    assert_eq!(
        failed.execution,
        Some(IssueExecutionDetail::Unavailable {
            reason: IssueExecutionUnavailableReason::ReadFailed,
        })
    );
    assert!(
        collection(&h, true)
            .issue(&h.fixture.reference("I_001"))
            .is_err(),
        "a simultaneous feedback snapshot failure is not hidden as absent feedback"
    );
}

mod snapshot;
