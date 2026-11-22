//! A later writer must not replace the fact returned by an earlier transaction.
use super::*;
use crate::application::port::outbound::issue_automation_store_port::*;
use crate::domain::issue_automation::collection::{
    IssueCollectionFence, IssueCollectionItem, IssueResyncBatch,
};
use crate::domain::issue_automation::decision::{
    IssueDecisionId, IssueDecisionRequest, StoredIssueDecision,
};
use crate::domain::issue_automation::triage::IssueTriageAssessment;
use crate::domain::issue_automation::{IssueAutomationBinding, TrackedIssue};

#[derive(Clone, Copy, PartialEq, Eq)]
enum RaceAfter {
    Mode,
    ConfirmedMode,
    Attempt,
    RejectedDiscovery,
}

struct RacingStore {
    inner: Arc<SqliteIssueAutomationStoreAdapter>,
    race: RaceAfter,
    projection_reads: AtomicUsize,
}

impl RacingStore {
    fn new(fixture: &Fixture, race: RaceAfter) -> Arc<Self> {
        Arc::new(Self {
            inner: fixture.store(),
            race,
            projection_reads: AtomicUsize::new(0),
        })
    }

    fn set_competing_mode(&self, workspace: &str, mode: IssueCollectionMode) {
        let state = self.inner.get_collection(workspace).unwrap().unwrap();
        assert!(matches!(
            self.inner
                .set_collection_mode(workspace, Some(state.epoch), mode)
                .unwrap(),
            IssueCollectionModeSaveResult::Saved { changed: true, .. }
        ));
    }
}

macro_rules! forward_store {
    ($(fn $name:ident($($argument:ident: $kind:ty),*) -> $result:ty;)*) => {
        $(fn $name(&self, $($argument: $kind),*) -> $result {
            self.inner.$name($($argument),*)
        })*
    };
}

impl IssueAutomationStorePort for RacingStore {
    fn get_collection_projection(
        &self,
        workspace: &str,
    ) -> Result<Option<IssueCollectionProjection>> {
        self.projection_reads.fetch_add(1, Ordering::SeqCst);
        self.inner.get_collection_projection(workspace)
    }

    fn set_collection_mode(
        &self,
        workspace: &str,
        expected_epoch: Option<u64>,
        mode: IssueCollectionMode,
    ) -> Result<IssueCollectionModeSaveResult> {
        let result = self
            .inner
            .set_collection_mode(workspace, expected_epoch, mode)?;
        if self.race == RaceAfter::Mode {
            assert_eq!(mode, IssueCollectionMode::Off);
            self.set_competing_mode(workspace, IssueCollectionMode::Observe);
        }
        Ok(result)
    }

    fn set_collection_mode_exact(
        &self,
        workspace: &str,
        expected: IssueCollectionFence,
        from: IssueCollectionMode,
        mode: IssueCollectionMode,
    ) -> Result<IssueCollectionModeSaveResult> {
        if self.race == RaceAfter::ConfirmedMode {
            self.set_competing_mode(workspace, IssueCollectionMode::Observe);
            self.set_competing_mode(workspace, IssueCollectionMode::Off);
        }
        self.inner
            .set_collection_mode_exact(workspace, expected, from, mode)
    }

    fn apply_discovery_page(
        &self,
        workspace: &str,
        expected: IssueCollectionFence,
        request: &IssueDiscoveryRequest,
        page: &IssueDiscoveryPage,
    ) -> Result<IssueCollectionApplyResult> {
        if self.race == RaceAfter::RejectedDiscovery {
            self.set_competing_mode(workspace, IssueCollectionMode::Off);
        }
        let result = self
            .inner
            .apply_discovery_page(workspace, expected, request, page)?;
        if self.race == RaceAfter::RejectedDiscovery {
            assert!(matches!(
                result,
                IssueCollectionApplyResult::Discarded { .. }
            ));
            self.set_competing_mode(workspace, IssueCollectionMode::Observe);
        }
        Ok(result)
    }

    fn record_collection_attempt(
        &self,
        workspace: &str,
        expected: IssueCollectionFence,
        at_unix_ms: u64,
        failure: Option<IssueCollectionFailure>,
    ) -> Result<IssueCollectionApplyResult> {
        let result = self
            .inner
            .record_collection_attempt(workspace, expected, at_unix_ms, failure)?;
        if self.race == RaceAfter::Attempt {
            assert!(matches!(result, IssueCollectionApplyResult::Applied(_)));
            self.set_competing_mode(workspace, IssueCollectionMode::Off);
        }
        Ok(result)
    }

