//! The storage package assembly. Its scanner, read-only queries and shared reads use one index.
mod schemas;
use crate::domain::kernel_extension::ExtensionContext;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::adapter::outbound::storage_analysis::{
    NativeStorageAnalysisAdapter, StorageRuntimeProbe,
};
use crate::application::service::kernel_extensions::{
    ExtensionJobContext,
    registry::{ExtensionHandler, ExtensionRegistration, ExtensionResponseGuard},
};
use crate::application::service::storage_analysis::StorageAnalysisService;
use crate::application::service::storage_sharing::*;
use crate::domain::kernel_extension::{ExtensionDescriptor, ExtensionJob, ExtensionScope};
use crate::domain::storage_analysis::*;

pub(crate) const STORAGE_EXTENSION_ID: &str = "akra.storage-analysis";

const ACTIONS: &[&str] = &[
    "volumes",
    "scan",
    "live_children",
    "prioritize",
    "snapshot",
    "get_node",
    "list_children",
    "list_files",
    "find_hotspots",
    "ancestors",
    "reveal_path",
    "share_preview",
    "share_create",
    "share_revoke",
    "shared_get_node",
    "shared_list_children",
    "shared_find_hotspots",
    "resolve_evidence",
];

pub(crate) fn registration(
    root: PathBuf,
    probe: Option<StorageRuntimeProbe>,
) -> ExtensionRegistration {
    let mut adapter = NativeStorageAnalysisAdapter::new(root);
    if let Some(probe) = probe {
        adapter = adapter.with_runtime_probe(probe);
    }
    registration_with(StorageAnalysisService::new(Arc::new(adapter)))
}

fn registration_with(storage: StorageAnalysisService) -> ExtensionRegistration {
    ExtensionRegistration {
        manifest: ExtensionDescriptor {
            id: STORAGE_EXTENSION_ID.into(),
            version: 1,
            api_version: 1,
            scope: ExtensionScope::LocalUser,
            label: "저장 공간 분석기".into(),
            panel: Some(STORAGE_EXTENSION_ID.into()),
            permissions: vec![
                "metadata.read".into(),
                "storage.read".into(),
                "git.read".into(),
            ],
            operations: schemas::operations(),
        },
        handler: Arc::new(StorageExtension {
            shares: StorageSharingService::new(storage.clone()),
            storage,
        }),
    }
}

struct StorageExtension {
    storage: StorageAnalysisService,
    shares: StorageSharingService,
}

