//! Native 확인과 이슈/세션 권한을 가진 원문 열람·재연결 유스케이스다.
use crate::application::port::inbound::vault_restore_session_port::{
    MAX_VIEWING_SESSION, VaultRestoreReconnectRequest, VaultRestoreSessionCandidate,
    VaultRestoreSessionFailure as Failure, VaultRestoreSessionField,
    VaultRestoreSessionHold as Hold, VaultRestoreSessionMarker, VaultRestoreSessionPlace,
    VaultRestoreSessionPort, VaultRestoreSessionPrepared, VaultRestoreSessionRequest,
    VaultRestoreSessionReview,
};
use crate::application::port::outbound::evidence_vault_port::{
    VaultRecordField, VaultRestoreResponse, VaultStoreError,
};
use crate::application::port::outbound::issue_snapshot_port::{
    IssueSnapshotPort, IssueSnapshotResolveRequest,
};
use crate::application::port::outbound::vault_native_confirmation_port::{
    VaultNativeConfirmationOutcome, VaultNativeConfirmationPort, VaultNativeConfirmationPurpose,
    VaultNativeConfirmationRequest,
};
use crate::application::port::outbound::vault_restore_workspace_port::VaultRestoreWorkspacePort;
use crate::application::service::evidence_vault_restore::{
    EvidenceVaultRestoreService, VaultApprovedMapping, VaultMarkerPlace, VaultReconnectApproval,
    VaultReconnectReview, VaultRestoreHold,
};
use crate::application::service::evidence_vault_unlock::{
    EvidenceVaultUnlockService, VaultSharedCopyReason, VaultUnlockGrant, VaultUnlockOutcome,
    VaultUnlockRequest, VaultUserDecision, VaultViewingSessionId,
};
use crate::domain::issue_work::{IssueLocator, IssueProvider, IssueReference};
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

pub(crate) struct DesktopVaultRestoreDependencies {
    pub supported: bool,
    pub snapshots: Arc<dyn IssueSnapshotPort>,
    pub unlock: Arc<EvidenceVaultUnlockService>,
    pub restore: Arc<EvidenceVaultRestoreService>,
    pub confirmation: Arc<dyn VaultNativeConfirmationPort>,
    pub workspace: Arc<dyn VaultRestoreWorkspacePort>,
}

struct PreparedTarget {
    issue_url: String,
    issue: IssueReference,
}

struct SessionAccess {
    target: PreparedTarget,
    grant: VaultUnlockGrant,
}

enum State {
    Unprepared,
    Preparing,
    Prepared(PreparedTarget),
    Restoring,
    Held(SessionAccess),
    Reviewing,
    Reviewed {
        access: SessionAccess,
        review: Box<VaultReconnectReview>,
    },
    ConfirmingRewrite,
    Reconnecting,
    Committing,
    Reconnected(SessionAccess),
    Consumed,
    Cancelled,
}

enum RestoreWork {
    Initial(PreparedTarget),
    Final(SessionAccess),
}

pub(crate) struct DesktopVaultRestoreService {
    dependencies: DesktopVaultRestoreDependencies,
    cancelled: AtomicBool,
    deadline: Instant,
    session: VaultViewingSessionId,
    state: Mutex<State>,
}

