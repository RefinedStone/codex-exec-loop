//! Private inherited-pipe entry point for the Gateway-owned native input process.
use crate::application::port::outbound::provider_credentials_port::NativePromptPort;
use crate::domain::provider_credentials::{
    CredentialResult, ProviderCredentialError as Error, SecretToken,
};
use crate::private_ipc::credential_frames as frames;
use crate::private_ipc::credential_prompt::{PromptRequest, PromptResponse};
use std::io::{Read, Write};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests;

/// The composition root injects the direct OS input adapter here, not the child-spawning adapter.
pub(crate) fn run(port: Arc<dyn NativePromptPort>) -> anyhow::Result<()> {
    crate::private_ipc::require_inherited_pipes()?;
    serve(
        std::io::stdin(),
        std::io::stdout(),
        port,
        frames::FIRST_REQUEST_TIMEOUT,
        frames::SESSION_TIMEOUT,
    )
    .map_err(|_| anyhow::anyhow!("native credential input did not complete"))
}
enum InputOutcome {
    Token(SecretToken),
    Confirmed,
}
fn serve<R: Read + Send + 'static, W: Write>(
    mut input: R,
    mut output: W,
    port: Arc<dyn NativePromptPort>,
    first_timeout: Duration,
    lifetime: Duration,
) -> CredentialResult<()> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let monitor = cancelled.clone();
    let (request_tx, request_rx) = mpsc::sync_channel(1);
    // A plain thread permits process exit after deadline even when OS input is modal.
    // It never holds a secret; parent death/extra input invalidates the owned request.
    std::thread::spawn(move || {
        let request = frames::read_header::<_, PromptRequest>(&mut input);
        let valid = request.is_ok();
        if request_tx.send(request).is_err() || !valid {
            return;
        }
        let mut byte = [0u8; 1];
        let _ = input.read(&mut byte);
        monitor.store(true, Ordering::SeqCst);
    });
    let request = request_rx
        .recv_timeout(first_timeout)
        .map_err(|_| Error::Cancelled)??;
    if request.version() != 1
        || !crate::domain::provider_credentials::valid_operation_id(request.operation_id())
    {
        return Err(Error::InvalidRequest);
    }
    if cancelled.load(Ordering::SeqCst) {
        return Err(Error::Cancelled);
    }
    let operation_id = request.operation_id().to_owned();
    let deadline = Instant::now() + lifetime;
    let worker_cancelled = cancelled.clone();
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = match request {
            PromptRequest::Token { request, .. } => port
                .prompt(&request, &worker_cancelled, deadline)
                .map(InputOutcome::Token),
            PromptRequest::MigrationConsent { offer, .. } => port
                .confirm_migration(&offer, &worker_cancelled, deadline)
                .and_then(|confirmed| {
                    if confirmed {
                        Ok(InputOutcome::Confirmed)
                    } else {
                        Err(Error::Cancelled)
                    }
                }),
        };
        let _ = result_tx.send(result);
    });
    let outcome = loop {
        if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
            cancelled.store(true, Ordering::SeqCst);
            return Err(Error::Cancelled);
        }
        match result_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(result) => break result,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Error::Unavailable),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    if cancelled.load(Ordering::SeqCst) {
        return Err(Error::Cancelled);
    }
    match outcome {
        Ok(InputOutcome::Token(token)) => {
            frames::write_header(
                &mut output,
                &PromptResponse::Submitted {
                    version: 1,
                    operation_id,
                },
            )?;
            frames::write_secret(&mut output, &token)
        }
        Ok(InputOutcome::Confirmed) => frames::write_header(
            &mut output,
            &PromptResponse::Confirmed {
                version: 1,
                operation_id,
            },
        ),
        Err(error) => frames::write_header(
            &mut output,
            &PromptResponse::Rejected {
                version: 1,
                operation_id,
                error,
            },
        ),
    }
}
