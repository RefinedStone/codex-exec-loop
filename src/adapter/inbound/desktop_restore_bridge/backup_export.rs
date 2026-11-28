//! One selected issue over private pipes. No destination path or key is read from argv.
use super::{Frames, first_request};
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use zeroize::Zeroizing;

const LIFETIME: Duration = Duration::from_secs(120);

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Reply<'a> {
    Exported {
        archive: &'a str,
        recovery_key: &'a str,
    },
    Rejected,
}

pub(in crate::adapter::inbound) async fn run() -> anyhow::Result<()> {
    let mut frames = Frames::new(tokio::io::stdin());
    let Some((_, target)) = first_request(&mut frames).await else {
        return Ok(());
    };
    if frames.buffered_input() {
        return Ok(());
    }
    let deadline = tokio::time::Instant::now() + LIFETIME;
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let work = tokio::task::spawn_blocking(move || {
        crate::composition::desktop_restore::export_backup(
            &target.issue,
            &worker_cancelled,
            deadline.into_std(),
        )
    });
    let export = tokio::select! {
        biased;
        _ = frames.unexpected_input() => {
            cancelled.store(true, Ordering::SeqCst);
            return Ok(());
        }
        _ = tokio::time::sleep_until(deadline) => {
            cancelled.store(true, Ordering::SeqCst);
            return Ok(());
        }
        result = work => result.ok().flatten(),
    };
    let key = export
        .as_ref()
        .map(|data| Zeroizing::new(data.recovery_key.expose_text()));
    let reply = match (&export, &key) {
        (Some(data), Some(key)) => Reply::Exported {
            archive: &data.archive,
            recovery_key: key,
        },
        _ => Reply::Rejected,
    };
    let mut bytes = Zeroizing::new(serde_json::to_vec(&reply)?);
    bytes.push(b'\n');
    let mut output = tokio::io::stdout();
    tokio::select! {
        biased;
        _ = frames.unexpected_input() => {},
        _ = tokio::time::sleep_until(deadline) => {},
        result = output.write_all(&bytes) => result?,
    }
    output.flush().await?;
    Ok(())
}