impl ExtensionHandler for StorageExtension {
    fn operations(&self) -> &'static [&'static str] {
        ACTIONS
    }

    fn invoke(&self, context: &ExtensionContext, action: &str, input: Value) -> Result<Value> {
        match action {
            "volumes" => {
                let _: schemas::Empty = parse(input)?;
                Ok(json!({"volumes":self.storage.list_volumes()?}))
            }
            "live_children" => encode(
                self.storage
                    .live_children(&parse::<StorageLiveQuery>(input)?)?,
            ),
            "prioritize" => {
                let request: schemas::PriorityInput = parse(input)?;
                encode(self.storage.prioritize(&request.job_id, &request.node_id)?)
            }
            "snapshot" => {
                let request: schemas::SnapshotInput = parse(input)?;
                encode(self.storage.snapshot(&request.snapshot_id)?)
            }
            "get_node" => {
                let request: schemas::NodeInput = parse(input)?;
                encode(
                    self.storage
                        .get_node(&request.snapshot_id, &request.node_id, None)?,
                )
            }
            "list_children" | "list_files" | "find_hotspots" => {
                let request: schemas::BrowseInput = parse(input)?;
                let query = request.query();
                match action {
                    "list_children" => encode(self.storage.list_children(&query)?),
                    "list_files" => encode(self.storage.list_files(&query)?),
                    _ => encode(
                        self.storage
                            .find_hotspots(&query, &request.thresholds.unwrap_or_default())?,
                    ),
                }
            }
            "ancestors" => {
                let request: schemas::NodeInput = parse(input)?;
                Ok(
                    json!({"snapshot_id":request.snapshot_id,"node_id":request.node_id,
                    "ancestors":self.storage.ancestors(&request.snapshot_id,&request.node_id,None)?}),
                )
            }
            "reveal_path" => {
                let request: schemas::NodeInput = parse(input)?;
                let path = self
                    .storage
                    .reveal_path(&request.snapshot_id, &request.node_id)?;
                Ok(
                    json!({"path":path,"host_id":context.host_id,"snapshot_id":request.snapshot_id,
                    "node_id":request.node_id}),
                )
            }
            "share_preview" => encode(
                self.shares
                    .preview(&share_caller(context)?, parse(input)?)?,
            ),
            "share_create" => encode(self.shares.create(&share_caller(context)?, parse(input)?)?),
            "share_revoke" => {
                let request: ShareRevokeRequest = parse(input)?;
                self.shares
                    .revoke(&share_caller(context)?, &request.grant_id)?;
                Ok(json!({}))
            }
            "shared_get_node" => encode(
                self.shares
                    .get_node(&share_caller(context)?, parse(input)?)?,
            ),
            "shared_list_children" => encode(
                self.shares
                    .list_children(&share_caller(context)?, parse(input)?)?,
            ),
            "shared_find_hotspots" => encode(
                self.shares
                    .find_hotspots(&share_caller(context)?, parse(input)?)?,
            ),
            "resolve_evidence" => encode(
                self.shares
                    .resolve(&share_caller(context)?, parse(input)?)?,
            ),
            _ => bail!("storage_operation_unavailable"),
        }
    }

    fn response_guard<'a>(
        &'a self,
        context: &ExtensionContext,
        action: &str,
        input: &Value,
        output: &Value,
    ) -> Result<Option<Box<dyn ExtensionResponseGuard + 'a>>> {
        if !matches!(
            action,
            "share_preview"
                | "share_create"
                | "share_revoke"
                | "shared_get_node"
                | "shared_list_children"
                | "shared_find_hotspots"
                | "resolve_evidence"
        ) {
            return Ok(None);
        }
        Ok(self
            .shares
            .response_guard(&share_caller(context)?, action, input, output)?
            .map(|guard| Box::new(guard) as Box<dyn ExtensionResponseGuard>))
    }
    fn run(&self, context: &ExtensionJobContext, action: &str, input: Value) -> Result<Value> {
        if action != "scan" {
            bail!("storage_operation_unavailable");
        }
        let request: schemas::ScanInput = parse(input)?;
        // A new explicit scan creates a new observation. Existing references never silently
        // retarget similarly named rows in its replacement.
        self.shares.invalidate();
        let scan = ScanRequest {
            job_id: context.job_id.clone(),
            volume_id: request.volume_id,
            options: request.options,
        };
        let result = self
            .storage
            .scan(&scan, context.cancel_flag(), &|progress| {
                if let Ok(progress) = serde_json::to_value(progress) {
                    let _ = context.report_progress(progress);
                }
            })?;
        encode(result)
    }

    fn dedupe_key(&self, action: &str, input: &Value) -> Result<Option<String>> {
        if action != "scan" {
            return Ok(None);
        }
        let request: schemas::ScanInput = parse(input.clone())?;
        request.options.validate().map_err(anyhow::Error::msg)?;
        Ok(Some(serde_json::to_string(&(
            request.volume_id,
            request.options,
        ))?))
    }

    fn cleanup(&self, _: &ExtensionContext, job: &ExtensionJob) -> Result<()> {
        if job.extension != STORAGE_EXTENSION_ID {
            bail!("storage_cleanup_owner_mismatch");
        }
        self.shares.invalidate_snapshot(&job.id);
        self.storage.cleanup(&job.id)
    }
}

fn share_caller(context: &ExtensionContext) -> Result<StorageShareCaller> {
    let (epoch, current_epoch) = context
        .conversation_generation
        .as_ref()
        .context("storage_share_unavailable")?;
    Ok(StorageShareCaller {
        host_id: context.host_id.clone(),
        activation_generation: context.activation_revision,
        connection_id: context.connection_id.clone(),
        thread_id: context
            .verified_thread
            .clone()
            .context("storage_share_unavailable")?,
        conversation_epoch: *epoch,
        current_epoch: current_epoch.clone(),
    })
}

fn parse<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| anyhow::anyhow!("storage_invalid_input"))
}
fn encode(value: impl serde::Serialize) -> Result<Value> {
    Ok(serde_json::to_value(value)?)
}

impl ExtensionResponseGuard for StorageResponseGuard<'_> {
    fn validate(&self) -> Result<()> {
        StorageResponseGuard::validate(self)
    }
}
