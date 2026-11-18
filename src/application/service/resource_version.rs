//! Session-owned Desktop providers. Credentials never enter this service.

use std::collections::BTreeMap;
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use tokio::sync::mpsc as async_mpsc;

use crate::application::port::inbound::resource_version_provider_port::{
    RESOURCE_VERSION_REQUEST_PREFIX, ResourceVersionProviderPort, ResourceVersionRequest,
};
use crate::application::port::outbound::external_resource_version_port::ExternalResourceVersionPort;
use crate::domain::work_context::{ExternalResourceRef, jira_resource_site, normalize_jira_site};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

struct Provider {
    site: Option<String>,
    requests: async_mpsc::Sender<ResourceVersionRequest>,
}

struct Pending {
    session: u64,
    answer: mpsc::SyncSender<Result<String>>,
}

#[derive(Default)]
struct RegistryState {
    next_id: u64,
    providers: BTreeMap<u64, Provider>,
    pending: BTreeMap<String, Pending>,
}

#[derive(Default)]
pub struct ResourceVersionService {
    state: Mutex<RegistryState>,
}

impl ResourceVersionProviderPort for ResourceVersionService {
    fn open(&self) -> Result<(u64, async_mpsc::Receiver<ResourceVersionRequest>)> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("provider lock poisoned"))?;
        state.next_id = state
            .next_id
            .checked_add(1)
            .ok_or_else(|| anyhow!("provider ids exhausted"))?;
        let id = state.next_id;
        let (requests, receiver) = async_mpsc::channel(16);
        state.providers.insert(
            id,
            Provider {
                site: None,
                requests,
            },
        );
        Ok((id, receiver))
    }

    fn register(&self, session: u64, site: Option<&str>) -> Result<()> {
        let site = site
            .map(|site| normalize_jira_site(site).ok_or_else(|| anyhow!("invalid Jira site")))
            .transpose()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("provider lock poisoned"))?;
        let provider = state
            .providers
            .get_mut(&session)
            .ok_or_else(|| anyhow!("provider disconnected"))?;
        provider.site = site;
        state
            .pending
            .retain(|_, pending| pending.session != session);
        Ok(())
    }

    fn close(&self, session: u64) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.providers.remove(&session);
        state
            .pending
            .retain(|_, pending| pending.session != session);
    }

    fn answer(&self, session: u64, id: &str, answer: Result<String>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .pending
            .get(id)
            .is_some_and(|pending| pending.session == session)
            && let Some(pending) = state.pending.remove(id)
        {
            // A timed-out caller has already dropped its receiver.
            let _ = pending.answer.send(answer);
        }
    }
}

impl ResourceVersionService {
    fn lookup(&self, resource: &ExternalResourceRef, timeout: Duration) -> Result<String> {
        let site = jira_resource_site(resource).ok_or_else(|| anyhow!("invalid Jira resource"))?;
        let (answer, receiver) = mpsc::sync_channel(1);
        let id = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow!("provider lock poisoned"))?;
            let Some((&session, provider)) = state.providers.iter().find(|(_, provider)| {
                provider.site.as_deref() == Some(site.as_str()) && !provider.requests.is_closed()
            }) else {
                bail!("no Desktop is connected to this Jira site")
            };
            let requests = provider.requests.clone();
            state.next_id = state
                .next_id
                .checked_add(1)
                .ok_or_else(|| anyhow!("provider ids exhausted"))?;
            let id = format!("{RESOURCE_VERSION_REQUEST_PREFIX}{}", state.next_id);
            requests
                .try_send(ResourceVersionRequest {
                    id: id.clone(),
                    resource: resource.clone(),
                })
                .map_err(|_| anyhow!("Desktop provider is unavailable"))?;
            state
                .pending
                .insert(id.clone(), Pending { session, answer });
            id
        };
        let result = receiver
            .recv_timeout(timeout)
            .map_err(|_| anyhow!("Desktop version request ended without an answer"));
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending
            .remove(&id);
        let version = result??;
        if version.len() > 64 || chrono::DateTime::parse_from_rfc3339(&version).is_err() {
            bail!("Desktop returned an invalid update time")
        }
        Ok(version)
    }
}

impl ExternalResourceVersionPort for ResourceVersionService {
    fn kind(&self) -> &'static str {
        "jira"
    }

    fn available(&self, resource: &ExternalResourceRef) -> bool {
        let Some(site) = jira_resource_site(resource) else {
            return false;
        };
        self.state.lock().is_ok_and(|state| {
            state.providers.values().any(|provider| {
                provider.site.as_deref() == Some(site.as_str()) && !provider.requests.is_closed()
            })
        })
    }

    fn current_version(&self, resource: &ExternalResourceRef) -> Result<String> {
        self.lookup(resource, REQUEST_TIMEOUT)
    }
}

#[cfg(test)]
mod tests;
