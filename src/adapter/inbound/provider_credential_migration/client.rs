use super::*;
use crate::domain::provider_credentials::{
    CredentialResult, ProviderCredentialError as Error, SecretToken,
};
use crate::private_ipc::credential_frames as frames;
use protocol::{Hello, Request, Response, VERSION};
use std::io::{Read, Write};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::oneshot;

/// Desktop main launches this with fixed argv and inherited private stdin/stdout only.
pub(crate) async fn run(profile_id: String, runtime: PathBuf) -> anyhow::Result<()> {
    crate::private_ipc::require_inherited_pipes()?;
    let deadline = tokio::time::Instant::now() + frames::SESSION_TIMEOUT;
    let first_deadline =
        (tokio::time::Instant::now() + frames::FIRST_REQUEST_TIMEOUT).min(deadline);
    let (tx, rx) = oneshot::channel();
    // Do not use tokio::stdin: its blocking reader cannot be cancelled during runtime shutdown.
    std::thread::spawn(move || {
        let mut input = std::io::stdin();
        let request = frames::read_header::<_, Request>(&mut input);
        let _ = tx.send((request, input));
    });
    let (request, input) = tokio::time::timeout_at(first_deadline, rx)
        .await
        .map_err(|_| anyhow::anyhow!("provider migration input is unavailable"))?
        .map_err(|_| anyhow::anyhow!("provider migration input is unavailable"))?;
    let request = request.map_err(|_| anyhow::anyhow!("provider migration input is invalid"))?;
    let mut output = std::io::stdout();
    if let Err(error) = request.validate(&profile_id) {
        frames::write_header(&mut output, &Response::held(request.id(), error))?;
        return Ok(());
    }
    let endpoint = endpoint(&profile_id, &runtime)?;
    let connection = crate::private_ipc::connect_owner_checked(&endpoint, first_deadline).await;
    let mut connection = match connection {
        Ok(connection) => connection,
        Err(_) => {
            frames::write_header(
                &mut output,
                &Response::held(request.id(), Error::Unavailable),
            )?;
            return Ok(());
        }
    };
    // OS peer verification above occurs before even the offer is written, and before any token is read.
    let response = bridge(
        &profile_id,
        &request,
        input,
        &mut output,
        &mut connection,
        deadline,
    )
    .await;
    match response {
        Ok(response) => frames::write_header(&mut output, &response)?,
        Err(error) => frames::write_header(&mut output, &Response::held(request.id(), error))?,
    }
    Ok(())
}
async fn read_secret_from_owner<R: Read + Send + 'static>(
    mut input: R,
    provider: crate::domain::provider_credentials::Provider,
    deadline: tokio::time::Instant,
) -> CredentialResult<SecretToken> {
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let result = frames::read_secret(&mut input, provider).and_then(|token| {
            frames::require_eof(&mut input)?;
            Ok(token)
        });
        let _ = tx.send(result);
    });
    tokio::time::timeout_at(deadline, rx)
        .await
        .map_err(|_| Error::Cancelled)?
        .map_err(|_| Error::Unavailable)?
}
pub(super) async fn bridge<
    R: Read + Send + 'static,
    W: Write,
    S: AsyncRead + AsyncWrite + Unpin,
>(
    profile: &str,
    request: &Request,
    input: R,
    output: &mut W,
    connection: &mut S,
    deadline: tokio::time::Instant,
) -> CredentialResult<Response> {
    let first_deadline =
        (tokio::time::Instant::now() + frames::FIRST_REQUEST_TIMEOUT).min(deadline);
    let hello: Hello =
        tokio::time::timeout_at(first_deadline, frames::read_header_async(connection))
            .await
            .map_err(|_| Error::Unavailable)??;
    if hello.version != VERSION || hello.profile_id != profile || request.profile_id() != profile {
        return Err(Error::Unauthorized);
    }
    tokio::time::timeout_at(
        first_deadline,
        frames::write_header_async(connection, request),
    )
    .await
    .map_err(|_| Error::Unavailable)??;
    let response: Response =
        tokio::time::timeout_at(deadline, frames::read_header_async(connection))
            .await
            .map_err(|_| Error::OutcomeUnknown)??;
    response.validate(request)?;
    if !matches!(response, Response::Prepared { .. }) {
        return match tokio::time::timeout_at(deadline, frames::require_eof_async(connection)).await
        {
            Ok(Ok(())) => Ok(response),
            _ if request.operation_id().is_some() => Ok(Response::unknown(request)),
            _ => Err(Error::InvalidRequest),
        };
    }
    // This is the first point at which Desktop main may decrypt the legacy cipher snapshot.
    frames::write_header(output, &response)?;
    let token = match read_secret_from_owner(input, request.provider(), deadline).await {
        Ok(token) => token,
        Err(_) => return Ok(Response::unknown(request)),
    };
    // One secret transfer only. There is no retry after any byte may have reached the owner.
    if !tokio::time::timeout_at(deadline, frames::write_secret_async(connection, &token))
        .await
        .is_ok_and(|result| result.is_ok())
    {
        return Ok(Response::unknown(request));
    }
    drop(token);
    let terminal: Response =
        match tokio::time::timeout_at(deadline, frames::read_header_async(connection)).await {
            Ok(Ok(response)) => response,
            _ => return Ok(Response::unknown(request)),
        };
    if terminal.validate(request).is_err() || matches!(terminal, Response::Prepared { .. }) {
        return Ok(Response::unknown(request));
    }
    // The private endpoint closes after the terminal response. Extra output is not a receipt.
    match tokio::time::timeout_at(deadline, frames::require_eof_async(connection)).await {
        Ok(Ok(())) => Ok(terminal),
        _ => Ok(Response::unknown(request)),
    }
}