    forward_store! {
        fn get_issue_work(workspace: &str, issue: &IssueReference) -> Result<Option<crate::domain::issue_automation::coordinator::IssueWorkCheckpoint>>;
        fn save_issue_work(workspace: &str, expected: IssueTriageFence, assessment: &IssueTriageAssessment, candidate: &crate::domain::issue_automation::coordinator::IssueWorkCheckpoint) -> Result<crate::application::port::outbound::issue_automation_store_port::IssueWorkSaveResult>;
        fn apply_issue_resync(workspace: &str, expected: IssueCollectionFence, issue: &IssueReference, read: &IssueSourceRead) -> Result<IssueCollectionApplyResult>;
        fn insert_binding(workspace: &str, binding: &IssueAutomationBinding) -> Result<IssueAutomationBindingInsertResult>;
        fn get_binding(workspace: &str, binding_id: &IssueAutomationBindingId) -> Result<Option<StoredIssueAutomationBinding>>;
        fn list_bindings(workspace: &str) -> Result<Vec<StoredIssueAutomationBinding>>;
        fn save_binding(workspace: &str, expected_revision: u64, binding: &IssueAutomationBinding) -> Result<IssueAutomationBindingSaveResult>;
        fn insert_issue(workspace: &str, issue: &TrackedIssue) -> Result<IssueAutomationIssueInsertResult>;
        fn get_issue(workspace: &str, reference: &IssueReference) -> Result<Option<TrackedIssue>>;
        fn list_issues(workspace: &str, binding_id: Option<&IssueAutomationBindingId>) -> Result<Vec<TrackedIssue>>;
        fn save_issue(workspace: &str, expected_revision: u64, issue: &TrackedIssue) -> Result<IssueAutomationIssueSaveResult>;
        fn get_collection(workspace: &str) -> Result<Option<StoredIssueCollection>>;
        fn ensure_collection(workspace: &str) -> Result<StoredIssueCollection>;
        fn reset_discovery_cursor(workspace: &str, expected: IssueCollectionFence) -> Result<IssueCollectionApplyResult>;
        fn read_resync_batch(workspace: &str, expected: IssueCollectionFence, limit: u16) -> Result<Option<IssueResyncBatch>>;
        fn apply_resync_batch(workspace: &str, batch: &IssueResyncBatch, reads: &[IssueSourceRead]) -> Result<IssueCollectionApplyResult>;
        fn query_collection_issues(workspace: &str, query: &IssueCollectionQuery) -> Result<IssueCollectionPage>;
        fn scan_collection_issues(workspace: &str, binding_id: Option<&IssueAutomationBindingId>, visitor: &mut dyn FnMut(IssueCollectionItem) -> Result<()>) -> Result<u64>;
        fn read_triage_batch(workspace: &str, after: Option<&IssueReference>, limit: u16) -> Result<IssueTriageBatch>;
        fn save_triage(workspace: &str, expected: IssueTriageFence, assessment: &IssueTriageAssessment) -> Result<IssueTriageSaveResult>;
        fn get_issue_decision(workspace: &str, id: &IssueDecisionId) -> Result<Option<StoredIssueDecision>>;
        fn get_issue_decision_commit(workspace: &str, id: &IssueDecisionId) -> Result<Option<IssueDecisionCommit>>;
        fn ensure_issue_decision(workspace: &str, expected: IssueTriageFence, current: &IssueTriageAssessment, request: &IssueDecisionRequest) -> Result<IssueDecisionSaveResult>;
        fn save_issue_decision(workspace: &str, expected: IssueDecisionFence, current: &IssueTriageAssessment, request: &IssueDecisionRequest) -> Result<IssueDecisionSaveResult>;
    }
}

fn service_with_store(
    fixture: &Fixture,
    store: Arc<RacingStore>,
    source: Arc<FakeSource>,
    origin: Arc<FakeOrigin>,
) -> IssueCollectionService {
    IssueCollectionService::new(
        fixture.workspace.clone(),
        store,
        source,
        origin,
        IssuePolicyService::new(Arc::new(FakePolicy::default())),
    )
}

