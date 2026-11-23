//! Explicit subtree sharing. Operator inspection and shared reads use one persistent index.
//! This is a query-service boundary, not an OS sandbox for an actor with local shell access.
mod model;
mod projection;
mod prompt;
mod response_guard;
pub(crate) use model::*;
pub(crate) use response_guard::StorageResponseGuard;

use std::collections::BTreeMap;
use std::sync::Mutex;

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::hmac;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::storage_analysis::StorageAnalysisService;
use crate::domain::storage_analysis::{StorageNode, StorageQuery};

const PREVIEW_TTL_MS: i64 = 5 * 60 * 1000;
const GRANT_TTL_MS: i64 = 60 * 60 * 1000;
const MAX_SHARES: usize = 256;
const MAX_PAGE_ITEMS: usize = 20;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const UNAVAILABLE: &str = "storage_share_unavailable";

#[derive(Clone)]
struct PreviewRecord {
    value: StorageSharePreview,
    caller: StorageShareCaller,
    grant_id: Option<String>,
}
#[derive(Clone)]
struct GrantRecord {
    value: StorageShareGrant,
    activation_generation: u64,
}
#[derive(Default)]
struct Store {
    epoch: u64,
    previews: BTreeMap<String, PreviewRecord>,
    grants: BTreeMap<String, GrantRecord>,
}
pub(crate) struct StorageSharingService {
    storage: StorageAnalysisService,
    store: Mutex<Store>,
    cursor_key: hmac::Key,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SharedCursor {
    binding: String,
    inner: String,
}

impl StorageSharingService {
    pub(crate) fn new(storage: StorageAnalysisService) -> Self {
        Self {
            storage,
            store: Mutex::new(Store::default()),
            cursor_key: hmac::Key::new(hmac::HMAC_SHA256, &rand::random::<[u8; 32]>()),
        }
    }

    pub(crate) fn preview(
        &self,
        caller: &StorageShareCaller,
        request: SharePreviewRequest,
    ) -> Result<StorageSharePreview> {
        caller.check()?;
        ensure!(request.thread_id == caller.thread_id, UNAVAILABLE);
        let share_epoch = self.store.lock().unwrap().epoch;
        let snapshot = self
            .storage
            .snapshot(&request.snapshot_id)
            .map_err(private_error)?;
        let node = self
            .storage
            .get_node(&request.snapshot_id, &request.node_id, None)
            .map_err(private_error)?;
        ensure!(snapshot.revision == node.revision, UNAVAILABLE);
        let (root, _) = projection::shared_node(node, &request.node_id);
        let value = StorageSharePreview {
            id: new_id(),
            snapshot_id: snapshot.id,
            revision: snapshot.revision,
            node_id: request.node_id,
            thread_id: caller.thread_id.clone(),
            root,
            shared_fields: [
                "선택 폴더와 하위 항목의 이름·노드 ID",
                "확인한 할당량·논리 크기·직속 부모 대비 비율",
                "일부 확인·누락 사유 코드",
                "worktree 보호 사유 코드·분류",
                "공유 루트의 상위 경로와 부모 비율은 제외",
                "파일 본문과 절대 경로는 제외",
                "이미 보낸 대화 내용은 공유 해제 뒤에도 보존",
            ]
            .map(str::to_owned)
            .to_vec(),
            expires_at_ms: now_ms() + PREVIEW_TTL_MS,
        };
        caller.check()?;
        let mut store = self.store.lock().unwrap();
        purge(&mut store);
        ensure!(store.epoch == share_epoch, UNAVAILABLE);
        ensure!(store.previews.len() < MAX_SHARES, "storage_share_limit");
        store.previews.insert(
            value.id.clone(),
            PreviewRecord {
                value: value.clone(),
                caller: caller.clone(),
                grant_id: None,
            },
        );
        Ok(value)
    }

