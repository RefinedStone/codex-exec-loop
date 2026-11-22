//! 원문 열람·재연결 전용 inherited-pipe 입구다. 공개 요청은 OS 확인 결과를 지정할 수 없다.
mod protocol;
#[cfg(test)]
mod tests;

use crate::application::port::inbound::vault_restore_session_port::{
    VaultRestoreSessionPort, VaultRestoreSessionRequest,
};
use protocol::{Capabilities, Frames, Hold, OutputBudget, Reply, Request};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

const FIRST_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

struct EndSession(Arc<dyn VaultRestoreSessionPort>);
impl Drop for EndSession {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(super) async fn run() -> anyhow::Result<()> {
    let input = tokio::io::stdin();
    let mut output = tokio::io::stdout();
    let mut frames = Frames::new(input);
    let Some((id, request)) = first_request(&mut frames).await else {
        return Ok(());
    };
    let Ok(port) = crate::composition::desktop_restore::build() else {
        let encoded = OutputBudget::new().encode(
            &id,
            Reply::Held {
                reason: Hold::SourceUnavailable,
                can_reconnect: false,
            },
            true,
        )?;
        tokio::time::timeout(
            FIRST_REQUEST_TIMEOUT,
            protocol::write(&mut output, &encoded.bytes),
        )
        .await
        .map_err(|_| anyhow::anyhow!("local restore channel closed"))??;
        return Ok(());
    };
    session(frames, output, id, request, port)
        .await
        .map_err(|_| anyhow::anyhow!("local restore channel closed"))
}

async fn first_request<R: AsyncRead + Unpin>(
    frames: &mut Frames<R>,
) -> Option<(String, VaultRestoreSessionRequest)> {
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
    Some((
        id,
        VaultRestoreSessionRequest {
            issue_url,
            issue: issue.into_reference()?,
        },
    ))
}

async fn next_request<R: AsyncRead + Unpin>(
    frames: &mut Frames<R>,
    id: &str,
    port: &Arc<dyn VaultRestoreSessionPort>,
) -> Option<Request> {
    let deadline = tokio::time::Instant::from_std(port.deadline());
    let Ok(Ok(Some(request))) = tokio::time::timeout_at(deadline, frames.read()).await else {
        port.cancel();
        return None;
    };
    let (version, request_id) = request.header();
    if !protocol::valid_header(version, request_id)
        || request_id != id
        || frames.buffered_input()
        || !port.is_live()
    {
        port.cancel();
        return None;
    }
    Some(request)
}

/// 외부 I/O를 기다릴 때 입력·EOF·기한을 우선하여 세션을 즉시 무효화한다.
async fn operation<R, T, F>(
    frames: &mut Frames<R>,
    port: &Arc<dyn VaultRestoreSessionPort>,
    action: F,
) -> Option<T>
where
    R: AsyncRead + Unpin,
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    if !port.is_live() || frames.buffered_input() {
        port.cancel();
        return None;
    }
    let deadline = tokio::time::Instant::from_std(port.deadline());
    let mut running = tokio::task::spawn_blocking(action);
    let result = tokio::select! {
        biased;
        _ = frames.unexpected_input() => { port.cancel(); return None; }
        _ = tokio::time::sleep_until(deadline) => { port.cancel(); return None; }
        result = &mut running => result.ok(),
    };
    if !port.is_live() {
        port.cancel();
        return None;
    }
    result
}

/// true인 경우만 같은 세션의 다음 요청을 받는다. 최종 응답은 정상 helper 종료 뒤에만 UI가 사용한다.
async fn respond<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    frames: &mut Frames<R>,
    output: &mut W,
    budget: &mut OutputBudget,
    id: &str,
    port: &Arc<dyn VaultRestoreSessionPort>,
    reply: Reply<'_>,
    terminal: bool,
) -> io::Result<bool> {
    if !port.is_live() {
        return Ok(false);
    }
    let encoded = budget.encode(id, reply, terminal)?;
    let deadline = tokio::time::Instant::from_std(port.deadline());
    // 직렬화 뒤에도 추가 입력/EOF/마감을 먼저 검사한다. 마지막 개행을 쓴 뒤에는
    // 클라이언트가 다음 요청을 보낼 수 있으므로 flush 대기 중 그 요청을 취소로 오인하지 않는다.
    tokio::select! {
        biased;
        _ = frames.unexpected_input() => { port.cancel(); return Ok(false); }
        _ = tokio::time::sleep_until(deadline) => { port.cancel(); return Ok(false); }
        result = async {
            if !port.is_live() { return Err(io::Error::other("local restore channel closed")); }
            output.write_all(&encoded.bytes).await
        } => result?,
    }
    tokio::time::timeout_at(deadline, output.flush())
        .await
        .map_err(|_| io::Error::other("local restore channel closed"))??;
    Ok(!encoded.terminal && port.is_live())
}

