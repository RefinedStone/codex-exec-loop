//! Native 확인과 이슈/세션 권한을 가진 원문 열람 유스케이스다. transport의 순서 검사를 신뢰하지 않는다.
use crate::application::port::inbound::vault_restore_session_port::{
    MAX_VIEWING_SESSION, VaultRestoreSessionHold as Hold, VaultRestoreSessionPort,
    VaultRestoreSessionRequest,
};
use crate::application::port::outbound::evidence_vault_port::VaultRestoreResponse;
use crate::application::port::outbound::issue_snapshot_port::{
    IssueSnapshotPort, IssueSnapshotResolveRequest,
};
use crate::application::port::outbound::vault_native_confirmation_port::{
    VaultNativeConfirmationOutcome, VaultNativeConfirmationPort, VaultNativeConfirmationRequest,
};
use crate::application::port::outbound::vault_restore_workspace_port::VaultRestoreWorkspacePort;
use crate::application::service::evidence_vault_restore::{
    EvidenceVaultRestoreService, VaultRestoreHold,
};
use crate::application::service::evidence_vault_unlock::{
    EvidenceVaultUnlockService, VaultUnlockOutcome, VaultUnlockRequest, VaultUserDecision,
    VaultViewingSessionId,
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
enum State {
    Unprepared,
    Preparing,
    Prepared(IssueReference),
    Consumed,
    Cancelled,
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
    fn prepare_target(&self, request: VaultRestoreSessionRequest) -> Result<IssueReference, Hold> {
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
        self.ensure_live()?;
        let outcome = self.dependencies.confirmation.confirm(
            &VaultNativeConfirmationRequest {
                issue_url: request.issue_url,
                workspace_dir: workspace,
            },
            &self.cancelled,
            self.deadline,
        );
        self.ensure_live()?;
        match outcome {
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            VaultNativeConfirmationOutcome::Confirmed => Ok(request.issue),
            VaultNativeConfirmationOutcome::Cancelled => Err(Hold::ConfirmationCancelled),
            VaultNativeConfirmationOutcome::Unsupported => Err(Hold::PlatformUnsupported),
            VaultNativeConfirmationOutcome::Unavailable => Err(Hold::SourceUnavailable),
        }
    }
}
impl VaultRestoreSessionPort for DesktopVaultRestoreService {
    fn prepare(&self, request: VaultRestoreSessionRequest) -> Result<(), Hold> {
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
            Ok(issue) => {
                *state = State::Prepared(issue);
                Ok(())
            }
            Err(reason) => {
                *state = State::Consumed;
                drop(state);
                self.dependencies.unlock.end_session(&self.session);
                Err(reason)
            }
        }
    }
    fn restore(&self) -> Result<VaultRestoreResponse, Hold> {
        self.ensure_live()?;
        let issue = {
            let mut state = self.state();
            let State::Prepared(issue) = &*state else {
                return Err(Hold::Locked);
            };
            let issue = issue.clone();
            *state = State::Consumed;
            issue
        };
        self.ensure_live()?;
        // Prepared는 이 서비스의 native 확인 성공으로만 만들어진다.
        let grant = match self.dependencies.unlock.unlock(VaultUnlockRequest {
            issue,
            session: self.session.clone(),
            decision: VaultUserDecision::Confirmed,
        }) {
            VaultUnlockOutcome::Unlocked(grant) => grant,
            VaultUnlockOutcome::SharedCopyOnly(_) => return Err(Hold::Locked),
        };
        self.ensure_live()?;
        let restored = self
            .dependencies
            .restore
            .restore_for_grant(&grant)
            .map_err(restore_hold)?;
        self.ensure_live()?;
        Ok(restored)
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.dependencies.unlock.end_session(&self.session);
        *self.state() = State::Cancelled;
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
        VaultRestoreHold::RecordUnavailable(_) => Hold::RecordUnavailable,
        VaultRestoreHold::ChangedDuringReview | VaultRestoreHold::MappingRejected => {
            Hold::SourceUnavailable
        }
    }
}