#[test]
fn mode_change_returns_its_own_commit_without_a_fresh_projection_read() {
    let fixture = Fixture::new();
    let store = RacingStore::new(&fixture, RaceAfter::Mode);
    store.inner.ensure_collection(&fixture.workspace).unwrap();
    let service = service_with_store(&fixture, store.clone(), no_source(), Arc::default());

    let result = service.set_mode(IssueCollectionMode::Off).unwrap();

    assert_eq!(result.previous, Some(IssueCollectionMode::Observe));
    assert!(result.changed);
    assert_eq!(
        result.projection.state.config.mode,
        IssueCollectionMode::Off
    );
    assert_eq!(result.projection.state.epoch, 2);
    assert_eq!(store.projection_reads.load(Ordering::SeqCst), 0);
    let latest = service.projection().unwrap();
    assert_eq!(latest.state.config.mode, IssueCollectionMode::Observe);
    assert_eq!(latest.state.epoch, 3);
    assert!(latest.catalog_revision > result.projection.catalog_revision);
    assert_eq!(store.projection_reads.load(Ordering::SeqCst), 1);
}

#[test]
fn successful_and_failed_attempts_keep_their_commit_before_a_later_off() {
    for origin_unavailable in [false, true] {
        let fixture = Fixture::new();
        let store = RacingStore::new(&fixture, RaceAfter::Attempt);
        let origin = Arc::new(FakeOrigin::default());
        origin
            .unavailable
            .store(origin_unavailable, Ordering::SeqCst);
        let source = if origin_unavailable {
            no_source()
        } else {
            FakeSource::three()
        };
        let service = service_with_store(&fixture, store.clone(), source, origin);

        let result = service
            .collect_once(&IssueCollectionCancellation::new(), 100)
            .unwrap();

        assert!(!result.discarded);
        assert_eq!(
            result.projection.state.config.mode,
            IssueCollectionMode::Observe
        );
        assert_eq!(result.projection.state.status.last_attempt_at_ms, Some(100));
        assert_eq!(
            result.projection.state.status.last_failure,
            origin_unavailable.then_some(IssueCollectionFailure::OriginUnavailable)
        );
        assert_eq!(
            result.projection.state.status.last_success_at_ms,
            (!origin_unavailable).then_some(100)
        );
        assert_eq!(store.projection_reads.load(Ordering::SeqCst), 1);
        let latest = store
            .inner
            .get_collection_projection(&fixture.workspace)
            .unwrap()
            .unwrap();
        assert_eq!(latest.state.config.mode, IssueCollectionMode::Off);
        assert_eq!(latest.state.epoch, result.projection.state.epoch + 1);
        assert!(latest.catalog_revision > result.projection.catalog_revision);
    }
}

#[test]
fn a_rejected_page_keeps_the_rejection_snapshot_before_a_later_observe() {
    let fixture = Fixture::new();
    let store = RacingStore::new(&fixture, RaceAfter::RejectedDiscovery);
    let source = FakeSource::three();
    let service = service_with_store(&fixture, store.clone(), source.clone(), Arc::default());

    let result = service
        .collect_once(&IssueCollectionCancellation::new(), 100)
        .unwrap();

    assert!(result.discarded);
    assert!(result.newly_tracked.is_empty());
    assert_eq!(
        result.projection.state.config.mode,
        IssueCollectionMode::Off
    );
    assert_eq!(result.projection.state.epoch, 2);
    assert_eq!(store.projection_reads.load(Ordering::SeqCst), 1);
    assert!(source.resyncs.lock().unwrap().is_empty());
    let latest = store
        .inner
        .get_collection_projection(&fixture.workspace)
        .unwrap()
        .unwrap();
    assert_eq!(latest.state.config.mode, IssueCollectionMode::Observe);
    assert_eq!(latest.state.epoch, 3);
    assert!(latest.catalog_revision > result.projection.catalog_revision);
}

#[test]
fn a_verified_mode_scope_is_not_retried_after_a_competing_control_commit() {
    let fixture = Fixture::new();
    let store = RacingStore::new(&fixture, RaceAfter::ConfirmedMode);
    store
        .inner
        .set_collection_mode(&fixture.workspace, None, IssueCollectionMode::Off)
        .unwrap();
    let service = service_with_store(&fixture, store.clone(), no_source(), Arc::default());
    let confirmation = mode_confirmation(&service, IssueCollectionMode::Observe);
    let before = store
        .inner
        .get_collection(&fixture.workspace)
        .unwrap()
        .unwrap();
    let error = service
        .set_mode_with_confirmation(IssueCollectionMode::Observe, Some(&confirmation))
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<IssueConfirmationError>(),
        Some(&IssueConfirmationError::ScopeMismatch)
    );
    let after = store
        .inner
        .get_collection(&fixture.workspace)
        .unwrap()
        .unwrap();
    assert_eq!(after.config.mode, IssueCollectionMode::Off);
    assert_eq!(after.epoch, before.epoch + 2);
    assert_eq!(after.revision, before.revision + 2);
    assert_eq!(store.projection_reads.load(Ordering::SeqCst), 0);
}
