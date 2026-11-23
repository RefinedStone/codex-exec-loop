//! Host-owned extension admission, activation and durable job lifetime.
mod inbound;
mod jobs;
pub(crate) mod registry;
mod retention;
#[cfg(test)]
mod tests;

use crate::application::port::outbound::extension_state_store_port::ExtensionStateStorePort;
use crate::domain::kernel_extension::*;
use anyhow::{Context, Result, ensure};
use registry::{ExtensionRegistry, RegisteredExtension};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

const MAX_PAYLOAD: usize = 1_048_576;
const MAX_JOBS: usize = 128;
const MAX_RECEIPTS: usize = 512;
const MAX_ACTIVE_JOBS: usize = 8;

pub(crate) struct ExtensionJobContext {
    pub context: ExtensionContext,
    pub job_id: String,
    cancel: Arc<AtomicBool>,
    host: Weak<ExtensionService>,
}
impl ExtensionJobContext {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }
    pub fn report_progress(&self, progress: Value) -> Result<()> {
        ensure!(!self.cancelled(), "extension job was cancelled");
        self.host
            .upgrade()
            .context("extension host stopped")?
            .progress(&self.job_id, progress)
    }
}

struct RuntimeState {
    durable: ExtensionDurableState,
    cancellations: BTreeMap<String, Arc<AtomicBool>>,
    sequence: u64,
    cleanup_claims: BTreeSet<String>,
    stopped: bool,
}
pub(crate) struct ExtensionService {
    registry: ExtensionRegistry,
    store: Arc<dyn ExtensionStateStorePort>,
    scope: ExtensionScope,
    permissions: BTreeSet<String>,
    host_id: String,
    generation: String,
    state: Mutex<RuntimeState>,
    changed: broadcast::Sender<ExtensionEvent>,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ConfigureInput {
    enabled: bool,
    expected_revision: u64,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct JobInput {
    job_id: String,
}
#[derive(serde::Serialize, schemars::JsonSchema)]
struct JobsResponse {
    jobs: Vec<ExtensionJob>,
}

impl ExtensionService {
    pub fn new(
        registry: ExtensionRegistry,
        store: Arc<dyn ExtensionStateStorePort>,
        scope: ExtensionScope,
        host_id: Option<String>,
        permissions: BTreeSet<String>,
    ) -> Result<Arc<Self>> {
        let mut durable = store.load()?;
        if durable.host_id.is_empty() {
            durable.host_id = host_id
                .clone()
                .unwrap_or_else(|| format!("local-user-{}", random_id()));
        }
        if let Some(expected) = host_id {
            ensure!(
                expected == durable.host_id,
                "extension store belongs to another host"
            );
        }
        for extension in registry.entries() {
            durable
                .activations
                .entry(extension.manifest.id.clone())
                .or_default();
        }
        for job in durable.jobs.values_mut().filter(|job| job.state.active()) {
            job.state = ExtensionJobState::Interrupted;
            job.error = Some("host restarted before the worker completed".into());
            job.revision += 1;
            job.updated_at_ms = now_ms();
        }
        store.save(&durable)?;
        let service = Arc::new(Self {
            registry,
            store,
            scope,
            permissions,
            host_id: durable.host_id.clone(),
            generation: random_id(),
            state: Mutex::new(RuntimeState {
                durable,
                cancellations: BTreeMap::new(),
                sequence: 0,
                cleanup_claims: BTreeSet::new(),
                stopped: false,
            }),
            changed: broadcast::channel(64).0,
        });
        service.recover_cleanup()?;
        service.reap_expired()?;
        service.spawn_retention_worker()?;
        Ok(service)
    }
    pub fn host_id(&self) -> &str {
        &self.host_id
    }
    pub fn generation(&self) -> &str {
        &self.generation
    }
    pub fn scope(&self) -> ExtensionScope {
        self.scope
    }
    pub fn subscribe(&self) -> broadcast::Receiver<ExtensionEvent> {
        self.changed.subscribe()
    }
    pub fn busy(&self) -> bool {
        self.lock()
            .durable
            .jobs
            .values()
            .any(|job| job.state.active() || job.cleanup == ExtensionCleanupState::Failed)
    }
    pub fn shutdown(&self) {
        let mut state = self.lock();
        state.stopped = true;
        for cancel in state.cancellations.values() {
            cancel.store(true, Ordering::Release);
        }
    }
    pub fn context(
        &self,
        connection_id: String,
        repository_key: Option<String>,
    ) -> ExtensionContext {
        ExtensionContext {
            host_id: self.host_id.clone(),
            generation: self.generation.clone(),
            connection_id,
            repository_key,
            verified_thread: None,
            activation_revision: 0,
        }
    }
    pub fn catalog(&self) -> Vec<ExtensionCatalogEntry> {
        let state = self.lock();
        let mut catalog = self
            .registry
            .entries()
            .map(|entry| self.catalog_entry_locked(entry, &state))
            .collect::<Vec<_>>();
        for (manifest, errors) in self.registry.failures() {
            if self.registry.get(&manifest.id).is_ok() {
                continue;
            }
            catalog.push(ExtensionCatalogEntry {
                id: manifest.id.clone(),
                version: manifest.version,
                api_version: manifest.api_version,
                scope: manifest.scope,
                label: manifest.label.clone(),
                panel: manifest.panel.clone().unwrap_or_default(),
                permissions: manifest.permissions.clone(),
                actions: Vec::new(),
                operations: Vec::new(),
                compatible: false,
                compatibility_reason: Some("extension registration failed".into()),
                enabled: false,
                desired_enabled: false,
                lifecycle: ExtensionLifecycle::Error,
                revision: 0,
                generation: self.generation.clone(),
                sequence: state.sequence,
                registration_errors: errors.clone(),
                error: errors.first().cloned(),
            });
        }
        catalog
    }

    pub fn call(
        self: &Arc<Self>,
        mut context: ExtensionContext,
        extension: &str,
        version: u32,
        action: &str,
        input: Value,
        idempotency_key: Option<String>,
    ) -> Result<Value> {
        self.check_context(&context)?;
        ensure!(
            serde_json::to_vec(&input)?.len() <= MAX_PAYLOAD,
            "extension input is too large"
        );
        let entry = self.registry.get(extension)?;
        ensure!(
            entry.manifest.version == version,
            "unsupported extension version"
        );
        // Management and receipt observation remain available while disabled/incompatible.
        match action {
            "configure" => {
                return self.configure(&context, extension, serde_json::from_value(input)?);
            }
            "jobs" => {
                ensure!(input == json!({}), "jobs accepts an empty object");
                let state = self.lock();
                let mut jobs = state
                    .durable
                    .jobs
                    .values()
                    .filter(|job| job.extension == extension)
                    .cloned()
                    .collect::<Vec<_>>();
                jobs.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms).then(a.id.cmp(&b.id)));
                let activation = &state.durable.activations[extension];
                return serde_json::to_value(JobsResponse {
                    jobs: jobs
                        .into_iter()
                        .map(|job| visible_job(job, activation))
                        .collect(),
                })
                .map_err(Into::into);
            }
            "job" => {
                let input: JobInput = serde_json::from_value(input)?;
                let job = self.job(extension, &input.job_id)?;
                let activation = self.lock().durable.activations[extension].clone();
                return serde_json::to_value(visible_job(job, &activation)).map_err(Into::into);
            }
            "cancel" => {
                let input: JobInput = serde_json::from_value(input)?;
                return self.cancel_job(extension, &input.job_id);
            }
            _ => {}
        }
        let operation = entry
            .operations
            .get(action)
            .context("extension action is not registered")?;
        ensure!(
            operation.input.is_valid(&input),
            "extension input does not match its registered schema"
        );
        let revision = self.authorize(entry)?;
        context.activation_revision = revision;
        if operation.descriptor.kind == ExtensionOperationKind::Job {
            return self.start_job(context, entry, action, input, idempotency_key, revision);
        }
        // Dispatch after admission rechecks authority; completion must still belong to this revision.
        ensure!(
            self.authorize(entry)? == revision,
            "extension activation changed before dispatch"
        );
        let output = entry.handler.invoke(&context, action, input)?;
        self.check_context(&context)?;
        ensure!(
            self.authorize(entry)? == revision,
            "extension activation changed during dispatch"
        );
        validate_output(&operation.output, &output)?;
        Ok(output)
    }

