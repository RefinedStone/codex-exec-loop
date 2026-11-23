//! Gateway-owned prompt process. Only this private pipe carries newly entered tokens.
use crate::application::port::outbound::provider_credentials_port::NativePromptPort;
use crate::domain::provider_credentials::*;
use crate::private_ipc::credential_frames as frames;
use crate::private_ipc::credential_prompt::{PromptRequest, PromptResponse};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

mod child;
mod native;
#[cfg(test)]
mod tests;

pub(crate) struct NativeCredentialPromptAdapter;
/// Used only by the dedicated prompt subcommand, never by the Gateway service directly.
pub(crate) struct NativeCredentialInputAdapter;

impl NativePromptPort for NativeCredentialPromptAdapter {
    fn prompt(
        &self,
        request: &ProviderCredentialPromptRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<SecretToken> {
        validate(request)?;
        let wire = PromptRequest::Token {
            version: 1,
            request: request.clone(),
        };
        match invoke(wire, cancelled, deadline)? {
            PromptOutcome::Token(token) => Ok(token),
            PromptOutcome::Confirmed => Err(ProviderCredentialError::InvalidRequest),
        }
    }
    fn confirm_migration(
        &self,
        request: &LegacyImportOffer,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<bool> {
        validate(&request.connect_request())?;
        if !valid_snapshot_id(&request.snapshot_id) {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        match invoke(
            PromptRequest::MigrationConsent {
                version: 1,
                offer: request.clone(),
            },
            cancelled,
            deadline,
        ) {
            Ok(PromptOutcome::Confirmed) => Ok(true),
            Err(ProviderCredentialError::Cancelled) => Ok(false),
            Ok(PromptOutcome::Token(_)) => Err(ProviderCredentialError::InvalidRequest),
            Err(error) => Err(error),
        }
    }
}
impl NativePromptPort for NativeCredentialInputAdapter {
    fn supported(&self) -> bool {
        cfg!(any(windows, target_os = "macos", target_os = "linux"))
    }
    fn prompt(
        &self,
        request: &ProviderCredentialPromptRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<SecretToken> {
        validate(request)?;
        check_live(cancelled, deadline)?;
        native::prompt(request, cancelled, deadline)
    }
    fn confirm_migration(
        &self,
        request: &LegacyImportOffer,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<bool> {
        validate(&request.connect_request())?;
        if !valid_snapshot_id(&request.snapshot_id) {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        check_live(cancelled, deadline)?;
        native::confirm(&request.connect_request(), true, cancelled, deadline)
    }
}
fn validate(request: &ConnectRequest) -> CredentialResult<()> {
    if !valid_operation_id(&request.operation_id)
        || !valid_profile_id(&request.profile_id)
        || !valid_revision(request.expected_revision)
    {
        return Err(ProviderCredentialError::InvalidRequest);
    }
    if request.configuration.normalized(request.provider)? != request.configuration {
        return Err(ProviderCredentialError::InvalidRequest);
    }
    Ok(())
}
fn check_live(cancelled: &AtomicBool, deadline: Instant) -> CredentialResult<()> {
    if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
        Err(ProviderCredentialError::Cancelled)
    } else {
        Ok(())
    }
}
enum PromptOutcome {
    Token(SecretToken),
    Confirmed,
}
fn invoke(
    request: PromptRequest,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> CredentialResult<PromptOutcome> {
    check_live(cancelled, deadline)?;
    let program = std::env::current_exe().map_err(|_| ProviderCredentialError::Unavailable)?;
    let cwd = std::env::current_dir().map_err(|_| ProviderCredentialError::Unavailable)?;
    let program = crate::trusted_executable::validate_absolute(&program, &cwd)
        .and_then(|program| {
            crate::trusted_executable::validate_native_executable(&program)?;
            Ok(program)
        })
        .map_err(|_| ProviderCredentialError::Unavailable)?;
    let mut command = child::prompt_command(&program, |name| std::env::var_os(name));
    let output = child::capture(
        &mut command,
        frames::encode_header(&request)?,
        true,
        cancelled,
        deadline,
        frames::MAX_HEADER_BYTES + frames::MAX_SECRET_BYTES + 4,
    )?;
    decode_response(&request, &output)
}
fn decode_response(request: &PromptRequest, output: &[u8]) -> CredentialResult<PromptOutcome> {
    let mut reader = std::io::Cursor::new(output);
    let response: PromptResponse = frames::read_header(&mut reader)?;
    let result = match response {
        PromptResponse::Submitted {
            version: 1,
            operation_id,
        } if operation_id == request.operation_id()
            && matches!(request, PromptRequest::Token { .. }) =>
        {
            PromptOutcome::Token(frames::read_secret(&mut reader, request.provider())?)
        }
        PromptResponse::Confirmed {
            version: 1,
            operation_id,
        } if operation_id == request.operation_id()
            && matches!(request, PromptRequest::MigrationConsent { .. }) =>
        {
            PromptOutcome::Confirmed
        }
        PromptResponse::Rejected {
            version: 1,
            operation_id,
            error,
        } if operation_id == request.operation_id() => {
            frames::require_eof(&mut reader)?;
            return Err(error);
        }
        _ => return Err(ProviderCredentialError::InvalidRequest),
    };
    frames::require_eof(&mut reader)?;
    Ok(result)
}