impl DesktopVaultRestoreService {
    pub(crate) fn new(dependencies: DesktopVaultRestoreDependencies, lifetime: Duration) -> Self {
        let session = dependencies.unlock.begin_session();
        Self {
            dependencies,
            cancelled: AtomicBool::new(false),
            deadline: Instant::now() + lifetime.min(MAX_VIEWING_SESSION),
            session,
            state: Mutex::new(State::Unprepared),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn ensure_live(&self) -> Result<(), Hold> {
        if self.is_live() {
            Ok(())
        } else {
            self.cancel();
            Err(Hold::Locked)
        }
    }

    fn consume(&self) {
        {
            let mut state = self.state();
            if !matches!(*state, State::Cancelled) {
                *state = State::Consumed;
            }
        }
        self.dependencies.unlock.end_session(&self.session);
    }

    fn prepare_target(&self, request: VaultRestoreSessionRequest) -> Result<PreparedTarget, Hold> {
        if !self.dependencies.supported {
            return Err(Hold::PlatformUnsupported);
        }
        validate_issue(&request.issue)?;
        let locator = locator(&request.issue_url).ok_or(Hold::NotBound)?;
        self.ensure_live()?;
        let workspace = self
            .dependencies
            .workspace
            .canonical_workspace(&request.issue.project_scope.workspace_dir)
            .map_err(|_| Hold::NotBound)?;
        if workspace != request.issue.project_scope.workspace_dir {
            return Err(Hold::NotBound);
        }
        self.ensure_live()?;
        let snapshot = self
            .dependencies
            .snapshots
            .resolve(IssueSnapshotResolveRequest {
                locator,
                project_scope: request.issue.project_scope.clone(),
            })
            .map_err(|_| Hold::LatestReadFailed)?;
        if snapshot.issue != request.issue {
            return Err(Hold::NotBound);
        }
        drop(snapshot);
        let target = PreparedTarget {
            issue_url: request.issue_url,
            issue: request.issue,
        };
        self.confirm(&target, VaultNativeConfirmationPurpose::ViewOriginal)?;
        Ok(target)
    }

    fn confirm(
        &self,
        target: &PreparedTarget,
        purpose: VaultNativeConfirmationPurpose,
    ) -> Result<(), Hold> {
        self.ensure_live()?;
        let outcome = self.dependencies.confirmation.confirm(
            &VaultNativeConfirmationRequest {
                issue_url: target.issue_url.clone(),
                workspace_dir: target.issue.project_scope.workspace_dir.clone(),
                purpose,
            },
            &self.cancelled,
            self.deadline,
        );
        self.ensure_live()?;
        match outcome {
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            VaultNativeConfirmationOutcome::Confirmed => Ok(()),
            VaultNativeConfirmationOutcome::Cancelled => Err(Hold::ConfirmationCancelled),
            VaultNativeConfirmationOutcome::Unsupported => Err(Hold::PlatformUnsupported),
            VaultNativeConfirmationOutcome::Unavailable => Err(Hold::SourceUnavailable),
        }
    }

    fn take_restore_work(&self) -> Result<RestoreWork, Hold> {
        let mut state = self.state();
        match std::mem::replace(&mut *state, State::Restoring) {
            State::Prepared(target) => Ok(RestoreWork::Initial(target)),
            State::Reconnected(access) => Ok(RestoreWork::Final(access)),
            other => {
                *state = other;
                Err(Hold::Locked)
            }
        }
    }

    fn unlock_target(&self, target: PreparedTarget) -> Result<SessionAccess, Hold> {
        self.ensure_live()?;
        // Prepared는 이 서비스의 첫 native 확인 성공으로만 만들어진다.
        let grant = match self.dependencies.unlock.unlock(VaultUnlockRequest {
            issue: target.issue.clone(),
            session: self.session.clone(),
            decision: VaultUserDecision::Confirmed,
        }) {
            VaultUnlockOutcome::Unlocked(grant) => grant,
            VaultUnlockOutcome::SharedCopyOnly(_) => return Err(Hold::Locked),
        };
        self.ensure_live()?;
        Ok(SessionAccess { target, grant })
    }

    fn finish_restore(
        &self,
        access: SessionAccess,
        allow_reconnect: bool,
        restored: Result<VaultRestoreResponse, VaultRestoreHold>,
    ) -> Result<VaultRestoreResponse, Failure> {
        self.ensure_live().map_err(Failure::terminal)?;
        let mut state = self.state();
        if !matches!(*state, State::Restoring) || !self.is_live() {
            drop(state);
            self.cancel();
            return Err(Failure::terminal(Hold::Locked));
        }
        match restored {
            Err(reason) if allow_reconnect && reconnectable(&reason) => {
                *state = State::Held(access);
                Err(Failure {
                    reason: restore_hold(reason),
                    can_reconnect: true,
                })
            }
            result => {
                *state = State::Consumed;
                drop(state);
                self.dependencies.unlock.end_session(&self.session);
                result.map_err(|reason| Failure::terminal(restore_hold(reason)))
            }
        }
    }

    /// 최종 grant 재검사 뒤, 첫 저장 바로 전의 단회 진입이다. cancel과 같은 mutex로 순서를 정한다.
    fn admit_commit(&self) -> Result<(), VaultRestoreHold> {
        let admitted = {
            let mut state = self.state();
            if matches!(*state, State::Reconnecting) && self.is_live() {
                *state = State::Committing;
                true
            } else {
                false
            }
        };
        if admitted {
            Ok(())
        } else {
            self.cancel();
            Err(VaultRestoreHold::Locked(
                VaultSharedCopyReason::SessionExpired,
            ))
        }
    }

    fn reconnect_review(
        &self,
        access: SessionAccess,
        review: VaultReconnectReview,
        request: VaultRestoreReconnectRequest,
    ) -> Result<(), Hold> {
        if request.reviewed_hash != review_hash(&review) {
            return Err(Hold::ChangedDuringReview);
        }
        let mappings = request
            .mappings
            .into_iter()
            .map(|mapping| VaultApprovedMapping {
                marker: mapping.marker,
                place: VaultMarkerPlace {
                    field: match mapping.place.field {
                        VaultRestoreSessionField::Title => VaultRecordField::Title,
                        VaultRestoreSessionField::Body => VaultRecordField::Body,
                    },
                    position: mapping.place.position,
                },
            })
            .collect();
        let approval =
            VaultReconnectApproval::from_review(&review, mappings).map_err(restore_hold)?;
        drop(review);
        self.confirm(
            &access.target,
            VaultNativeConfirmationPurpose::RewriteBinding,
        )?;
        {
            let mut state = self.state();
            if !matches!(*state, State::ConfirmingRewrite) || !self.is_live() {
                drop(state);
                self.cancel();
                return Err(Hold::Locked);
            }
            *state = State::Reconnecting;
        }
        self.dependencies
            .restore
            .reconnect_with_commit_gate(&access.grant, &approval, || self.admit_commit())
            .map_err(restore_hold)?;
        self.ensure_live()?;
        let mut state = self.state();
        if !matches!(*state, State::Committing) || !self.is_live() {
            drop(state);
            self.cancel();
            return Err(Hold::Locked);
        }
        *state = State::Reconnected(access);
        Ok(())
    }
}

impl VaultRestoreSessionPort for DesktopVaultRestoreService {
    fn prepare(
        &self,
        request: VaultRestoreSessionRequest,
    ) -> Result<VaultRestoreSessionPrepared, Hold> {
        self.ensure_live()?;
        {
            let mut state = self.state();
            if !matches!(*state, State::Unprepared) {
                return Err(Hold::Locked);
            }
            *state = State::Preparing;
        }
        // 외부 I/O 중에는 state mutex를 잡지 않는다. cancel은 즉시 세션을 무효화할 수 있다.
        let prepared = self.prepare_target(request);
        let mut state = self.state();
        if !matches!(*state, State::Preparing) || !self.is_live() {
            drop(state);
            self.cancel();
            return Err(Hold::Locked);
        }
        match prepared {
            Ok(target) => {
                *state = State::Prepared(target);
                Ok(VaultRestoreSessionPrepared { reconnect: true })
            }
            Err(reason) => {
                *state = State::Consumed;
                drop(state);
                self.dependencies.unlock.end_session(&self.session);
                Err(reason)
            }
        }
    }

    fn restore(&self) -> Result<VaultRestoreResponse, Failure> {
        self.ensure_live().map_err(Failure::terminal)?;
        let work = self.take_restore_work().map_err(Failure::terminal)?;
        let (access, allow_reconnect) = match work {
            RestoreWork::Initial(target) => match self.unlock_target(target) {
                Ok(access) => (access, true),
                Err(reason) => {
                    self.consume();
                    return Err(Failure::terminal(reason));
                }
            },
            RestoreWork::Final(access) => (access, false),
        };
        let restored = self.dependencies.restore.restore_for_grant(&access.grant);
        self.finish_restore(access, allow_reconnect, restored)
    }

    fn review(&self) -> Result<VaultRestoreSessionReview, Hold> {
        self.ensure_live()?;
        let access = {
            let mut state = self.state();
            match std::mem::replace(&mut *state, State::Reviewing) {
                State::Held(access) => access,
                other => {
                    *state = other;
                    return Err(Hold::Locked);
                }
            }
        };
        let reviewed = self.dependencies.restore.review_reconnect(&access.grant);
        self.ensure_live()?;
        let review = match reviewed {
            Ok(review) => review,
            Err(reason) => {
                self.consume();
                return Err(restore_hold(reason));
            }
        };
        let projection = project_review(&review);
        let mut state = self.state();
        if !matches!(*state, State::Reviewing) || !self.is_live() {
            drop(state);
            self.cancel();
            return Err(Hold::Locked);
        }
        *state = State::Reviewed {
            access,
            review: Box::new(review),
        };
        Ok(projection)
    }

    fn reconnect(&self, request: VaultRestoreReconnectRequest) -> Result<(), Hold> {
        self.ensure_live()?;
        let (access, review) = {
            let mut state = self.state();
            match std::mem::replace(&mut *state, State::ConfirmingRewrite) {
                State::Reviewed { access, review } => (access, *review),
                other => {
                    *state = other;
                    return Err(Hold::Locked);
                }
            }
        };
        let result = self.reconnect_review(access, review, request);
        if result.is_err() {
            self.consume();
        }
        result
    }

    fn cancel(&self) {
        {
            let mut state = self.state();
            // gate와 취소의 승자는 이 짧은 구간에서 결정한다. 이미 시작한 commit을 되돌리지 않는다.
            self.cancelled.store(true, Ordering::SeqCst);
            *state = State::Cancelled;
        }
        self.dependencies.unlock.end_session(&self.session);
    }

    fn is_live(&self) -> bool {
        !self.cancelled.load(Ordering::SeqCst) && Instant::now() < self.deadline
    }

    fn deadline(&self) -> Instant {
        self.deadline
    }
}

impl Drop for DesktopVaultRestoreService {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn review_hash(review: &VaultReconnectReview) -> String {
    format!(
        "{}:{}",
        review.reviewed_hash.version, review.reviewed_hash.sha256
    )
}

fn project_place(place: &VaultMarkerPlace) -> VaultRestoreSessionPlace {
    VaultRestoreSessionPlace {
        field: match place.field {
            VaultRecordField::Title => VaultRestoreSessionField::Title,
            VaultRecordField::Body => VaultRestoreSessionField::Body,
        },
        position: place.position,
    }
}

fn project_review(review: &VaultReconnectReview) -> VaultRestoreSessionReview {
    VaultRestoreSessionReview {
        reviewed_hash: review_hash(review),
        published: review.published.clone(),
        markers: review
            .markers
            .iter()
            .map(|marker| VaultRestoreSessionMarker {
                marker: marker.marker.clone(),
                previous: project_place(&marker.previous),
                candidates: marker
                    .candidates
                    .iter()
                    .map(|place| VaultRestoreSessionCandidate {
                        place: project_place(place),
                        context: review.context(place).to_string(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn reconnectable(reason: &VaultRestoreHold) -> bool {
    matches!(
        reason,
        VaultRestoreHold::PublishedCopyChanged
            | VaultRestoreHold::MarkerMoved
            | VaultRestoreHold::MarkerDuplicated
            | VaultRestoreHold::MarkerMissing
    )
}

fn validate_issue(issue: &IssueReference) -> Result<(), Hold> {
    let node_id = |value: &str| {
        !value.is_empty()
            && value.len() <= 200
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-=".contains(&byte))
    };
    let workspace = &issue.project_scope.workspace_dir;
    let absolute = workspace.starts_with('/')
        || (workspace.len() >= 3
            && workspace.as_bytes()[0].is_ascii_alphabetic()
            && workspace.as_bytes()[1] == b':'
            && b"/\\".contains(&workspace.as_bytes()[2]));
    if issue.provider != IssueProvider::GitHub
        || issue.server_id != "github.com"
        || !node_id(&issue.repository_id)
        || !node_id(&issue.issue_id)
        || !absolute
        || workspace.len() > 4096
        || workspace.trim() != workspace
        || workspace.chars().any(char::is_control)
    {
        return Err(Hold::NotBound);
    }
    Ok(())
}
fn locator(url: &str) -> Option<IssueLocator> {
    let suffix = url.strip_prefix("https://github.com/")?;
    let segments = suffix.split('/').collect::<Vec<_>>();
    if segments.len() != 4 || segments[2] != "issues" {
        return None;
    }
    let valid_name = |name: &str| {
        !name.is_empty()
            && name.len() <= 100
            && name != "."
            && name != ".."
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    };
    if !valid_name(segments[0])
        || !valid_name(segments[1])
        || segments[3].starts_with('0')
        || !segments[3].bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let number = segments[3].parse::<u64>().ok()?;
    if number == 0 || number > i64::MAX as u64 {
        return None;
    }
    Some(IssueLocator {
        owner: segments[0].to_owned(),
        repository: segments[1].to_owned(),
        number,
    })
}
fn restore_hold(reason: VaultRestoreHold) -> Hold {
    match reason {
        VaultRestoreHold::Locked(_) => Hold::Locked,
        VaultRestoreHold::LatestReadFailed => Hold::LatestReadFailed,
        VaultRestoreHold::NotBound => Hold::NotBound,
        VaultRestoreHold::PublishedCopyChanged => Hold::PublishedCopyChanged,
        VaultRestoreHold::MarkerMoved => Hold::MarkerMoved,
        VaultRestoreHold::MarkerDuplicated => Hold::MarkerDuplicated,
        VaultRestoreHold::MarkerMissing => Hold::MarkerMissing,
        VaultRestoreHold::RecordUnavailable(VaultStoreError::WriteOutcomeUnknown) => {
            Hold::WriteOutcomeUnknown
        }
        VaultRestoreHold::RecordUnavailable(_) => Hold::RecordUnavailable,
        VaultRestoreHold::ChangedDuringReview => Hold::ChangedDuringReview,
        VaultRestoreHold::MappingRejected => Hold::MappingRejected,
    }
}
