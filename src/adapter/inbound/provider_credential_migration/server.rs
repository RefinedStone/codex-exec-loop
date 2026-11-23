use super::*;
use crate::application::port::outbound::provider_credentials_port::NativeOperationOwner;
use crate::domain::provider_credentials::*;
use crate::private_ipc::credential_frames as frames;
use protocol::{Hello, Request, Response, VERSION};
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::sync::oneshot;

pub(super) async fn listen(
    mut listener: crate::private_ipc::PrivateIpcListener,
    state: Arc<State>,
    mut stop: watch::Receiver<bool>,
) {
    let sessions = Arc::new(Semaphore::new(8));
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        if *stop.borrow() {
            break;
        }
        tokio::select! {
            biased;
            _=stop.changed()=>break,
            Some(_)=tasks.join_next(),if !tasks.is_empty()=>{},
            connection=listener.accept()=>{
                let Ok(connection)=connection else{break};
                let Ok(permit)=sessions.clone().try_acquire_owned()else{drop(connection);continue};
                if crate::private_ipc::verify_client_owner(&connection).is_err(){continue;}
                let state=state.clone();
                let stop=stop.clone();
                tasks.spawn(async move{
                    let (mut input,mut output)=tokio::io::split(connection);
                    let mut interruption=stop.clone();
                    tokio::select!{
                        biased;
                        _=interruption.changed()=>{},
                        _=session(&mut input,&mut output,state,stop,permit)=>{},
                    }
                });
            }
        }
    }
    // Stop reaches every session's header, prompt and raw-frame wait. Native work has its own owner flag.
    while tasks.join_next().await.is_some() {}
}
struct OwnerGuard {
    owner: NativeOperationOwner,
    state: Arc<State>,
    completed: bool,
    permit: Option<OwnedSemaphorePermit>,
}
impl Drop for OwnerGuard {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        self.owner.cancel();
        if let Some(permit) = self.permit.take() {
            // Eight permits bound both live sessions and durable cancellation work.
            let _ = self
                .state
                .cancellations
                .try_send((self.owner.clone(), permit));
        }
    }
}
async fn service_call<T: Send + 'static, R: AsyncRead + Unpin>(
    state: &Arc<State>,
    owner: &NativeOperationOwner,
    input: &mut R,
    stop: &mut watch::Receiver<bool>,
    action: impl FnOnce() -> CredentialResult<T> + Send + 'static,
) -> CredentialResult<T> {
    if !owner.is_live() || *stop.borrow() {
        return Err(ProviderCredentialError::Cancelled);
    }
    let permit = state
        .workers
        .clone()
        .try_acquire_owned()
        .map_err(|_| ProviderCredentialError::Busy)?;
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _permit = permit;
        let _ = tx.send(action());
    });
    tokio::select! {
        biased;
        _=stop.changed()=>{owner.cancel();Err(ProviderCredentialError::Cancelled)}
        _=tokio::time::sleep_until(owner.deadline().into())=>{owner.cancel();Err(ProviderCredentialError::OutcomeUnknown)}
        _=input.read_u8()=>{owner.cancel();Err(ProviderCredentialError::Cancelled)}
        result=rx=>result.map_err(|_|ProviderCredentialError::Unavailable)?,
    }
}
pub(super) async fn session<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    input: &mut R,
    output: &mut W,
    state: Arc<State>,
    mut stop: watch::Receiver<bool>,
    permit: OwnedSemaphorePermit,
) -> CredentialResult<()> {
    let deadline = Instant::now() + frames::SESSION_TIMEOUT;
    let first_deadline = (Instant::now() + frames::FIRST_REQUEST_TIMEOUT).min(deadline);
    let hello = Hello {
        version: VERSION,
        profile_id: state.profile_id.clone(),
    };
    tokio::time::timeout_at(
        first_deadline.into(),
        frames::write_header_async(output, &hello),
    )
    .await
    .map_err(|_| ProviderCredentialError::Unavailable)??;
    let request: Request = tokio::select! {
        biased;
        _=stop.changed()=>return Err(ProviderCredentialError::Cancelled),
        result=tokio::time::timeout_at(first_deadline.into(),frames::read_header_async(input))=>
            result.map_err(|_|ProviderCredentialError::Unavailable)??,
    };
    if let Err(error) = request.validate(&state.profile_id) {
        return frames::write_header_async(output, &Response::held(request.id(), error)).await;
    }
    let owner = NativeOperationOwner::new(format!("{:032x}", rand::random::<u128>()), deadline)?;
    let mut guard = OwnerGuard {
        owner: owner.clone(),
        state: state.clone(),
        completed: false,
        permit: Some(permit),
    };
    let response = match &request {
        Request::Offer { .. } => offer(&request, input, output, &state, &owner, &mut stop).await?,
        Request::Operation { .. } => {
            let query = request.operation().unwrap();
            let service = state.service.clone();
            match service_call(&state, &owner, input, &mut stop, move || {
                service.migration_operation(query)
            })
            .await
            {
                Ok(receipt) => Response::from_receipt(request.id(), receipt),
                Err(error) => Response::held(request.id(), error),
            }
        }
        Request::Cleanup {
            profile_id,
            operation_id,
            provider,
            snapshot_id,
            ..
        } => {
            let query = LegacyCleanupRequest {
                profile_id: profile_id.clone(),
                operation_id: operation_id.clone(),
                provider: *provider,
                snapshot_id: snapshot_id.clone(),
            };
            let service = state.service.clone();
            match service_call(&state, &owner, input, &mut stop, move || {
                service.acknowledge_legacy_cleanup(query)
            })
            .await
            {
                Ok(receipt) => Response::from_receipt(request.id(), receipt),
                Err(error) => Response::held(request.id(), error),
            }
        }
        Request::ValidateLink { reference, url, .. } => {
            let reference = reference.clone();
            let url = url.clone();
            let validate = state.validate_link.clone();
            match service_call(&state, &owner, input, &mut stop, move || {
                validate(&reference, &url)
            })
            .await
            {
                Ok(url) => Response::ValidatedLink {
                    version: VERSION,
                    id: request.id().into(),
                    url,
                },
                Err(error) => Response::held(request.id(), error),
            }
        }
    };
    response.validate(&request)?;
    tokio::time::timeout_at(
        deadline.into(),
        frames::write_header_async(output, &response),
    )
    .await
    .map_err(|_| ProviderCredentialError::OutcomeUnknown)??;
    guard.completed = match &response {
        Response::Unknown { .. } | Response::Prepared { .. } => false,
        Response::Operation { receipt, .. } | Response::Committed { receipt, .. } => {
            !receipt.status.is_pending()
        }
        Response::Held { .. } | Response::ValidatedLink { .. } => true,
    };
    Ok(())
}
async fn offer<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    request: &Request,
    input: &mut R,
    output: &mut W,
    state: &Arc<State>,
    owner: &NativeOperationOwner,
    stop: &mut watch::Receiver<bool>,
) -> CredentialResult<Response> {
    let offer = request.offer().unwrap();
    let service = state.service.clone();
    let prepared_offer = offer.clone();
    let prepared_owner = owner.clone();
    let prepared = service_call(state, owner, input, stop, move || {
        service.prepare_import(prepared_offer, prepared_owner)
    })
    .await;
    let receipt = match prepared {
        Ok(receipt) => receipt,
        Err(error) => {
            return receipt_after_error(request, input, state, owner, stop, error, false).await;
        }
    };
    if receipt.status != CredentialOperationStatus::Prepared {
        return Ok(Response::from_receipt(request.id(), receipt));
    }
    if receipt.profile_id != offer.profile_id
        || receipt.operation_id != offer.operation_id
        || receipt.provider != offer.provider
        || receipt.expected_revision != offer.expected_revision
        || receipt.legacy_snapshot_id.as_deref() != Some(offer.snapshot_id.as_str())
    {
        return Err(ProviderCredentialError::InvalidRequest);
    }
    let prepared = Response::Prepared {
        version: VERSION,
        id: request.id().into(),
        profile_id: offer.profile_id.clone(),
        operation_id: offer.operation_id.clone(),
        expected_revision: offer.expected_revision,
    };
    tokio::time::timeout_at(
        owner.deadline().into(),
        frames::write_header_async(output, &prepared),
    )
    .await
    .map_err(|_| ProviderCredentialError::Unavailable)??;
    let token = tokio::select! {
        biased;
        _=stop.changed()=>{owner.cancel();return Err(ProviderCredentialError::Cancelled)},
        result=tokio::time::timeout_at(owner.deadline().into(),frames::read_secret_async(input,offer.provider))=>
            result.map_err(|_|ProviderCredentialError::Cancelled)??,
    };
    let service = state.service.clone();
    let import_owner = owner.clone();
    match service_call(state, owner, input, stop, move || {
        service.import_secret(offer, token, import_owner)
    })
    .await
    {
        Ok(receipt) => Ok(Response::from_receipt(request.id(), receipt)),
        Err(error) => receipt_after_error(request, input, state, owner, stop, error, true).await,
    }
}
async fn receipt_after_error<R: AsyncRead + Unpin>(
    request: &Request,
    input: &mut R,
    state: &Arc<State>,
    owner: &NativeOperationOwner,
    stop: &mut watch::Receiver<bool>,
    error: ProviderCredentialError,
    secret_sent: bool,
) -> CredentialResult<Response> {
    if !owner.is_live() {
        return Ok(Response::unknown(request));
    }
    let query = request.operation().unwrap();
    let service = state.service.clone();
    match service_call(state, owner, input, stop, move || {
        service.migration_operation(query)
    })
    .await
    {
        Ok(receipt) => Ok(Response::from_receipt(request.id(), receipt)),
        Err(ProviderCredentialError::NotFound) if !secret_sent => {
            Ok(Response::held(request.id(), error))
        }
        Err(_) => Ok(Response::unknown(request)),
    }
}
