//! Local-only typed provider RPC. Entire request families are excluded from recording before validation.
use crate::application::port::outbound::provider_credentials_port::{
    NativeOperationOwner, ProviderCredentialMigrationPort,
};
use crate::composition::provider_credentials::ProviderServices;
use crate::domain::provider_credentials::*;
use crate::domain::provider_work::LinearFilters;
use crate::domain::work_context::ExternalResourceRef;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Semaphore, mpsc};
use tokio_tungstenite::tungstenite::Message;

const CREDENTIAL_PREFIX: &str = "akra/providerCredentials/";
const READ_PREFIX: &str = "akra/providers/";
const MAX_REQUEST_BYTES: usize = 16 << 10;
const INPUT_DEADLINE: Duration = Duration::from_secs(120);

pub(super) fn is_provider_method(method: &str) -> bool {
    method.starts_with(CREDENTIAL_PREFIX) || method.starts_with(READ_PREFIX)
}
pub(super) fn malformed_provider_frame(text: &str) -> bool {
    // Duplicate keys and JSON batches can be valid JSON but invalid envelopes. Inspect every
    // top-level method (and each batch entry) before any generic forwarding or recording.
    if !text.contains("provider") && !text.contains('\\') {
        return false;
    }
    let mut sensitive = false;
    let mut decoder = serde_json::Deserializer::from_str(text);
    let parsed =
        serde::de::DeserializeSeed::deserialize(ProviderFrameSeed(&mut sensitive), &mut decoder)
            .and_then(|()| decoder.end());
    // A failed candidate parse may stop before an escaped method name is reached.
    // Treat it as sensitive rather than forwarding its undecodable body to the recorder.
    let candidate = sensitive || parsed.is_err();
    candidate
        && !matches!(super::router::route_client_text(text),
        super::router::ClientTextRoute::AkraRequest { method, .. }
        | super::router::ClientTextRoute::AkraNotification { method } if is_provider_method(&method))
}
struct ProviderFrameSeed<'a>(&'a mut bool);
impl<'de> serde::de::DeserializeSeed<'de> for ProviderFrameSeed<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}
impl<'de> serde::de::Visitor<'de> for ProviderFrameSeed<'_> {
    type Value = ();
    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a provider request envelope")
    }

    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some(key) = map.next_key::<String>()? {
            if key == "method" {
                let method = map.next_value::<Value>()?;
                if method.as_str().is_some_and(is_provider_method) {
                    *self.0 = true;
                }
            } else {
                map.next_value::<serde::de::IgnoredAny>()?;
            }
        }
        Ok(())
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> Result<(), A::Error> {
        while sequence
            .next_element_seed(ProviderFrameSeed(&mut *self.0))?
            .is_some()
        {}
        Ok(())
    }
}
fn safe_response_id(id: Value) -> Value {
    if id.as_i64().is_some()
        || id
            .as_str()
            .is_some_and(|value| !value.is_empty() && value.len() <= 128)
    {
        id
    } else {
        Value::Null
    }
}
pub(super) fn invalid_response(id: Value) -> String {
    error_response(id, -32602, ProviderCredentialError::InvalidRequest)
}
pub(super) fn unavailable_response(id: Value) -> String {
    error_response(id, -32601, ProviderCredentialError::Unsupported)
}
fn error_response(id: Value, code: i64, error: ProviderCredentialError) -> String {
    let id = safe_response_id(id);
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":error.to_string(),"data":{"kind":error}}}).to_string()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<T> {
    jsonrpc: String,
    id: Value,
    method: String,
    params: T,
}
fn params<T: serde::de::DeserializeOwned>(text: &str, method: &str) -> CredentialResult<T> {
    if text.len() > MAX_REQUEST_BYTES {
        return Err(ProviderCredentialError::InvalidRequest);
    }
    let envelope: Envelope<T> =
        serde_json::from_str(text).map_err(|_| ProviderCredentialError::InvalidRequest)?;
    let valid_id = envelope.id.as_i64().is_some()
        || envelope
            .id
            .as_str()
            .is_some_and(|id| !id.is_empty() && id.len() <= 128);
    if envelope.jsonrpc != "2.0" || envelope.method != method || !valid_id {
        return Err(ProviderCredentialError::InvalidRequest);
    }
    Ok(envelope.params)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateRequest {
    provider: Provider,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadRequest {
    connection: ConnectionRef,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JiraPromptRequest {
    connection: ConnectionRef,
    key: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JiraVersionRequest {
    connection: ConnectionRef,
    key: String,
    url: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LinearSummaryRequest {
    connection: ConnectionRef,
    filters: LinearFilters,
    refresh: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LinearPromptRequest {
    connection: ConnectionRef,
    id: String,
}

#[derive(Default)]
struct SessionOwners {
    closed: bool,
    owners: Vec<NativeOperationOwner>,
}

pub(super) struct ProviderSession {
    services: ProviderServices,
    owners: Arc<Mutex<SessionOwners>>,
    capacity: Arc<Semaphore>,
    watch: tokio::task::JoinHandle<()>,
}
impl ProviderSession {
    pub(super) fn start(services: ProviderServices, outbox: mpsc::Sender<Message>) -> Self {
        let mut changes = services.credentials.subscribe();
        let service = services.credentials.clone();
        let watch = tokio::spawn(async move {
            loop {
                match changes.recv().await {
                    Ok(change) => {
                        let message = json!({"jsonrpc":"2.0","method":"akra/providerCredentials/changed","params":change});
                        if outbox
                            .send(Message::text(message.to_string()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Resend both current revisions; a lag flag makes clients reload their state.
                        let service = service.clone();
                        let states = tokio::task::spawn_blocking(move || [Provider::Jira, Provider::Linear]
                            .into_iter().filter_map(|provider| service.state(provider).ok())
                            .map(|state| json!({"provider":state.provider,"profileId":state.profile_id,"revision":state.revision,"lagged":true}))
                            .collect::<Vec<_>>()).await.unwrap_or_default();
                        for state in states {
                            let message = json!({"jsonrpc":"2.0","method":"akra/providerCredentials/changed","params":state});
                            if outbox
                                .send(Message::text(message.to_string()))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Self {
            services,
            owners: Arc::new(Mutex::new(SessionOwners::default())),
            capacity: Arc::new(Semaphore::new(16)),
            watch,
        }
    }
    pub(super) fn request(
        &self,
        id: Value,
        method: String,
        text: String,
        outbox: mpsc::Sender<Message>,
    ) {
        let services = self.services.clone();
        let owners = self.owners.clone();
        let capacity = self.capacity.clone();
        tokio::spawn(async move {
            let permit = match capacity.try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    let _ = outbox
                        .send(Message::text(error_response(
                            id,
                            -32000,
                            ProviderCredentialError::Busy,
                        )))
                        .await;
                    return;
                }
            };
            let response_id = id.clone();
            let result =
                tokio::task::spawn_blocking(move || handle(&services, &owners, &method, &text))
                    .await;
            drop(permit);
            let response = match result {
                Ok(Ok(value)) => {
                    json!({"jsonrpc":"2.0","id":response_id,"result":value}).to_string()
                }
                Ok(Err(error)) => error_response(
                    response_id,
                    if error == ProviderCredentialError::InvalidRequest {
                        -32602
                    } else if error == ProviderCredentialError::Unsupported {
                        -32601
                    } else {
                        -32000
                    },
                    error,
                ),
                Err(_) => error_response(response_id, -32000, ProviderCredentialError::Unavailable),
            };
            // Provider reads may include private company content. Never pass response bodies to a recorder.
            let _ = outbox.send(Message::text(response)).await;
        });
    }
}
impl Drop for ProviderSession {
    fn drop(&mut self) {
        self.watch.abort();
        if let Ok(mut registry) = self.owners.lock() {
            registry.closed = true;
            for owner in &registry.owners {
                owner.cancel();
            }
            let owners = registry.owners.clone();
            let service = self.services.credentials.clone();
            tokio::task::spawn_blocking(move || {
                for owner in owners {
                    let _ = service.cancel_owner(&owner);
                }
            });
        }
    }
}
fn value<T: serde::Serialize>(value: T) -> CredentialResult<Value> {
    serde_json::to_value(value).map_err(|_| ProviderCredentialError::Unavailable)
}
fn handle(
    services: &ProviderServices,
    owners: &Mutex<SessionOwners>,
    method: &str,
    text: &str,
) -> CredentialResult<Value> {
    match method {
        "akra/providerCredentials/state" => {
            let request: StateRequest = params(text, method)?;
            value(services.credentials.state(request.provider)?)
        }
        "akra/providerCredentials/connect" => {
            let request: ConnectRequest = params(text, method)?;
            let owner = NativeOperationOwner::new(
                format!("{:032x}", rand::random::<u128>()),
                Instant::now() + INPUT_DEADLINE,
            )?;
            {
                let mut registry = owners
                    .lock()
                    .map_err(|_| ProviderCredentialError::Unavailable)?;
                if registry.closed {
                    return Err(ProviderCredentialError::Cancelled);
                }
                registry.owners.push(owner.clone());
            }
            let cancellation = CredentialOperationRequest {
                operation_id: request.operation_id.clone(),
                profile_id: request.profile_id.clone(),
                provider: request.provider,
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                services.credentials.start_connect(request, owner.clone())
            }))
            .unwrap_or_else(|_| {
                owner.cancel();
                let _ = services.credentials.cancel(cancellation);
                Err(ProviderCredentialError::Unavailable)
            });
            owner.cancel();
            if result.is_err() {
                let _ = services.credentials.cancel_owner(&owner);
            }
            if let Ok(mut registry) = owners.lock() {
                registry
                    .owners
                    .retain(|candidate| candidate.id() != owner.id());
            }
            value(result?)
        }
        "akra/providerCredentials/disconnect" => {
            value(services.credentials.disconnect(params(text, method)?)?)
        }
        "akra/providerCredentials/cancel" => {
            value(services.credentials.cancel(params(text, method)?)?)
        }
        "akra/providerCredentials/operation" => {
            value(services.credentials.operation(params(text, method)?)?)
        }
        "akra/providers/jira/summary" => {
            let request: ReadRequest = params(text, method)?;
            value(services.work.jira_summary(&request.connection)?)
        }
        "akra/providers/jira/issuePrompt" => {
            let request: JiraPromptRequest = params(text, method)?;
            Ok(json!({"text":services.work.jira_issue_prompt(&request.connection, &request.key)?}))
        }
        "akra/providers/jira/currentVersion" => {
            let request: JiraVersionRequest = params(text, method)?;
            let resource = ExternalResourceRef {
                kind: "jira".to_owned(),
                key: request.key,
                url: request.url,
                title_hint: None,
            };
            Ok(
                json!({"version":services.work.jira_current_version(&request.connection, &resource)?}),
            )
        }
        "akra/providers/linear/summary" => {
            let request: LinearSummaryRequest = params(text, method)?;
            value(services.work.linear_summary(
                &request.connection,
                &request.filters,
                request.refresh,
            )?)
        }
        "akra/providers/linear/issuePrompt" => {
            let request: LinearPromptRequest = params(text, method)?;
            Ok(json!({"text":services.work.linear_issue_prompt(&request.connection, &request.id)?}))
        }
        _ => Err(ProviderCredentialError::Unsupported),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_wire_rejects_secret_fields_without_echoing_them() {
        let method = "akra/providerCredentials/connect";
        let request = json!({"jsonrpc":"2.0","id":1,"method":method,"params":{
            "operationId":"test","profileId":"a".repeat(64),"provider":"linear","expectedRevision":0,
            "configuration":{},"token":"synthetic-secret-canary"
        }}).to_string();
        assert!(matches!(
            params::<ConnectRequest>(&request, method),
            Err(ProviderCredentialError::InvalidRequest)
        ));
        assert!(!invalid_response(json!(1)).contains("synthetic-secret-canary"));
    }
    #[test]
    fn nested_secrets_and_arbitrary_provider_proxy_are_rejected() {
        let method = "akra/providerCredentials/connect";
        let request = json!({"jsonrpc":"2.0","id":1,"method":method,"params":{
            "operationId":"test","profileId":"a".repeat(64),"provider":"linear","expectedRevision":0,
            "configuration":{"token":"synthetic-secret-canary"}
        }}).to_string();
        assert!(params::<ConnectRequest>(&request, method).is_err());
        let read = json!({"jsonrpc":"2.0","id":1,"method":"akra/providers/jira/summary",
            "params":{"connection":{"profileId":"a".repeat(64),"provider":"jira","connectionId":"b".repeat(32),"revision":1},
            "url":"https://attacker.invalid","authorization":"synthetic-secret-canary"}}).to_string();
        assert!(params::<ReadRequest>(&read, "akra/providers/jira/summary").is_err());
        assert!(is_provider_method("akra/providers/jira/arbitrary"));
        assert!(is_provider_method("akra/providerCredentials/importSecret"));
    }
    #[test]
    fn malformed_credential_frames_do_not_fall_through_to_recording() {
        assert!(malformed_provider_frame(
            r#"{"method":"akra/providerCredentials/connect","token":"synthetic-secret-canary""#
        ));
        for text in [
            r#"{"params":{"token":"synthetic-secret-canary",},"method":"akra\/providers\/jira\/summary"}"#,
            r#"{"params":{"token":"synthetic-secret-canary",},"method":"akra/\u0070roviderCredentials/connect"}"#,
        ] {
            assert!(malformed_provider_frame(text), "{text}");
        }
        assert!(!malformed_provider_frame(
            r#"{"method":"thread/list","params":{}}"#
        ));
    }
    #[test]
    fn duplicate_envelopes_and_batches_are_sensitive_before_routing() {
        for text in [
            r#"{"id":1,"method":"akra/providerCredentials/connect","method":"akra/providerCredentials/connect","params":{"token":"canary"}}"#,
            r#"{"id":1,"id":2,"method":"akra/providerCredentials/connect","params":{"token":"canary"}}"#,
            r#"{"id":1,"method":"akra/providerCredentials/connect","method":"thread/list","params":{"token":"canary"}}"#,
            r#"[{"id":1,"method":"akra/providerCredentials/connect","params":{"token":"canary"}}]"#,
            r#"[{"id":1,"method":"akra\u002fproviderCredentials\u002fconnect","params":{"token":"canary"}}]"#,
        ] {
            assert!(malformed_provider_frame(text), "{text}");
        }
        assert!(!malformed_provider_frame(
            r#"{"id":1,"method":"thread/start","params":{"example":{"method":"akra/providerCredentials/state"}}}"#
        ));
        assert_eq!(
            serde_json::from_str::<Value>(&invalid_response(json!({"token":"canary"}))).unwrap()["id"],
            Value::Null
        );
    }
}