    fn configure(
        self: &Arc<Self>,
        context: &ExtensionContext,
        id: &str,
        input: ConfigureInput,
    ) -> Result<Value> {
        let entry = self.registry.get(id)?;
        if input.enabled {
            self.check_permissions(entry)?;
        }
        {
            let mut state = self.lock();
            let current = &state.durable.activations[id];
            ensure!(
                current.revision == input.expected_revision,
                "extension activation revision changed"
            );
            ensure!(
                !input.enabled
                    || !matches!(
                        current.lifecycle,
                        ExtensionLifecycle::Disabling | ExtensionLifecycle::Error
                    ),
                "extension cleanup must finish before enabling"
            );
            if current.desired_enabled == input.enabled
                && matches!(
                    (input.enabled, current.lifecycle),
                    (true, ExtensionLifecycle::Enabled) | (false, ExtensionLifecycle::Disabled)
                )
            {
                return Ok(serde_json::to_value(
                    self.catalog_entry_locked(entry, &state),
                )?);
            }
            let mut next = state.durable.clone();
            let activation = next.activations.get_mut(id).expect("registered activation");
            activation.desired_enabled = input.enabled;
            activation.lifecycle = if input.enabled {
                ExtensionLifecycle::Enabled
            } else {
                ExtensionLifecycle::Disabling
            };
            activation.error = None;
            activation.revision += 1;
            if !input.enabled {
                for job in next
                    .jobs
                    .values_mut()
                    .filter(|job| job.extension == id && job.state.active())
                {
                    job.state = ExtensionJobState::Cancelling;
                    job.revision += 1;
                    job.sequence = state.sequence + 1;
                    job.updated_at_ms = now_ms();
                }
            }
            self.persist(&mut state, next)?;
            if !input.enabled {
                let ids = state
                    .durable
                    .jobs
                    .values()
                    .filter(|j| j.extension == id && j.state.active())
                    .map(|j| j.id.clone())
                    .collect::<Vec<_>>();
                for job in ids {
                    if let Some(flag) = state.cancellations.get(&job) {
                        flag.store(true, Ordering::Release);
                    }
                }
            }
            self.publish(&mut state, id, None);
        }
        if !input.enabled {
            let service = self.clone();
            let context = context.clone();
            let id = id.to_owned();
            std::thread::Builder::new()
                .name("akra-extension-cleanup".into())
                .spawn(move || {
                    if let Err(error) = service.cleanup_disabled(&context, &id) {
                        service.record_cleanup_error(&id, error.to_string());
                    }
                })?;
        }
        Ok(serde_json::to_value(
            self.catalog()
                .into_iter()
                .find(|e| e.id == id)
                .context("extension disappeared")?,
        )?)
    }
    fn catalog_entry_locked(
        &self,
        entry: &RegisteredExtension,
        state: &RuntimeState,
    ) -> ExtensionCatalogEntry {
        let activation = &state.durable.activations[&entry.manifest.id];
        let compatible = entry.manifest.api_version == EXTENSION_API_VERSION;
        let mut operations = entry.manifest.operations.clone();
        operations.extend(management_operations());
        ExtensionCatalogEntry {
            id: entry.manifest.id.clone(),
            version: entry.manifest.version,
            api_version: entry.manifest.api_version,
            scope: entry.manifest.scope,
            label: entry.manifest.label.clone(),
            panel: entry.manifest.panel.clone().unwrap_or_default(),
            permissions: entry.manifest.permissions.clone(),
            actions: operations.iter().map(|o| o.name.clone()).collect(),
            operations,
            compatible,
            compatibility_reason: (!compatible).then(|| "unsupported extension API version".into()),
            enabled: compatible && activation.lifecycle == ExtensionLifecycle::Enabled,
            desired_enabled: activation.desired_enabled,
            lifecycle: activation.lifecycle,
            revision: activation.revision,
            generation: self.generation.clone(),
            sequence: state.sequence,
            registration_errors: self.registry.errors(&entry.manifest.id),
            error: activation.error.clone(),
        }
    }
    fn check_context(&self, context: &ExtensionContext) -> Result<()> {
        ensure!(
            context.host_id == self.host_id && context.generation == self.generation,
            "extension host attachment is stale"
        );
        ensure!(
            !context.connection_id.is_empty(),
            "extension connection is unavailable"
        );
        ensure!(
            self.scope != ExtensionScope::Repository
                || context.repository_key.as_deref() == Some(self.host_id.as_str()),
            "extension repository attachment differs"
        );
        ensure!(!self.lock().stopped, "extension host is stopping");
        Ok(())
    }
    fn check_permissions(&self, entry: &RegisteredExtension) -> Result<()> {
        ensure!(
            entry.manifest.api_version == EXTENSION_API_VERSION,
            "incompatible extension API version"
        );
        ensure!(
            entry
                .manifest
                .permissions
                .iter()
                .all(|p| self.permissions.contains(p)),
            "extension permission is not granted"
        );
        Ok(())
    }
    fn authorize(&self, entry: &RegisteredExtension) -> Result<u64> {
        self.check_permissions(entry)?;
        let state = self.lock();
        ensure!(!state.stopped, "extension host is stopping");
        let activation = &state.durable.activations[&entry.manifest.id];
        ensure!(
            activation.desired_enabled && activation.lifecycle == ExtensionLifecycle::Enabled,
            "extension is disabled"
        );
        Ok(activation.revision)
    }
    fn lock(&self) -> MutexGuard<'_, RuntimeState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
    fn persist(&self, state: &mut RuntimeState, next: ExtensionDurableState) -> Result<()> {
        self.store.save(&next)?;
        state.durable = next;
        Ok(())
    }
    fn publish(&self, state: &mut RuntimeState, extension: &str, job_id: Option<String>) {
        state.sequence += 1;
        let revision = state
            .durable
            .activations
            .get(extension)
            .map_or(0, |a| a.revision);
        let _ = self.changed.send(ExtensionEvent {
            kernel: self.host_id.clone(),
            extension: extension.into(),
            generation: self.generation.clone(),
            revision,
            sequence: state.sequence,
            job_id,
        });
    }
    fn job(&self, extension: &str, id: &str) -> Result<ExtensionJob> {
        self.lock()
            .durable
            .jobs
            .get(id)
            .filter(|j| j.extension == extension)
            .cloned()
            .context("extension job is unavailable")
    }
}
pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
fn random_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}
fn validate_output(validator: &jsonschema::Validator, output: &Value) -> Result<()> {
    ensure!(
        serde_json::to_vec(output)?.len() <= MAX_PAYLOAD,
        "extension output is too large"
    );
    ensure!(
        validator.is_valid(output),
        "extension output does not match its registered schema"
    );
    Ok(())
}

