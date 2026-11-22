//! 원문 열람 전용 inherited-pipe 입구다. 공개 요청은 OS 확인 결과를 지정할 수 없다.
mod protocol;
#[cfg(test)]
mod tests;

use crate::application::port::outbound::evidence_vault_port::VaultRestoreResponse;
use crate::application::port::outbound::issue_snapshot_port::IssueSnapshotResolveRequest;
use crate::application::port::outbound::vault_native_confirmation_port::{
    VaultNativeConfirmationOutcome, VaultNativeConfirmationRequest,
};
use crate::application::service::evidence_vault_restore::VaultRestoreHold;
use crate::application::service::evidence_vault_unlock::{
    EvidenceVaultUnlockService, VaultUnlockOutcome, VaultUnlockRequest, VaultUserDecision,
    VaultViewingSessionId,
};
use crate::composition::desktop_restore::DesktopRestoreServices;
use crate::domain::issue_work::IssueReference;
use protocol::{Frames, Hold, Reply, Request};
use std::io;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite};

const SESSION_LIFETIME: Duration = Duration::from_secs(5 * 60);
const FIRST_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

struct Lifetime {
    cancelled: AtomicBool,
    deadline: Instant,
    unlock: Arc<EvidenceVaultUnlockService>,
    session: VaultViewingSessionId,
}
impl Lifetime {
    fn live(&self) -> bool {
        !self.cancelled.load(Ordering::SeqCst) && Instant::now() < self.deadline
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.unlock.end_session(&self.session);
    }
}
struct EndSession(Arc<Lifetime>);
impl Drop for EndSession {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(super) async fn run() -> anyhow::Result<()> {
    let input = tokio::io::stdin();
    let mut output = tokio::io::stdout();
    let mut frames = Frames::new(input);
    let Some((id, url, issue)) = first_request(&mut frames).await else {
        return Ok(());
    };
    if !cfg!(any(target_os = "linux", target_os = "macos")) {
        held(&mut output, &id, Hold::PlatformUnsupported).await?;
        return Ok(());
    }
    let Ok(services) = crate::composition::desktop_restore::build() else {
        held(&mut output, &id, Hold::SourceUnavailable).await?;
        return Ok(());
    };
    session(frames, output, id, url, issue, services, SESSION_LIFETIME)
        .await
        .map_err(|_| anyhow::anyhow!("local restore channel closed"))
}

async fn first_request<R: AsyncRead + Unpin>(
    frames: &mut Frames<R>,
) -> Option<(String, String, IssueReference)> {
    let Ok(Ok(Some(Request::Prepare {
        version,
        id,
        issue_url,
        issue,
    }))) = tokio::time::timeout(FIRST_REQUEST_TIMEOUT, frames.read()).await
    else {
        return None;
    };
    if !protocol::valid_header(version, &id) {
        return None;
    }
    Some((id, issue_url, issue.into_reference()?))
}

async fn held<W: AsyncWrite + Unpin>(output: &mut W, id: &str, reason: Hold) -> io::Result<()> {
    let bytes = protocol::encode(id, Reply::Held { reason })?;
    tokio::time::timeout(FIRST_REQUEST_TIMEOUT, protocol::write(output, &bytes))
        .await
        .map_err(|_| io::Error::other("local restore channel closed"))?
}

async fn session<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut frames: Frames<R>,
    mut output: W,
    id: String,
    url: String,
    issue: IssueReference,
    services: DesktopRestoreServices,
    session_lifetime: Duration,
) -> io::Result<()> {
    if !services.supported {
        return held(&mut output, &id, Hold::PlatformUnsupported).await;
    }
    if frames.buffered_input() {
        return Ok(());
    }
    let services = Arc::new(services);
    let lifetime = Arc::new(Lifetime {
        cancelled: AtomicBool::new(false),
        deadline: Instant::now() + session_lifetime.min(SESSION_LIFETIME),
        session: services.unlock.begin_session(),
        unlock: services.unlock.clone(),
    });
    let _end = EndSession(lifetime.clone());
    let deadline = tokio::time::Instant::from_std(lifetime.deadline);
    let prepare_services = services.clone();
    let prepare_lifetime = lifetime.clone();
    let mut preparing = tokio::task::spawn_blocking(move || {
        prepare(&prepare_services, &prepare_lifetime, url, issue)
    });
    let prepared = tokio::select! {
        biased;
        _ = frames.unexpected_input() => { lifetime.cancel(); return Ok(()); }
        _ = tokio::time::sleep_until(deadline) => { lifetime.cancel(); return Ok(()); }
        result = &mut preparing => result.unwrap_or(Err(Hold::SourceUnavailable)),
    };
    if !lifetime.live() {
        return Ok(());
    }
    let issue = match prepared {
        Ok(issue) => issue,
        Err(reason) => return held(&mut output, &id, reason).await,
    };
    let bytes = protocol::encode(&id, Reply::Prepared)?;
    if !matches!(
        tokio::time::timeout_at(deadline, protocol::write(&mut output, &bytes)).await,
        Ok(Ok(()))
    ) {
        return Ok(());
    }
    let second = tokio::time::timeout_at(deadline, frames.read()).await;
    let Ok(Ok(Some(Request::Restore {
        version,
        id: restore_id,
    }))) = second
    else {
        return Ok(());
    };
    if !protocol::valid_header(version, &restore_id)
        || restore_id != id
        || frames.buffered_input()
        || !lifetime.live()
    {
        return Ok(());
    }

    let restore_services = services.clone();
    let restore_lifetime = lifetime.clone();
    let mut restoring =
        tokio::task::spawn_blocking(move || restore(&restore_services, &restore_lifetime, issue));
    let restored = tokio::select! {
        biased;
        _ = frames.unexpected_input() => { lifetime.cancel(); return Ok(()); }
        _ = tokio::time::sleep_until(deadline) => { lifetime.cancel(); return Ok(()); }
        result = &mut restoring => result.unwrap_or(Err(Hold::SourceUnavailable)),
    };
    if !lifetime.live() {
        return Ok(());
    }
    let bytes = match &restored {
        Ok(value) => protocol::encode(
            &id,
            Reply::Restored {
                title: value.title.expose(),
                body: value.body.expose(),
            },
        )?,
        Err(reason) => protocol::encode(&id, Reply::Held { reason: *reason })?,
    };
    // 직렬화가 끝난 뒤에도 EOF/추가 입력/마감을 먼저 검사한다. 평문 버퍼는 drop 시 지운다.
    tokio::select! {
        biased;
        _ = frames.unexpected_input() => { lifetime.cancel(); Ok(()) }
        _ = tokio::time::sleep_until(deadline) => { lifetime.cancel(); Ok(()) }
        result = async {
            if !lifetime.live() { return Ok(()); }
            protocol::write(&mut output, &bytes).await
        } => result,
    }
}