    pub(crate) fn create(
        &self,
        caller: &StorageShareCaller,
        request: ShareCreateRequest,
    ) -> Result<StorageShareGrant> {
        caller.check()?;
        ensure!(request.thread_id == caller.thread_id, UNAVAILABLE);
        let mut store = self.store.lock().unwrap();
        purge(&mut store);
        let preview = store
            .previews
            .get(&request.preview_id)
            .cloned()
            .context(UNAVAILABLE)?;
        ensure!(
            preview.caller.host_id == caller.host_id
                && preview.caller.thread_id == caller.thread_id
                && preview.caller.connection_id == caller.connection_id
                && preview.caller.conversation_epoch == caller.conversation_epoch
                && preview.caller.activation_generation == caller.activation_generation,
            UNAVAILABLE,
        );
        if let Some(id) = preview.grant_id {
            return store
                .grants
                .get(&id)
                .map(|record| record.value.clone())
                .context(UNAVAILABLE);
        }
        ensure!(store.grants.len() < MAX_SHARES, "storage_share_limit");
        let current = self
            .storage
            .snapshot(&preview.value.snapshot_id)
            .map_err(private_error)?;
        ensure!(current.revision == preview.value.revision, UNAVAILABLE);
        let mut grant = StorageShareGrant {
            id: new_id(),
            host_id: caller.host_id.clone(),
            thread_id: caller.thread_id.clone(),
            snapshot_id: preview.value.snapshot_id.clone(),
            revision: preview.value.revision,
            root_node_id: preview.value.node_id.clone(),
            expires_at_ms: now_ms() + GRANT_TTL_MS,
            reference: String::new(),
            prompt: String::new(),
        };
        grant.reference = evidence(&grant, &grant.root_node_id)?;
        grant.prompt = prompt::render(&grant);
        caller.check()?;
        store.grants.insert(
            grant.id.clone(),
            GrantRecord {
                value: grant.clone(),
                activation_generation: caller.activation_generation,
            },
        );
        store
            .previews
            .get_mut(&request.preview_id)
            .unwrap()
            .grant_id = Some(grant.id.clone());
        Ok(grant)
    }

    pub(crate) fn revoke(&self, caller: &StorageShareCaller, grant_id: &str) -> Result<()> {
        caller.check()?;
        let mut store = self.store.lock().unwrap();
        if let Some(grant) = store.grants.get(grant_id) {
            ensure!(
                grant.value.host_id == caller.host_id && grant.value.thread_id == caller.thread_id,
                UNAVAILABLE
            );
        }
        store.grants.remove(grant_id);
        Ok(())
    }

    /// Called before a new scan is admitted and after disable/cleanup. Existing conversation text
    /// cannot be recalled, but every later query/reference resolution fails closed.
    pub(crate) fn invalidate(&self) {
        let mut store = self.store.lock().unwrap();
        store.epoch = store.epoch.wrapping_add(1);
        store.previews.clear();
        store.grants.clear();
    }

    /// Retention of one old job must not revoke a newer snapshot\'s shares.
    pub(crate) fn invalidate_snapshot(&self, snapshot_id: &str) {
        let mut store = self.store.lock().unwrap();
        store.epoch = store.epoch.wrapping_add(1);
        store
            .previews
            .retain(|_, record| record.value.snapshot_id != snapshot_id);
        store
            .grants
            .retain(|_, record| record.value.snapshot_id != snapshot_id);
    }

    pub(crate) fn get_node(
        &self,
        caller: &StorageShareCaller,
        request: SharedNodeRequest,
    ) -> Result<SharedNodeResponse> {
        let grant = self.authorize(caller, &request.grant_id)?;
        let node = self
            .storage
            .get_node(
                &grant.value.snapshot_id,
                &request.node_id,
                Some(&grant.value.root_node_id),
            )
            .map_err(private_error)?;
        let answer = shared_answer(&grant.value, node)?;
        check_response(&answer)?;
        self.finish(caller, &grant)?;
        Ok(answer)
    }

    pub(crate) fn list_children(
        &self,
        caller: &StorageShareCaller,
        request: SharedQueryRequest,
    ) -> Result<SharedPageResponse> {
        self.page(caller, request, false)
    }

    pub(crate) fn find_hotspots(
        &self,
        caller: &StorageShareCaller,
        request: SharedQueryRequest,
    ) -> Result<SharedPageResponse> {
        self.page(caller, request, true)
    }