fn management_operations() -> Vec<ExtensionOperation> {
    fn schema<T: schemars::JsonSchema>() -> Value {
        serde_json::to_value(schemars::schema_for!(T)).expect("schema")
    }
    let empty = json!({"type":"object","additionalProperties":false,"properties":{}});
    vec![
        ExtensionOperation {
            name: "configure".into(),
            kind: ExtensionOperationKind::Command,
            input_schema: schema::<ConfigureInput>(),
            output_schema: schema::<ExtensionCatalogEntry>(),
            progress_schema: None,
        },
        ExtensionOperation {
            name: "jobs".into(),
            kind: ExtensionOperationKind::Query,
            input_schema: empty,
            output_schema: schema::<JobsResponse>(),
            progress_schema: None,
        },
        ExtensionOperation {
            name: "job".into(),
            kind: ExtensionOperationKind::Query,
            input_schema: schema::<JobInput>(),
            output_schema: schema::<ExtensionJob>(),
            progress_schema: None,
        },
        ExtensionOperation {
            name: "cancel".into(),
            kind: ExtensionOperationKind::Command,
            input_schema: schema::<JobInput>(),
            output_schema: schema::<ExtensionJob>(),
            progress_schema: None,
        },
    ]
}
fn visible_job(mut job: ExtensionJob, activation: &ExtensionActivation) -> ExtensionJob {
    if !activation.desired_enabled || activation.lifecycle != ExtensionLifecycle::Enabled {
        job.progress = Value::Null;
        job.result = None;
    }
    job
}
