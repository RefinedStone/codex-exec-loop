//! 한 번의 관찰 수집을 수행한다. 시계, worker 수명과 다음 실행 시각은 런타임 호스트가 소유한다.
//! 출처 호출은 저장 트랜잭션 밖에서 수행하고 각 저장은 mode/epoch/revision을 다시 검사한다.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow, bail};

use crate::application::port::inbound::stored_command_port::issue_confirmation::{
    IssueConfirmationDisplay, IssueConfirmationError, IssueConfirmationPreview,
    IssueConfirmationTarget, VerifiedHumanConfirmation,
};
use crate::application::port::outbound::issue_automation_store_port::{
    IssueAutomationStorePort, IssueCollectionApplyResult, IssueCollectionModeSaveResult,
};
use crate::application::port::outbound::issue_collection_origin_port::{
    IssueCollectionOrigin, IssueCollectionOriginPort,
};
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryRequest, IssueDiscoveryTarget, IssueResyncRequest, IssueSourcePort,
};
use crate::application::service::issue_policy::{IssuePolicyService, issue_policy_version};
use crate::domain::issue_automation::IssueAutomationBindingId;
use crate::domain::issue_automation::collection::{
    IssueCollectionFailure, IssueCollectionMode, IssueCollectionPage, IssueCollectionProjection,
    IssueCollectionQuery, StoredIssueCollection,
};
use crate::domain::issue_automation::source::{
    IssueDiscoverySelection, IssueSourceContent, IssueSourceDisclosure, IssueSourceFailure,
    IssueSourceRead, IssueSourceRepository,
};
use crate::domain::issue_policy::IssueDisclosureTarget;
use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};

pub const ISSUE_DISCOVERY_PAGE_SIZE: u16 = 25;
pub const ISSUE_RESYNC_BATCH_SIZE: u16 = 10;
const MODE_CAS_ATTEMPTS: usize = 3;

/// 서로 독립적인 저장 장부를 조립하는 동안 사실이 바뀌었다. 조회 전체만 한 번 다시 읽는다.
#[derive(Debug)]
pub(super) struct IssueDetailSnapshotChanged;

impl std::fmt::Display for IssueDetailSnapshotChanged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the issue detail changed during the read")
    }
}

impl std::error::Error for IssueDetailSnapshotChanged {}

/// 취소와 짧은 장부 commit을 선형화한다. 원격 호출 동안에는 commit_gate를 잡지 않는다.
/// cancel이 먼저 끝나면 이후 commit은 없고, commit이 먼저 시작됐으면 cancel은 그 저장만 기다린다.
#[derive(Default)]
pub struct IssueCollectionCancellation {
    cancelled: AtomicBool,
    commit_gate: Mutex<()>,
}

