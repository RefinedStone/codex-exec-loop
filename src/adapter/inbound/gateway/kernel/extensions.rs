use super::*;
use crate::adapter::inbound::kernel_protocol::{
    METHOD_EXTENSION_ATTACH, METHOD_EXTENSION_CHANGED, METHOD_EXTENSION_DETACH,
    METHOD_EXTENSION_LIST,
    extensions::{ExtensionAttachRequest, ExtensionAttachResponse, ExtensionCall, ExtensionScope},
};
use crate::application::port::inbound::extension_host_port::ExtensionHostPort;
use crate::domain::kernel_extension::ExtensionContext;
use crate::domain::kernel_extension::{ExtensionEvent, ExtensionScope as HostScope};

pub(super) struct AttachedExtension {
    host: Arc<dyn ExtensionHostPort>,
    events: tokio::task::JoinHandle<()>,
}
impl Drop for AttachedExtension {
    fn drop(&mut self) {
        self.events.abort();
    }
}

impl KernelSession {
    pub(super) fn extension_context(&self, host: &dyn ExtensionHostPort) -> ExtensionContext {
        let mut context = host.context(
            self.connection_id.clone(),
            (host.scope() == HostScope::Repository).then(|| host.host_id().to_owned()),
        );
        if let Some(verified) = &self.request_conversation {
            context.verified_thread = Some(verified.thread_id.clone());
            context.conversation_generation =
                Some((verified.epoch, verified.current_epoch.clone()));
        }
        context
    }
    pub(super) async fn extension(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Value, KernelProtocolError> {
        if method == METHOD_EXTENSION_ATTACH {
            let request: ExtensionAttachRequest = parse_params(params)?;
            if request.scope != HostScope::LocalUser
                || request.client_name.trim().is_empty()
                || request.client_name.len() > 128
            {
                return Err(KernelProtocolError {
                    code: ERROR_INVALID_PARAMS,
                    message: "expected a local_user scope and a nonempty client name".into(),
                });
            }
            // The OS-private Gateway determines the user and host; neither is accepted from JSON.
            let kernels = self.kernels.clone();
            let host = blocking(move || kernels.local_extensions())
                .await?
                .map_err(|e| unavailable(format!("{e:#}")))?;
            let events =
                forward_extension_events(host.subscribe(), self.outbox.clone(), host.clone());
            self.local_extension = Some(AttachedExtension {
                host: host.clone(),
                events,
            });
            return to_result(ExtensionAttachResponse {
                kernel: host.host_id().into(),
                scope: HostScope::LocalUser,
                generation: host.generation().into(),
            });
        }
        if method == METHOD_EXTENSION_DETACH {
            let request: ExtensionScope = parse_params(params)?;
            if self
                .local_extension
                .as_ref()
                .is_none_or(|a| a.host.host_id() != request.kernel)
            {
                return Err(unavailable(
                    "local extension attachment is unavailable".into(),
                ));
            }
            self.local_extension = None;
            return Ok(json!({}));
        }
        let (kernel, request) = if method == METHOD_EXTENSION_LIST {
            let scope: ExtensionScope = parse_params(params)?;
            (scope.kernel, None)
        } else {
            let call: ExtensionCall = parse_params(params)?;
            (call.kernel.clone(), Some(call))
        };
        if let Some(local) = &self.local_extension
            && local.host.host_id() == kernel
        {
            let host = local.host.clone();
            return self.invoke_registered(host, request).await;
        }
        let attached = self.attached()?;
        if attached.kernel != kernel {
            return Err(unavailable("저장소 연결이 바뀌었습니다.".into()));
        }
        let repository = self
            .kernels
            .extensions(&attached.kernel)
            .map_err(|e| unavailable(format!("{e:#}")))?;
        let kernel_for_build = kernel.clone();
        let host = blocking(move || repository.common(&kernel_for_build))
            .await?
            .map_err(|e| unavailable(format!("{e:#}")))?;
        if self
            .attached
            .as_ref()
            .is_some_and(|a| a.extension_events.is_none())
        {
            let events =
                forward_extension_events(host.subscribe(), self.outbox.clone(), host.clone());
            self.attached.as_mut().expect("attachment").extension_events = Some(events);
        }
        self.invoke_registered(host, request).await
    }
    async fn invoke_registered(
        &self,
        host: Arc<dyn ExtensionHostPort>,
        request: Option<ExtensionCall>,
    ) -> Result<Value, KernelProtocolError> {
        let Some(request) = request else {
            return to_result(host.catalog());
        };
        let context = self.extension_context(host.as_ref());
        blocking(move || {
            host.call(
                context,
                &request.extension,
                request.version,
                &request.action,
                request.input,
                request.idempotency_key,
            )
        })
        .await?
        .map_err(|e| unavailable(format!("{e:#}")))
    }
}
use tokio::sync::broadcast;
fn forward_extension_events(
    mut events: broadcast::Receiver<ExtensionEvent>,
    outbox: mpsc::Sender<Message>,
    host: Arc<dyn ExtensionHostPort>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let payload = match events.recv().await {
                Ok(event) => serde_json::to_value(event).expect("event"),
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    json!({"kernel":host.host_id(),"extension":"*",
                    "generation":host.generation(),"resync":true})
                }
                Err(broadcast::error::RecvError::Closed) => break,
            };
            if send_extension_event(&outbox, payload).await.is_err() {
                break;
            }
        }
    })
}
async fn send_extension_event(
    outbox: &mpsc::Sender<Message>,
    payload: Value,
) -> Result<(), mpsc::error::SendError<Message>> {
    outbox
        .send(Message::text(
            json!({"jsonrpc":"2.0","method":METHOD_EXTENSION_CHANGED,"params":payload}).to_string(),
        ))
        .await
}
