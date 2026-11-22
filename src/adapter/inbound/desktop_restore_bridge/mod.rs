//! 원문 열람 전용 inherited-pipe 입구다. 공개 요청은 OS 확인 결과를 지정할 수 없다.
mod protocol;
#[cfg(test)]
mod tests;

use crate::application::port::inbound::vault_restore_session_port::{
    VaultRestoreSessionHold, VaultRestoreSessionPort, VaultRestoreSessionRequest,
};
use protocol::{Frames, Hold, Reply, Request};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

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
        held(&mut output, &id, Hold::SourceUnavailable).await?;
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
    request: VaultRestoreSessionRequest,
    port: Arc<dyn VaultRestoreSessionPort>,
) -> io::Result<()> {
    let _end = EndSession(port.clone());
    if frames.buffered_input() {
        return Ok(());
    }
    let deadline = tokio::time::Instant::from_std(port.deadline());
    let prepare_port = port.clone();
    let mut preparing = tokio::task::spawn_blocking(move || prepare_port.prepare(request));
    let prepared = tokio::select! {
        biased;
        _ = frames.unexpected_input() => { port.cancel(); return Ok(()); }
        _ = tokio::time::sleep_until(deadline) => { port.cancel(); return Ok(()); }
        result = &mut preparing => result.unwrap_or(Err(VaultRestoreSessionHold::SourceUnavailable)),
    };
    if !port.is_live() {
        return Ok(());
    }
    if let Err(reason) = prepared {
        return held(&mut output, &id, reason.into()).await;
    }
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
        || !port.is_live()
    {
        return Ok(());
    }

    let restore_port = port.clone();
    let mut restoring = tokio::task::spawn_blocking(move || restore_port.restore());
    let restored = tokio::select! {
        biased;
        _ = frames.unexpected_input() => { port.cancel(); return Ok(()); }
        _ = tokio::time::sleep_until(deadline) => { port.cancel(); return Ok(()); }
        result = &mut restoring => result.unwrap_or(Err(VaultRestoreSessionHold::SourceUnavailable)),
    };
    if !port.is_live() {
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
        Err(reason) => protocol::encode(
            &id,
            Reply::Held {
                reason: (*reason).into(),
            },
        )?,
    };
    // 직렬화가 끝난 뒤에도 EOF/추가 입력/마감을 먼저 검사한다. 평문 버퍼는 drop 시 지운다.
    tokio::select! {
        biased;
        _ = frames.unexpected_input() => { port.cancel(); Ok(()) }
        _ = tokio::time::sleep_until(deadline) => { port.cancel(); Ok(()) }
        result = async {
            if !port.is_live() { return Ok(()); }
            protocol::write(&mut output, &bytes).await
        } => result,
    }
}
