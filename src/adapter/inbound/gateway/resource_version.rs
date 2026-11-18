//! Reverse version requests are isolated from the app-server RPC stream.

use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::application::port::inbound::resource_version_provider_port::{
    RESOURCE_VERSION_REQUEST_PREFIX, ResourceVersionProviderPort,
};

pub(super) const METHOD_PREFIX: &str = "akra/resourceVersion/";

pub(super) struct ProviderSession {
    service: Arc<dyn ResourceVersionProviderPort>,
    session: u64,
    relay: tokio::task::JoinHandle<()>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Registration {
    kind: String,
    site_url: Option<String>,
}

impl ProviderSession {
    pub(super) fn start(
        service: Arc<dyn ResourceVersionProviderPort>,
        outbox: mpsc::Sender<Message>,
    ) -> Result<Self> {
        let (session, mut requests) = service.open()?;
        let relay = tokio::spawn(async move {
            while let Some(request) = requests.recv().await {
                let message = json!({
                    "jsonrpc": "2.0", "id": request.id,
                    "method": "akra/resourceVersion/current",
                    "params": { "kind": "jira", "key": request.resource.key, "url": request.resource.url }
                });
                if outbox
                    .send(Message::text(message.to_string()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        Ok(Self {
            service,
            session,
            relay,
        })
    }

    pub(super) fn request(&self, id: Value, method: &str, text: &str) -> String {
        let result = self.register(method, text);
        match result {
            Ok(()) => json!({"jsonrpc":"2.0","id":id,"result":{}}),
            Err(error) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":error.to_string()}})
            }
        }
        .to_string()
    }

    fn register(&self, method: &str, text: &str) -> Result<()> {
        let envelope: Value = serde_json::from_str(text)?;
        let params: Registration = serde_json::from_value(envelope["params"].clone())?;
        if params.kind != "jira" {
            bail!("unsupported provider kind")
        }
        match method {
            "akra/resourceVersion/register" => {
                let site = params
                    .site_url
                    .ok_or_else(|| anyhow!("siteUrl is required"))?;
                self.service.register(self.session, Some(&site))
            }
            "akra/resourceVersion/unregister" => self.service.register(self.session, None),
            _ => bail!("unknown resource provider method"),
        }
    }

    pub(super) fn answer(&self, id: &str, text: &str) {
        let result = serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|message| {
                if message.get("error").is_some() {
                    return None;
                }
                message
                    .get("result")?
                    .get("version")?
                    .as_str()
                    .map(str::to_owned)
            })
            .ok_or_else(|| anyhow!("Desktop could not read the Jira version"));
        self.service.answer(self.session, id, result);
    }
}

impl Drop for ProviderSession {
    fn drop(&mut self) {
        self.service.close(self.session);
        self.relay.abort();
    }
}

pub(super) fn response_id(text: &str) -> Option<String> {
    if !text.contains(RESOURCE_VERSION_REQUEST_PREFIX) {
        return None;
    }
    let message: Value = serde_json::from_str(text).ok()?;
    if message.get("method").is_some() {
        return None;
    }
    let id = message.get("id")?.as_str()?;
    id.starts_with(RESOURCE_VERSION_REQUEST_PREFIX)
        .then(|| id.to_owned())
}
