use super::{ExtensionContext, ExtensionJobContext};
use crate::domain::kernel_extension::{
    ExtensionDescriptor, ExtensionJob, ExtensionOperation, ExtensionScope,
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// A short, non-I/O authority check held across the host's response commit.
/// Implementations may retain their own permission-store lock. Acquiring the guard must not
/// hold the host lock; validate must not perform I/O or reenter the host.
pub(crate) trait ExtensionResponseGuard {
    fn validate(&self) -> Result<()>;
}

pub(crate) trait ExtensionHandler: Send + Sync {
    fn operations(&self) -> &'static [&'static str];
    fn invoke(&self, _context: &ExtensionContext, _action: &str, _input: Value) -> Result<Value> {
        bail!("extension operation is not an immediate operation")
    }
    fn run(&self, _context: &ExtensionJobContext, _action: &str, _input: Value) -> Result<Value> {
        bail!("extension operation is not a background job")
    }
    /// Called after output-schema validation and before the host takes its state lock.
    /// The lock order is the handler's authority store, then the host. The returned guard is
    /// dropped after the host lock is released, on both success and rejection.
    fn response_guard<'a>(
        &'a self,
        _context: &ExtensionContext,
        _action: &str,
        _input: &Value,
        _output: &Value,
    ) -> Result<Option<Box<dyn ExtensionResponseGuard + 'a>>> {
        Ok(None)
    }
    fn dedupe_key(&self, _action: &str, _input: &Value) -> Result<Option<String>> {
        Ok(None)
    }
    /// Bounded response budget for this package; progress keeps the host's smaller limit.
    fn max_output_bytes(&self) -> usize {
        super::MAX_PAYLOAD
    }
    /// Upper bound of this extension's active jobs, below the host-wide bound.
    fn max_active_jobs(&self) -> Option<usize> {
        None
    }
    /// Remove only artifacts belonging to this exact job ID and generation.
    fn cleanup(&self, _context: &ExtensionContext, _job: &ExtensionJob) -> Result<()> {
        Ok(())
    }
}
pub(crate) struct ExtensionRegistration {
    pub manifest: ExtensionDescriptor,
    pub handler: Arc<dyn ExtensionHandler>,
}
pub(crate) struct RegisteredOperation {
    pub descriptor: ExtensionOperation,
    pub input: jsonschema::Validator,
    pub output: jsonschema::Validator,
    pub progress: Option<jsonschema::Validator>,
}
pub(crate) struct RegisteredExtension {
    pub manifest: ExtensionDescriptor,
    pub handler: Arc<dyn ExtensionHandler>,
    pub operations: BTreeMap<String, RegisteredOperation>,
}
pub(crate) struct ExtensionRegistry {
    entries: BTreeMap<String, RegisteredExtension>,
    failures: BTreeMap<String, (ExtensionDescriptor, Vec<String>)>,
}
impl ExtensionRegistry {
    /// A package registers atomically. A bad package never disables another valid package.
    pub fn build(registrations: Vec<ExtensionRegistration>, scope: ExtensionScope) -> Result<Self> {
        ensure!(
            registrations.len() <= 128,
            "extension package limit exceeded"
        );
        let mut registry = Self {
            entries: BTreeMap::new(),
            failures: BTreeMap::new(),
        };
        for registration in registrations {
            let manifest = registration.manifest.clone();
            let result = if registry.entries.contains_key(&manifest.id) {
                Err(anyhow::anyhow!(
                    "duplicate extension ID; existing package retained"
                ))
            } else {
                compile_registration(registration, scope)
            };
            match result {
                Ok(entry) => {
                    registry.entries.insert(entry.manifest.id.clone(), entry);
                }
                Err(error) => {
                    registry
                        .failures
                        .entry(manifest.id.clone())
                        .or_insert_with(|| (manifest, Vec::new()))
                        .1
                        .push(error.to_string());
                }
            }
        }
        Ok(registry)
    }
    pub fn get(&self, id: &str) -> Result<&RegisteredExtension> {
        self.entries
            .get(id)
            .context("extension is not installed or registration failed")
    }
    pub fn entries(&self) -> impl Iterator<Item = &RegisteredExtension> {
        self.entries.values()
    }
    pub fn failures(&self) -> impl Iterator<Item = &(ExtensionDescriptor, Vec<String>)> {
        self.failures.values()
    }
    pub fn errors(&self, id: &str) -> Vec<String> {
        self.failures
            .get(id)
            .map_or_else(Vec::new, |(_, e)| e.clone())
    }
}
fn compile_registration(
    registration: ExtensionRegistration,
    scope: ExtensionScope,
) -> Result<RegisteredExtension> {
    let manifest = registration.manifest;
    ensure!(valid_id(&manifest.id), "invalid extension ID");
    ensure!(
        manifest.version > 0 && manifest.api_version > 0,
        "invalid extension version"
    );
    ensure!(
        manifest.scope == scope,
        "extension scope does not match host"
    );
    ensure!(
        !manifest.operations.is_empty() && manifest.operations.len() <= 64,
        "invalid operation count"
    );
    let mut operations = BTreeMap::new();
    for operation in &manifest.operations {
        ensure!(
            valid_action(&operation.name),
            "invalid or reserved extension action"
        );
        ensure!(
            !operations.contains_key(&operation.name),
            "duplicate extension action"
        );
        operations.insert(
            operation.name.clone(),
            RegisteredOperation {
                descriptor: operation.clone(),
                input: compile(&operation.input_schema)?,
                output: compile(&operation.output_schema)?,
                progress: operation
                    .progress_schema
                    .as_ref()
                    .map(compile)
                    .transpose()?,
            },
        );
    }
    let declared = operations
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let implemented = registration
        .handler
        .operations()
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    ensure!(
        declared == implemented,
        "declared actions and handler actions differ"
    );
    ensure!(
        implemented.len() == registration.handler.operations().len(),
        "duplicate handler action"
    );
    Ok(RegisteredExtension {
        manifest,
        handler: registration.handler,
        operations,
    })
}
fn valid_id(id: &str) -> bool {
    (3..=128).contains(&id.len())
        && id.contains('.')
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'-'))
}
fn valid_action(action: &str) -> bool {
    !action.is_empty()
        && action.len() <= 64
        && action
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        && !matches!(action, "configure" | "jobs" | "job" | "cancel" | "catalog")
}
fn compile(schema: &Value) -> Result<jsonschema::Validator> {
    ensure!(
        serde_json::to_vec(schema)?.len() <= 262_144,
        "schema is too large"
    );
    check_references(schema)?;
    jsonschema::validator_for(schema).map_err(|e| anyhow::anyhow!("invalid extension schema: {e}"))
}
fn check_references(value: &Value) -> Result<()> {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(key.as_str(), "$ref" | "$dynamicRef") {
                    ensure!(
                        value.as_str().is_some_and(|v| v.starts_with('#')),
                        "external schema references are not allowed"
                    );
                }
                check_references(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                check_references(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}
