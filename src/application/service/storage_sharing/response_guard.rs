//! Final response authority is held across the host's activation commit check.
use super::*;
use serde_json::Value;
use std::sync::MutexGuard;

pub(crate) struct StorageResponseGuard<'a> {
    _store: MutexGuard<'a, Store>,
    caller: StorageShareCaller,
    expires_at_ms: i64,
}
impl StorageResponseGuard<'_> {
    /// No I/O or host reentry: called while both share and host authority locks are held.
    pub(crate) fn validate(&self) -> Result<()> {
        self.caller.check()?;
        ensure!(self.expires_at_ms > now_ms(), UNAVAILABLE);
        Ok(())
    }
}
impl StorageSharingService {
    pub(crate) fn response_guard<'a>(
        &'a self,
        caller: &StorageShareCaller,
        action: &str,
        input: &Value,
        output: &Value,
    ) -> Result<Option<StorageResponseGuard<'a>>> {
        let (grant_id, preview_id) = match action {
            "share_preview" => (
                None,
                Some(output["id"].as_str().context(UNAVAILABLE)?.to_owned()),
            ),
            "share_create" => (
                Some(output["id"].as_str().context(UNAVAILABLE)?.to_owned()),
                None,
            ),
            "shared_get_node" | "shared_list_children" | "shared_find_hotspots" => (
                Some(input["grant_id"].as_str().context(UNAVAILABLE)?.to_owned()),
                None,
            ),
            "resolve_evidence" => (
                Some(parse_evidence(input["reference"].as_str().context(UNAVAILABLE)?)?.grant_id),
                None,
            ),
            "share_revoke" => (None, None),
            _ => return Ok(None),
        };
        caller.check()?;
        let store = self.store.lock().unwrap();
        let expires_at_ms = if let Some(id) = grant_id {
            let grant = store.grants.get(&id).context(UNAVAILABLE)?;
            ensure!(
                grant.value.host_id == caller.host_id
                    && grant.value.thread_id == caller.thread_id
                    && grant.activation_generation == caller.activation_generation
                    && output["snapshot_id"].as_str() == Some(&grant.value.snapshot_id)
                    && output["revision"].as_u64() == Some(grant.value.revision),
                UNAVAILABLE
            );
            grant.value.expires_at_ms
        } else if let Some(id) = preview_id {
            let preview = store.previews.get(&id).context(UNAVAILABLE)?;
            ensure!(
                preview.caller.host_id == caller.host_id
                    && preview.caller.thread_id == caller.thread_id
                    && preview.caller.connection_id == caller.connection_id
                    && preview.caller.conversation_epoch == caller.conversation_epoch
                    && preview.caller.activation_generation == caller.activation_generation,
                UNAVAILABLE
            );
            preview.value.expires_at_ms
        } else {
            i64::MAX
        };
        Ok(Some(StorageResponseGuard {
            _store: store,
            caller: caller.clone(),
            expires_at_ms,
        }))
    }
}