impl IssueCollectionCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        let _guard = self
            .commit_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.cancelled.store(true, Ordering::Release);
    }

    pub(crate) fn commit_if_active<T>(
        &self,
        commit: impl FnOnce() -> Result<T>,
    ) -> Result<Option<T>> {
        let _guard = self
            .commit_gate
            .lock()
            .map_err(|_| anyhow!("the issue collection commit gate is unavailable"))?;
        if self.is_cancelled() {
            return Ok(None);
        }
        commit().map(Some)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCollectionModeChange {
    pub previous: Option<IssueCollectionMode>,
    pub projection: IssueCollectionProjection,
    pub changed: bool,
}

/// 해당 트랜잭션에서 이미 확정한 등록 사실이다. 후속 수집 취소·실패와 별도로 전달한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCollectionTrackedFact {
    pub issue: IssueReference,
    pub binding_id: IssueAutomationBindingId,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCollectionRunOutcome {
    pub projection: IssueCollectionProjection,
    pub newly_tracked: Vec<IssueReference>,
    pub discarded: bool,
}

struct CollectionProgress {
    projection: IssueCollectionProjection,
    newly_tracked: Vec<IssueReference>,
    discarded: bool,
}

pub struct IssueCollectionService {
    workspace_dir: String,
    store: Arc<dyn IssueAutomationStorePort>,
    source: Arc<dyn IssueSourcePort>,
    origin: Arc<dyn IssueCollectionOriginPort>,
    policy: IssuePolicyService,
    publication_journal: Option<Arc<dyn IssuePublishIntentJournalPort>>,
    triage: Option<Arc<super::issue_triage::IssueTriageService>>,
    coordinator: Option<Arc<super::issue_coordinator::IssueCoordinatorService>>,
    autonomy: Option<Arc<super::issue_autonomy::IssueAutonomyService>>,
    interactions: Option<Arc<super::issue_interactions::IssueInteractionsService>>,
}

impl IssueCollectionService {
    pub fn new(
        workspace_dir: String,
        store: Arc<dyn IssueAutomationStorePort>,
        source: Arc<dyn IssueSourcePort>,
        origin: Arc<dyn IssueCollectionOriginPort>,
        policy: IssuePolicyService,
    ) -> Self {
        Self {
            workspace_dir,
            store,
            source,
            origin,
            policy,
            publication_journal: None,
            triage: None,
            coordinator: None,
            autonomy: None,
            interactions: None,
        }
    }

    /// 원격 읽기부터 관찰 저장까지 피드백 쓰기와 같은 lease로 직렬화한다.
    pub fn with_publication_journal(
        mut self,
        journal: Arc<dyn IssuePublishIntentJournalPort>,
    ) -> Self {
        self.publication_journal = Some(journal);
        self
    }

    pub fn with_autonomy(
        mut self,
        autonomy: Arc<super::issue_autonomy::IssueAutonomyService>,
    ) -> Self {
        self.autonomy = Some(autonomy);
        self
    }

    pub fn autonomy_service(&self) -> Option<Arc<super::issue_autonomy::IssueAutonomyService>> {
        self.autonomy.clone()
    }

    pub fn autonomy_state(
        &self,
    ) -> crate::domain::issue_automation::autonomy::IssueAutonomyProjection {
        if let Some(autonomy) = &self.autonomy {
            return autonomy.state();
        }
        self.triage
            .as_ref()
            .map(|triage| triage.autonomy_projection())
            .unwrap_or(
                crate::domain::issue_automation::autonomy::IssueAutonomyProjection::Unavailable,
            )
    }

    pub fn with_interactions(
        mut self,
        interactions: Arc<super::issue_interactions::IssueInteractionsService>,
    ) -> Self {
        self.interactions = Some(interactions);
        self
    }

    pub fn with_coordinator(
        mut self,
        coordinator: Arc<super::issue_coordinator::IssueCoordinatorService>,
    ) -> Self {
        self.coordinator = Some(coordinator);
        self
    }

    pub fn coordinator_service(
        &self,
    ) -> Option<Arc<super::issue_coordinator::IssueCoordinatorService>> {
        self.coordinator.clone()
    }

    pub fn with_triage(mut self, triage: Arc<super::issue_triage::IssueTriageService>) -> Self {
        self.triage = Some(triage);
        self
    }

    pub fn triage_service(&self) -> Option<Arc<super::issue_triage::IssueTriageService>> {
        self.triage.clone()
    }

    /// 조회로 기본 Observe를 저장하거나 수집을 시작하지 않는다.
    pub fn policy_inspection(
        &self,
    ) -> Option<super::issue_automation_policy::IssueAutomationPolicyInspection> {
        self.triage
            .as_ref()
            .map(|triage| triage.policy_inspection())
    }

    pub fn issue(
        &self,
        reference: &IssueReference,
    ) -> Result<crate::domain::issue_automation::collection::IssueCollectionDetail> {
        match self.read_issue_detail(reference) {
            Err(error) if error.is::<IssueDetailSnapshotChanged>() => {
                self.read_issue_detail(reference)
            }
            result => result,
        }
    }

    fn read_issue_detail(
        &self,
        reference: &IssueReference,
    ) -> Result<crate::domain::issue_automation::collection::IssueCollectionDetail> {
        use crate::domain::issue_automation::execution::{
            IssueExecutionDetail, IssueExecutionUnavailableReason,
        };

        let triage = self
            .triage
            .as_ref()
            .ok_or_else(|| anyhow!("the issue triage service is unavailable"))?;
        let mut detail = triage.issue(reference)?;
        if let Some(item) = &detail.item {
            detail.execution = match self
                .store
                .get_issue_execution(&self.workspace_dir, reference)
            {
                Ok(Some(record)) => Some(IssueExecutionDetail::from_record(
                    &item.issue,
                    item.work.as_ref(),
                    &record,
                    triage.execution_attempt_limit().ok(),
                )),
                Ok(None) => None,
                Err(_) => Some(IssueExecutionDetail::Unavailable {
                    reason: IssueExecutionUnavailableReason::ReadFailed,
                }),
            };
        }
        if let (Some(interactions), Some(item)) = (&self.interactions, &detail.item) {
            detail.interaction = Some(interactions.project(item)?);
        }
        if let (Some(coordinator), Some(_)) = (&self.coordinator, &detail.item) {
            detail.feedback = coordinator.feedback_detail(reference)?;
        }
        // Execution/work/collection changes share this catalog revision. Feedback has its own
        // fence in feedback_detail; its counter must never be compared with the catalog.
        let after = self.store.read_triage_batch(&self.workspace_dir, None, 1)?;
        if after.revision != detail.revision {
            return Err(IssueDetailSnapshotChanged.into());
        }
        Ok(detail)
    }

    pub fn report(
        &self,
        binding_id: Option<&IssueAutomationBindingId>,
    ) -> Result<crate::domain::issue_automation::collection::IssueTriageObserveReport> {
        self.triage
            .as_ref()
            .ok_or_else(|| anyhow!("the issue triage service is unavailable"))?
            .report(binding_id)
    }

    /// 기본 Observe도 영속된 뒤에 반환한다. 기존 Off나 읽기 오류를 기본값으로 대체하지 않는다.
    pub fn projection(&self) -> Result<IssueCollectionProjection> {
        self.store
            .ensure_collection(&self.workspace_dir)
            .map_err(ledger_unavailable)?;
        self.project()
    }

    pub fn issues(&self, query: &IssueCollectionQuery) -> Result<IssueCollectionPage> {
        query.validate()?;
        if let Some(triage) = &self.triage {
            return triage.issues(query);
        }
        self.store
            .query_collection_issues(&self.workspace_dir, query)
            .map_err(ledger_unavailable)
    }

    /// 준비 조회는 기본 Observe를 저장하지 않는다. 실제 이전 mode/epoch/revision만 묶는다.
    pub fn prepare_mode_confirmation(
        &self,
        mode: IssueCollectionMode,
    ) -> Result<IssueConfirmationPreview> {
        let current = self
            .store
            .get_collection(&self.workspace_dir)
            .map_err(ledger_unavailable)?;
        let previous = current.as_ref().map(|state| state.config.mode);
        let (message, confirm_label) = match mode {
            IssueCollectionMode::Observe => ("이슈 관찰을 켤까요?", "관찰 켜기"),
            IssueCollectionMode::Off => ("이슈 관찰을 끌까요?", "관찰 끄기"),
        };
        Ok(IssueConfirmationPreview {
            target: mode_confirmation_target(&self.workspace_dir, current.as_ref(), mode),
            display: IssueConfirmationDisplay {
                message: message.to_owned(),
                detail: format!(
                    "현재: {}\n변경 후: {}",
                    mode_label(previous),
                    mode_label(Some(mode))
                ),
                confirm_label: confirm_label.to_owned(),
            },
        })
    }

    /// 증거 없는 호출도 반드시 같은 인가 경계를 지난다. 하향과 no-op은 확인이 필요 없다.
    pub fn set_mode(&self, mode: IssueCollectionMode) -> Result<IssueCollectionModeChange> {
        self.set_mode_with_confirmation(mode, None)
    }

    pub fn set_mode_with_confirmation(
        &self,
        mode: IssueCollectionMode,
        confirmation: Option<&VerifiedHumanConfirmation>,
    ) -> Result<IssueCollectionModeChange> {
        for _ in 0..MODE_CAS_ATTEMPTS {
            let current = self
                .store
                .get_collection(&self.workspace_dir)
                .map_err(ledger_unavailable)?;
            let previous = current.as_ref().map(|state| state.config.mode);
            let target = mode_confirmation_target(&self.workspace_dir, current.as_ref(), mode);
            if let Some(confirmation) = confirmation {
                confirmation.require_target(&target)?;
            }
            // 미설정의 기본값은 Observe이다. 명시적 Off를 다시 켜는 명령은 상향이다.
            let raises =
                previous == Some(IssueCollectionMode::Off) && mode == IssueCollectionMode::Observe;
            let result = if raises {
                confirmation.ok_or(IssueConfirmationError::Required)?;
                let state = current
                    .as_ref()
                    .ok_or(IssueConfirmationError::ScopeMismatch)?;
                self.store
                    .set_collection_mode_exact(
                        &self.workspace_dir,
                        state.fence(),
                        state.config.mode,
                        mode,
                    )
                    .map_err(ledger_unavailable)?
            } else {
                self.store
                    .set_collection_mode(
                        &self.workspace_dir,
                        current.as_ref().map(|state| state.epoch),
                        mode,
                    )
                    .map_err(ledger_unavailable)?
            };
            match result {
                IssueCollectionModeSaveResult::Saved {
                    projection,
                    changed,
                    ..
                } => {
                    return Ok(IssueCollectionModeChange {
                        previous,
                        projection: *projection,
                        changed,
                    });
                }
                IssueCollectionModeSaveResult::Conflict { .. } if raises => {
                    // 사람이 본 mode 판이 바뀌었다. 같은 증거를 새 판으로 옮겨 재시도하지 않는다.
                    return Err(IssueConfirmationError::ScopeMismatch.into());
                }
                IssueCollectionModeSaveResult::Conflict { .. } => continue,
            }
        }
        bail!("the issue collection configuration changed concurrently")
    }

    /// 기존 fixture가 실제 검증 완료 분기를 거치게 하는 시험 전용 준비다.
    #[cfg(test)]
    pub(crate) fn set_mode_for_test(
        &self,
        mode: IssueCollectionMode,
    ) -> Result<IssueCollectionModeChange> {
        let preview = self.prepare_mode_confirmation(mode)?;
        let confirmation = VerifiedHumanConfirmation::from_verified(
            preview.target,
            format!("desktop-confirmation:{}", "a".repeat(32)),
        )?;
        self.set_mode_with_confirmation(mode, Some(&confirmation))
    }

    /// 원격 한 페이지와 유한한 재동기화 묶음만 수행한다. 실제 API 호출은 최대 1 + 10회다.
    pub fn collect_once(
        &self,
        cancel: &IssueCollectionCancellation,
        at_unix_ms: u64,
    ) -> Result<IssueCollectionRunOutcome> {
        self.collect_once_with_committed(cancel, at_unix_ms, &|_| {})
    }

    /// callback은 commit gate 안에서 실행한다. 빠른 mailbox 전송만 수행하며
    /// 취소나 서비스 호출을 재진입하지 않는다. cancel이 돌아오면 모든 확정 사실이 이미 전달됐다.
    pub fn collect_once_with_committed(
        &self,
        cancel: &IssueCollectionCancellation,
        at_unix_ms: u64,
        committed: &(dyn Fn(Vec<IssueCollectionTrackedFact>) + Send + Sync),
    ) -> Result<IssueCollectionRunOutcome> {
        let projection = cancel
            .commit_if_active(|| self.projection())?
            .ok_or_else(|| anyhow!("the issue collection was cancelled"))?;
        let mut progress = CollectionProgress {
            projection,
            newly_tracked: Vec::new(),
            discarded: false,
        };
        if !self.may_read(&progress.projection.state, cancel)? {
            return self.discard(progress);
        }
        let origin = match self.origin.github_origin(&self.workspace_dir) {
            Ok(origin) => origin,
            Err(_) => {
                return self.finish(
                    progress,
                    cancel,
                    at_unix_ms,
                    Some(IssueCollectionFailure::OriginUnavailable),
                );
            }
        };
        let policy = match self.policy.resolve(&self.workspace_dir) {
            Ok(policy) => policy,
            Err(_) => {
                return self.finish(
                    progress,
                    cancel,
                    at_unix_ms,
                    Some(IssueCollectionFailure::PolicyUnavailable),
                );
            }
        };
        let disclosure = IssueSourceDisclosure {
            policy: *policy.disclosure_for(IssueDisclosureTarget::AiHandoff),
            policy_version: issue_policy_version(&policy),
        };
        let capabilities = self.source.capabilities();
        if !capabilities.discovery || !capabilities.resync {
            return self.finish(
                progress,
                cancel,
                at_unix_ms,
                Some(IssueCollectionFailure::Source(
                    IssueSourceFailure::UnsupportedProvider,
                )),
            );
        }

        let repository =
            progress
                .projection
                .binding
                .as_ref()
                .map(|binding| IssueSourceRepository {
                    provider: binding.provider(),
                    server_id: binding.server_id().to_string(),
                    repository_id: binding.repository_id().to_string(),
                    project_scope: binding.project_scope().clone(),
                });
        if repository.as_ref().is_some_and(|repository| {
            repository.provider != IssueProvider::GitHub
                || repository.server_id != "github.com"
                || repository.project_scope.workspace_dir != self.workspace_dir
        }) {
            return self.finish(
                progress,
                cancel,
                at_unix_ms,
                Some(IssueCollectionFailure::Source(
                    IssueSourceFailure::IdentityMismatch,
                )),
            );
        }
        let selection = IssueDiscoverySelection::AllOpen;
        if let (Some(cursor), Some(repository)) =
            (&progress.projection.state.discovery_cursor, &repository)
        {
            // 바인딩의 안정 좌표가 다르면 정책 변경으로 간주하여 우회하지 않는다.
            if cursor.to_record().repository != *repository {
                return self.finish(
                    progress,
                    cancel,
                    at_unix_ms,
                    Some(IssueCollectionFailure::Source(
                        IssueSourceFailure::IdentityMismatch,
                    )),
                );
            }
            if cursor
                .validate_for(
                    repository,
                    &selection,
                    ISSUE_DISCOVERY_PAGE_SIZE,
                    &disclosure,
                )
                .is_err()
            {
                let result = cancel.commit_if_active(|| {
                    self.store
                        .reset_discovery_cursor(
                            &self.workspace_dir,
                            progress.projection.state.fence(),
                        )
                        .map_err(ledger_unavailable)
                })?;
                if !adopt(&mut progress, result) {
                    return self.discard(progress);
                }
            }
        }
        let request = self.discovery_request(
            &origin,
            repository,
            &progress.projection.state,
            selection,
            disclosure.clone(),
        );
        if let Err(reason) = request.validate() {
            return self.finish(
                progress,
                cancel,
                at_unix_ms,
                Some(IssueCollectionFailure::Source(reason)),
            );
        }
        if !self.may_read(&progress.projection.state, cancel)? {
            return self.discard(progress);
        }
        // 이 lease는 SQL transaction이 아니다. 원격 읽기와 저장 사이의 자기 라벨 변경을 막는다.
        let publication_lease = self
            .publication_journal
            .as_ref()
            .map(|journal| journal.acquire_publication_lease(&self.workspace_dir))
            .transpose()?;
        if let Some(lease) = &publication_lease {
            lease.verify_identity()?;
        }
        let mut failure = None;
        match self.source.discover(request.clone()) {
            Ok(page) => {
                let result = cancel.commit_if_active(|| {
                    if let Some(lease) = &publication_lease {
                        lease.verify_identity()?;
                    }
                    let result = self
                        .store
                        .apply_discovery_page(
                            &self.workspace_dir,
                            progress.projection.state.fence(),
                            &request,
                            &page,
                        )
                        .map_err(ledger_unavailable)?;
                    report_committed(&result, committed)?;
                    Ok(result)
                })?;
                if !adopt(&mut progress, result) {
                    return self.discard(progress);
                }
            }
            Err(reason) => failure = Some(IssueCollectionFailure::Source(reason)),
        }

        // 발견에 실패해도 이미 알고 있는 ID는 별도로 읽는다. 목록 실패를 기존 이슈의 변화 없음으로 보지 않는다.
        if !self.may_read(&progress.projection.state, cancel)? {
            return self.discard(progress);
        }
        let Some(batch) = self
            .store
            .read_resync_batch(
                &self.workspace_dir,
                progress.projection.state.fence(),
                ISSUE_RESYNC_BATCH_SIZE,
            )
            .map_err(ledger_unavailable)?
        else {
            return self.discard(progress);
        };
        let mut reads = Vec::with_capacity(batch.targets.len());
        for reference in &batch.targets {
            if !self.may_read(&progress.projection.state, cancel)? {
                return self.discard(progress);
            }
            if let Some(lease) = &publication_lease {
                lease.verify_identity()?;
            }
            let read = self.source.resync(IssueResyncRequest {
                issue: reference.clone(),
                disclosure: disclosure.clone(),
            });
            let read = checked_resync_read(reference, &disclosure.policy_version, read);
            if let IssueSourceRead::Unknown { reason, .. } = &read {
                failure.get_or_insert(IssueCollectionFailure::Source(*reason));
            }
            reads.push(read);
        }
        let result = cancel.commit_if_active(|| {
            if let Some(lease) = &publication_lease {
                lease.verify_identity()?;
            }
            let result = self
                .store
                .apply_resync_batch(&self.workspace_dir, &batch, &reads)
                .map_err(ledger_unavailable)?;
            report_committed(&result, committed)?;
            Ok(result)
        })?;
        if !adopt(&mut progress, result) {
            return self.discard(progress);
        }
        self.finish(progress, cancel, at_unix_ms, failure)
    }

    fn discovery_request(
        &self,
        origin: &IssueCollectionOrigin,
        repository: Option<IssueSourceRepository>,
        state: &StoredIssueCollection,
        selection: IssueDiscoverySelection,
        disclosure: IssueSourceDisclosure,
    ) -> IssueDiscoveryRequest {
        // 새 순회의 첫 페이지는 origin이 여전히 같은 저장소인지 확인한다.
        // 후속 페이지는 이름에 의존하지 않고 확인한 저장소 ID에 연결한다.
        let target = match (state.discovery_cursor.as_ref(), repository) {
            (Some(_), Some(repository)) => IssueDiscoveryTarget::Repository(repository),
            (_, repository) => IssueDiscoveryTarget::GitHubLocator {
                owner: origin.owner.clone(),
                name: origin.name.clone(),
                project_scope: IssueProjectScope {
                    workspace_dir: self.workspace_dir.clone(),
                },
                expected_repository_id: repository.map(|repository| repository.repository_id),
            },
        };
        IssueDiscoveryRequest {
            target,
            selection,
            cursor: state.discovery_cursor.clone(),
            page_size: ISSUE_DISCOVERY_PAGE_SIZE,
            disclosure,
        }
    }

    fn may_read(
        &self,
        expected: &StoredIssueCollection,
        cancel: &IssueCollectionCancellation,
    ) -> Result<bool> {
        if cancel.is_cancelled() || expected.config.mode == IssueCollectionMode::Off {
            return Ok(false);
        }
        let current = self
            .store
            .get_collection(&self.workspace_dir)
            .map_err(ledger_unavailable)?;
        Ok(current.is_some_and(|current| {
            current.config.mode == IssueCollectionMode::Observe
                && current.fence() == expected.fence()
        }) && !cancel.is_cancelled())
    }

    fn finish(
        &self,
        mut progress: CollectionProgress,
        cancel: &IssueCollectionCancellation,
        at_unix_ms: u64,
        failure: Option<IssueCollectionFailure>,
    ) -> Result<IssueCollectionRunOutcome> {
        let result = cancel.commit_if_active(|| {
            self.store
                .record_collection_attempt(
                    &self.workspace_dir,
                    progress.projection.state.fence(),
                    at_unix_ms,
                    failure,
                )
                .map_err(ledger_unavailable)
        })?;
        if !adopt(&mut progress, result) {
            return self.discard(progress);
        }
        self.outcome(progress)
    }

    fn discard(&self, mut progress: CollectionProgress) -> Result<IssueCollectionRunOutcome> {
        progress.discarded = true;
        self.outcome(progress)
    }

    fn outcome(&self, progress: CollectionProgress) -> Result<IssueCollectionRunOutcome> {
        Ok(IssueCollectionRunOutcome {
            projection: progress.projection,
            newly_tracked: progress.newly_tracked,
            discarded: progress.discarded,
        })
    }

    fn project(&self) -> Result<IssueCollectionProjection> {
        self.store
            .get_collection_projection(&self.workspace_dir)
            .map_err(ledger_unavailable)?
            .ok_or_else(|| anyhow!("the issue collection projection is unavailable"))
    }
}

fn report_committed(
    result: &IssueCollectionApplyResult,
    report: &(dyn Fn(Vec<IssueCollectionTrackedFact>) + Send + Sync),
) -> Result<()> {
    if let IssueCollectionApplyResult::Applied(commit) = result
        && !commit.newly_tracked.is_empty()
    {
        let binding_id = commit
            .projection
            .state
            .binding_id
            .as_ref()
            .ok_or_else(|| anyhow!("the committed issue collection has no binding"))?;
        report(
            commit
                .newly_tracked
                .iter()
                .map(|issue| IssueCollectionTrackedFact {
                    issue: issue.clone(),
                    binding_id: binding_id.clone(),
                    revision: commit.projection.catalog_revision,
                })
                .collect(),
        );
    }
    Ok(())
}

pub(crate) fn checked_resync_read(
    reference: &IssueReference,
    policy_version: &str,
    read: IssueSourceRead,
) -> IssueSourceRead {
    let failure = match &read {
        IssueSourceRead::Observed(observation) if observation.issue == *reference => {
            let observed_policy = match &observation.content {
                IssueSourceContent::Shared(snapshot) => &snapshot.policy_version,
                IssueSourceContent::Withheld { policy_version, .. } => policy_version,
            };
            if observed_policy == policy_version {
                return read;
            }
            IssueSourceFailure::MalformedResponse
        }
        IssueSourceRead::Unknown { issue, .. } if issue == reference => return read,
        _ => IssueSourceFailure::IdentityMismatch,
    };
    IssueSourceRead::Unknown {
        issue: reference.clone(),
        reason: failure,
    }
}

fn adopt(progress: &mut CollectionProgress, result: Option<IssueCollectionApplyResult>) -> bool {
    match result {
        Some(IssueCollectionApplyResult::Applied(commit)) => {
            progress.projection = commit.projection;
            progress.newly_tracked.extend(commit.newly_tracked);
            true
        }
        Some(IssueCollectionApplyResult::Discarded { projection, .. }) => {
            progress.projection = *projection;
            progress.discarded = true;
            false
        }
        None => {
            progress.discarded = true;
            false
        }
    }
}

fn ledger_unavailable(_: anyhow::Error) -> anyhow::Error {
    anyhow!("the issue collection ledger is unavailable")
}

fn mode_confirmation_target(
    workspace: &str,
    current: Option<&StoredIssueCollection>,
    to: IssueCollectionMode,
) -> IssueConfirmationTarget {
    IssueConfirmationTarget::Mode {
        workspace: workspace.to_owned(),
        from: current.map(|state| state.config.mode),
        epoch: current.map(|state| state.epoch),
        revision: current.map(|state| state.revision),
        to,
    }
}

fn mode_label(mode: Option<IssueCollectionMode>) -> &'static str {
    match mode {
        Some(IssueCollectionMode::Off) => "꺼짐",
        Some(IssueCollectionMode::Observe) => "관찰",
        None => "기본 관찰",
    }
}

#[cfg(test)]
mod tests;
