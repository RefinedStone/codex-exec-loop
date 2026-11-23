//! The only E2 addition to a worker sandbox is its host-created generation root.
//! A read-only or full-access execution policy is preserved verbatim.
use super::{ProtectedThreadWorkspace, protocol::SandboxModeValue};
use anyhow::{Context, Result, bail};
use std::path::Path;

use crate::domain::conversation_runtime_envelope::{
    ConversationRuntimeConfigurationRequest, ConversationRuntimeEnvelope,
    ConversationRuntimeObservedValue, ConversationRuntimeRequestedValue,
    ConversationRuntimeSandboxPolicy,
};
use crate::domain::planning::evidence::validate_evidence_token;
use crate::private_sqlite::PinnedPrivateDirectory;

#[cfg(test)]
mod tests;

pub(super) struct WorkerEvidenceRoot {
    directory: PinnedPrivateDirectory,
    value: String,
}

impl WorkerEvidenceRoot {
    pub(super) fn prepare(root: Option<&Path>, policy: SandboxModeValue) -> Result<Option<Self>> {
        if policy != SandboxModeValue::WorkspaceWrite {
            return Ok(None);
        }
        let Some(root) = root else {
            return Ok(None);
        };
        if !root.is_absolute() {
            bail!("worker evidence root must be absolute");
        }
        let generation = root
            .file_name()
            .and_then(|value| value.to_str())
            .context("worker evidence generation is missing")?;
        validate_evidence_token(generation)?;
        let incoming = root
            .parent()
            .context("worker evidence incoming parent is missing")?;
        let evidence = incoming
            .parent()
            .context("worker evidence parent is missing")?;
        if incoming.file_name() != Some(std::ffi::OsStr::new("incoming"))
            || evidence.file_name() != Some(std::ffi::OsStr::new("evidence"))
        {
            bail!("worker evidence root is not a fixed incoming generation directory");
        }
        let runtime = evidence
            .parent()
            .context("worker evidence runtime is missing")?;
        let directory = PinnedPrivateDirectory::open(runtime, false)?
            .child("evidence", false)?
            .child("incoming", false)?
            .child(generation, false)?;
        let value = root
            .to_str()
            .context("worker evidence root is not Unicode")?
            .to_string();
        Ok(Some(Self { directory, value }))
    }

    pub(super) fn value(&self) -> &str {
        &self.value
    }
    pub(super) fn verify(&self) -> Result<()> {
        self.directory.verify()
    }

    pub(super) fn apply_config(&self, workspace: &mut ProtectedThreadWorkspace) -> Result<()> {
        self.verify()?;
        workspace.config.insert(
            "sandbox_workspace_write".into(),
            serde_json::json!({"writable_roots": [&self.value]}),
        );
        Ok(())
    }

    pub(super) fn record_request(&self, request: &mut ConversationRuntimeConfigurationRequest) {
        let mut policy = SandboxModeValue::WorkspaceWrite.runtime_policy();
        if let ConversationRuntimeSandboxPolicy::WorkspaceWrite { writable_roots, .. } = &mut policy
        {
            *writable_roots = Some(vec![self.value.clone()]);
        }
        request.sandbox = ConversationRuntimeRequestedValue::Value(policy);
    }

    pub(super) fn verify_applied(
        &self,
        envelope: &ConversationRuntimeEnvelope,
        cwd: &str,
    ) -> Result<()> {
        self.verify()?;
        let policy = match &envelope.applied.sandbox {
            ConversationRuntimeObservedValue::Observed(policy)
            | ConversationRuntimeObservedValue::Defaulted(policy) => policy,
            _ => bail!("worker sandbox response omitted its required evidence root"),
        };
        let ConversationRuntimeSandboxPolicy::WorkspaceWrite {
            writable_roots: Some(roots),
            writable_roots_truncated: false,
            ..
        } = policy
        else {
            bail!("worker sandbox did not apply the bounded writable-root request");
        };
        if !roots.iter().any(|root| same_path(root, &self.value))
            || roots
                .iter()
                .any(|root| !same_path(root, &self.value) && !same_path(root, cwd))
        {
            bail!("worker sandbox applied a different set of writable roots");
        }
        self.verify()
    }
}

fn same_path(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}