async fn session<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut frames: Frames<R>,
    mut output: W,
    id: String,
    request: VaultRestoreSessionRequest,
    port: Arc<dyn VaultRestoreSessionPort>,
) -> io::Result<()> {
    let _end = EndSession(port.clone());
    let mut budget = OutputBudget::new();
    let preparing = port.clone();
    let Some(prepared) = operation(&mut frames, &port, move || preparing.prepare(request)).await
    else {
        return Ok(());
    };
    let prepared = match prepared {
        Ok(value) => value,
        Err(reason) => {
            respond(
                &mut frames,
                &mut output,
                &mut budget,
                &id,
                &port,
                Reply::Held {
                    reason: reason.into(),
                    can_reconnect: false,
                },
                true,
            )
            .await?;
            return Ok(());
        }
    };
    if !respond(
        &mut frames,
        &mut output,
        &mut budget,
        &id,
        &port,
        Reply::Prepared {
            capabilities: Capabilities {
                reconnect: prepared.reconnect,
            },
        },
        false,
    )
    .await?
    {
        return Ok(());
    }

    let Some(Request::Restore { .. }) = next_request(&mut frames, &id, &port).await else {
        return Ok(());
    };
    let restoring = port.clone();
    let Some(restored) = operation(&mut frames, &port, move || restoring.restore()).await else {
        return Ok(());
    };
    match restored {
        Ok(value) => {
            respond(
                &mut frames,
                &mut output,
                &mut budget,
                &id,
                &port,
                Reply::Restored {
                    title: value.title.expose(),
                    body: value.body.expose(),
                },
                true,
            )
            .await?;
            return Ok(());
        }
        Err(failure) => {
            let reason: Hold = failure.reason.into();
            let can_reconnect =
                prepared.reconnect && failure.can_reconnect && reason.reconnectable();
            if !respond(
                &mut frames,
                &mut output,
                &mut budget,
                &id,
                &port,
                Reply::Held {
                    reason,
                    can_reconnect,
                },
                !can_reconnect,
            )
            .await?
            {
                return Ok(());
            }
        }
    }

    let Some(Request::Review { .. }) = next_request(&mut frames, &id, &port).await else {
        return Ok(());
    };
    let reviewing = port.clone();
    let Some(review) = operation(&mut frames, &port, move || reviewing.review()).await else {
        return Ok(());
    };
    let review = match review {
        Ok(value) => value,
        Err(reason) => {
            respond(
                &mut frames,
                &mut output,
                &mut budget,
                &id,
                &port,
                Reply::Held {
                    reason: reason.into(),
                    can_reconnect: false,
                },
                true,
            )
            .await?;
            return Ok(());
        }
    };
    if !respond(
        &mut frames,
        &mut output,
        &mut budget,
        &id,
        &port,
        Reply::Review {
            review: (&review).into(),
        },
        false,
    )
    .await?
    {
        return Ok(());
    }
    drop(review);

    let Some(request) = next_request(&mut frames, &id, &port)
        .await
        .and_then(Request::into_reconnect)
    else {
        return Ok(());
    };
    let reconnecting = port.clone();
    let Some(reconnected) =
        operation(&mut frames, &port, move || reconnecting.reconnect(request)).await
    else {
        return Ok(());
    };
    if let Err(reason) = reconnected {
        respond(
            &mut frames,
            &mut output,
            &mut budget,
            &id,
            &port,
            Reply::Held {
                reason: reason.into(),
                can_reconnect: false,
            },
            true,
        )
        .await?;
        return Ok(());
    }
    if !respond(
        &mut frames,
        &mut output,
        &mut budget,
        &id,
        &port,
        Reply::Reconnected,
        false,
    )
    .await?
    {
        return Ok(());
    }

    let Some(Request::Restore { .. }) = next_request(&mut frames, &id, &port).await else {
        return Ok(());
    };
    let restoring = port.clone();
    let Some(restored) = operation(&mut frames, &port, move || restoring.restore()).await else {
        return Ok(());
    };
    let reply = match &restored {
        Ok(value) => Reply::Restored {
            title: value.title.expose(),
            body: value.body.expose(),
        },
        Err(failure) => Reply::Held {
            reason: failure.reason.into(),
            can_reconnect: false,
        },
    };
    respond(
        &mut frames,
        &mut output,
        &mut budget,
        &id,
        &port,
        reply,
        true,
    )
    .await?;
    Ok(())
}