    fn page(
        &self,
        caller: &StorageShareCaller,
        request: SharedQueryRequest,
        hotspots: bool,
    ) -> Result<SharedPageResponse> {
        ensure!(
            (1..=100).contains(&request.limit),
            "storage_invalid_page_size"
        );
        let grant = self.authorize(caller, &request.grant_id)?;
        ensure!(
            self.storage
                .is_descendant(
                    &grant.value.snapshot_id,
                    &request.node_id,
                    &grant.value.root_node_id
                )
                .unwrap_or(false),
            UNAVAILABLE
        );
        let binding = cursor_binding(&grant.value, &request, hotspots)?;
        let cursor = request
            .cursor
            .as_deref()
            .map(|cursor| self.decode_cursor(cursor, &binding))
            .transpose()?;
        let query = StorageQuery {
            snapshot_id: grant.value.snapshot_id.clone(),
            node_id: request.node_id.clone(),
            limit: request.limit.min(MAX_PAGE_ITEMS),
            cursor,
            sort: request.sort,
            name_filter: request.name_filter,
            extension_filter: None,
            recursive: false,
            scope_root_id: Some(grant.value.root_node_id.clone()),
        };
        // Scope is part of the database query, before sorting, aggregation and pagination.
        let (revision, nodes, total_count, next_cursor) = if hotspots {
            let page = self
                .storage
                .find_hotspots(&query, &request.thresholds.unwrap_or_default())
                .map_err(private_error)?;
            (
                page.revision,
                page.items
                    .into_iter()
                    .map(|row| {
                        (
                            row.node,
                            Some(SharedHotspotContribution {
                                reason: row.reason,
                                included_bytes: row.included_bytes,
                                role: row.role,
                                provisional: row.provisional,
                            }),
                        )
                    })
                    .collect::<Vec<_>>(),
                page.total_count,
                page.next_cursor,
            )
        } else {
            let page = self.storage.list_children(&query).map_err(private_error)?;
            (
                page.revision,
                page.items.into_iter().map(|node| (node, None)).collect(),
                page.total_count,
                page.next_cursor,
            )
        };
        ensure!(revision == grant.value.revision, UNAVAILABLE);
        let response = SharedPageResponse {
            schema_version: 1,
            grant_id: grant.value.id.clone(),
            snapshot_id: grant.value.snapshot_id.clone(),
            revision,
            items: nodes
                .into_iter()
                .map(|(node, hotspot)| {
                    let mut answer = shared_answer(&grant.value, node)?;
                    answer.hotspot = hotspot;
                    Ok(answer)
                })
                .collect::<Result<_>>()?,
            total_count,
            next_cursor: next_cursor
                .map(|inner| self.encode_cursor(&binding, inner))
                .transpose()?,
            response_limited: request.limit > MAX_PAGE_ITEMS,
            requested_limit: request.limit,
        };
        check_response(&response)?;
        self.finish(caller, &grant)?;
        Ok(response)
    }

    pub(crate) fn resolve(
        &self,
        caller: &StorageShareCaller,
        request: ResolveEvidenceRequest,
    ) -> Result<StorageEvidenceTarget> {
        caller.check()?;
        ensure!(request.thread_id == caller.thread_id, UNAVAILABLE);
        let reference = parse_evidence(&request.reference)?;
        let grant = self.authorize(caller, &reference.grant_id)?;
        ensure!(
            reference.version == 1
                && reference.host_id == grant.value.host_id
                && reference.thread_id == grant.value.thread_id
                && reference.snapshot_id == grant.value.snapshot_id
                && reference.revision == grant.value.revision,
            UNAVAILABLE
        );
        let root = Some(grant.value.root_node_id.as_str());
        let node = self
            .storage
            .get_node(&reference.snapshot_id, &reference.node_id, root)
            .map_err(private_error)?;
        let ancestors = self
            .storage
            .ancestors(&reference.snapshot_id, &reference.node_id, root)
            .map_err(private_error)?
            .into_iter()
            .map(|node| projection::shared_node(node, &grant.value.root_node_id).0)
            .collect();
        let answer = StorageEvidenceTarget {
            snapshot_id: grant.value.snapshot_id.clone(),
            revision: grant.value.revision,
            node_id: node.id.clone(),
            ancestors,
            node: projection::shared_node(node, &grant.value.root_node_id).0,
        };
        check_response(&answer)?;
        self.finish(caller, &grant)?;
        Ok(answer)
    }

    fn authorize(&self, caller: &StorageShareCaller, id: &str) -> Result<GrantRecord> {
        caller.check()?;
        let store = self.store.lock().unwrap();
        let grant = store.grants.get(id).context(UNAVAILABLE)?;
        ensure!(
            grant.value.host_id == caller.host_id
                && grant.value.thread_id == caller.thread_id
                && grant.activation_generation == caller.activation_generation
                && grant.value.expires_at_ms > now_ms(),
            UNAVAILABLE
        );
        caller.check()?;
        Ok(grant.clone())
    }