fn prepare(
    services: &DesktopRestoreServices,
    lifetime: &Lifetime,
    url: String,
    issue: IssueReference,
) -> Result<IssueReference, Hold> {
    if !lifetime.live() {
        return Err(Hold::Locked);
    }
    let locator = protocol::locator(&url).ok_or(Hold::NotBound)?;
    let workspace =
        (services.workspace)(&issue.project_scope.workspace_dir).map_err(|_| Hold::NotBound)?;
    if workspace != issue.project_scope.workspace_dir {
        return Err(Hold::NotBound);
    }
    if !lifetime.live() {
        return Err(Hold::Locked);
    }
    let snapshot = services
        .snapshots
        .resolve(IssueSnapshotResolveRequest {
            locator,
            project_scope: issue.project_scope.clone(),
        })
        .map_err(|_| Hold::LatestReadFailed)?;
    if snapshot.issue != issue {
        return Err(Hold::NotBound);
    }
    drop(snapshot);
    if !lifetime.live() {
        return Err(Hold::Locked);
    }
    let outcome = services.confirmation.confirm(
        &VaultNativeConfirmationRequest {
            issue_url: url,
            workspace_dir: workspace,
        },
        &lifetime.cancelled,
        lifetime.deadline,
    );
    if !lifetime.live() {
        return Err(Hold::Locked);
    }
    match outcome {
        #[cfg(any(target_os = "linux", target_os = "macos", test))]
        VaultNativeConfirmationOutcome::Confirmed => Ok(issue),
        VaultNativeConfirmationOutcome::Cancelled => Err(Hold::ConfirmationCancelled),
        VaultNativeConfirmationOutcome::Unsupported => Err(Hold::PlatformUnsupported),
        VaultNativeConfirmationOutcome::Unavailable => Err(Hold::SourceUnavailable),
    }
}

fn restore(
    services: &DesktopRestoreServices,
    lifetime: &Lifetime,
    issue: IssueReference,
) -> Result<VaultRestoreResponse, Hold> {
    if !lifetime.live() {
        return Err(Hold::Locked);
    }
    // 이 분기는 native prepare 성공 뒤에만 도달한다. JSON에는 이 결정을 나타내는 입력이 없다.
    let grant = match services.unlock.unlock(VaultUnlockRequest {
        issue,
        session: lifetime.session.clone(),
        decision: VaultUserDecision::Confirmed,
    }) {
        VaultUnlockOutcome::Unlocked(grant) => grant,
        VaultUnlockOutcome::SharedCopyOnly(_) => return Err(Hold::Locked),
    };
    if !lifetime.live() {
        return Err(Hold::Locked);
    }
    let restored = services
        .restore
        .restore_for_grant(&grant)
        .map_err(restore_hold)?;
    if !lifetime.live() {
        return Err(Hold::Locked);
    }
    Ok(restored)
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

#[cfg(test)]
async fn run_test_io<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    input: R,
    output: W,
    services: DesktopRestoreServices,
    duration: Duration,
) -> io::Result<()> {
    let mut frames = Frames::new(input);
    let Some((id, url, issue)) = first_request(&mut frames).await else {
        return Ok(());
    };
    session(frames, output, id, url, issue, services, duration).await
}