    fn finish(&self, caller: &StorageShareCaller, grant: &GrantRecord) -> Result<()> {
        let snapshot = self
            .storage
            .snapshot(&grant.value.snapshot_id)
            .map_err(private_error)?;
        ensure!(snapshot.revision == grant.value.revision, UNAVAILABLE);
        // No blocking I/O or output serialization may follow this final grant/epoch check.
        let current = self.authorize(caller, &grant.value.id)?;
        ensure!(
            current.value.snapshot_id == grant.value.snapshot_id
                && current.value.revision == grant.value.revision,
            UNAVAILABLE,
        );
        caller.check()
    }

    fn encode_cursor(&self, binding: &str, inner: String) -> Result<String> {
        let bytes = serde_json::to_vec(&SharedCursor {
            binding: binding.into(),
            inner,
        })?;
        let signature = hmac::sign(&self.cursor_key, &bytes);
        Ok(format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(&bytes),
            URL_SAFE_NO_PAD.encode(signature.as_ref())
        ))
    }

    fn decode_cursor(&self, cursor: &str, binding: &str) -> Result<String> {
        ensure!(cursor.len() <= 8192, "storage_invalid_cursor");
        let (payload, signature) = cursor.split_once('.').context("storage_invalid_cursor")?;
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| anyhow::anyhow!("storage_invalid_cursor"))?;
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| anyhow::anyhow!("storage_invalid_cursor"))?;
        hmac::verify(&self.cursor_key, &bytes, &signature)
            .map_err(|_| anyhow::anyhow!("storage_invalid_cursor"))?;
        let parsed: SharedCursor = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("storage_invalid_cursor"))?;
        ensure!(parsed.binding == binding, "storage_invalid_cursor");
        Ok(parsed.inner)
    }
}

fn private_error(_: anyhow::Error) -> anyhow::Error {
    anyhow::anyhow!(UNAVAILABLE)
}
fn new_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
fn purge(store: &mut Store) {
    let now = now_ms();
    store
        .previews
        .retain(|_, record| record.value.expires_at_ms > now);
    store
        .grants
        .retain(|_, record| record.value.expires_at_ms > now);
}
fn shared_answer(grant: &StorageShareGrant, node: StorageNode) -> Result<SharedNodeResponse> {
    ensure!(
        node.snapshot_id == grant.snapshot_id && node.revision == grant.revision,
        UNAVAILABLE
    );
    let (node, omitted_fields) = projection::shared_node(node, &grant.root_node_id);
    Ok(SharedNodeResponse {
        schema_version: 1,
        grant_id: grant.id.clone(),
        snapshot_id: grant.snapshot_id.clone(),
        revision: grant.revision,
        reference: evidence(grant, &node.id)?,
        node,
        omitted_fields,
        hotspot: None,
    })
}
fn evidence(grant: &StorageShareGrant, node_id: &str) -> Result<String> {
    Ok(format!(
        "akra-storage:{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&EvidenceReference {
            version: 1,
            host_id: grant.host_id.clone(),
            thread_id: grant.thread_id.clone(),
            grant_id: grant.id.clone(),
            snapshot_id: grant.snapshot_id.clone(),
            revision: grant.revision,
            node_id: node_id.to_owned(),
        })?)
    ))
}
fn parse_evidence(reference: &str) -> Result<EvidenceReference> {
    ensure!(reference.len() <= 4096, UNAVAILABLE);
    let encoded = reference
        .strip_prefix("akra-storage:")
        .context(UNAVAILABLE)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| anyhow::anyhow!(UNAVAILABLE))?;
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!(UNAVAILABLE))
}
fn cursor_binding(
    grant: &StorageShareGrant,
    request: &SharedQueryRequest,
    hotspots: bool,
) -> Result<String> {
    let values = serde_json::to_vec(&(
        &grant.id,
        &grant.host_id,
        &grant.thread_id,
        &grant.snapshot_id,
        grant.revision,
        &grant.root_node_id,
        &request.node_id,
        &request.sort,
        &request.name_filter,
        &request.thresholds,
        hotspots,
    ))?;
    Ok(format!("{:x}", Sha256::digest(values)))
}
fn check_response(value: &impl Serialize) -> Result<()> {
    if serde_json::to_vec(value)?.len() > MAX_RESPONSE_BYTES {
        bail!("storage_response_too_large: use a smaller page or a deeper shared folder");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
