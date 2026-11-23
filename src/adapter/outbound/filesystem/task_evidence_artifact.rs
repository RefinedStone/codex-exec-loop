//! Fixed-path, bounded capture of worker output. The sealed copy is host write-once;
//! it is not a claim that a separate process with the same OS account cannot modify it.
//! Every later use reopens and hashes the sealed bytes through the same private boundary.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use sha2::{Digest, Sha256};

use crate::application::port::outbound::task_evidence_artifact_port::TaskEvidenceArtifactPort;
use crate::domain::planning::evidence::{
    MAX_TASK_EVIDENCE_ARTIFACT_BYTES, TaskEvidenceArtifact, TaskEvidenceAttempt, TaskEvidenceError,
    TaskEvidenceTree,
};
use crate::private_sqlite::PinnedPrivateDirectory;

mod files;
#[cfg(test)]
mod tests;

const ARTIFACT_FILE: &str = "artifact";
type RuntimeDirectory = dyn Fn(&str) -> Result<PathBuf> + Send + Sync;

pub struct FilesystemTaskEvidenceArtifactAdapter {
    runtime_directory: Arc<RuntimeDirectory>,
}

impl FilesystemTaskEvidenceArtifactAdapter {
    pub fn new(runtime_directory: Arc<RuntimeDirectory>) -> Self {
        Self { runtime_directory }
    }

    fn evidence_root(&self, workspace: &str, create: bool) -> Result<PinnedPrivateDirectory> {
        let runtime = (self.runtime_directory)(workspace)?;
        PinnedPrivateDirectory::open(&runtime, false)?.child("evidence", create)
    }

    fn incoming(
        &self,
        workspace: &str,
        lease_generation: &str,
        create: bool,
    ) -> Result<PinnedPrivateDirectory> {
        validate_host_id(lease_generation)?;
        self.evidence_root(workspace, create)?
            .child("incoming", create)?
            .child(lease_generation, create)
    }

    fn sealed(
        &self,
        workspace: &str,
        attempt: &TaskEvidenceAttempt,
        create: bool,
    ) -> Result<PinnedPrivateDirectory> {
        attempt.validate()?;
        self.evidence_root(workspace, create)?
            .child("sealed", create)?
            .child(&attempt.attempt_id, create)
    }

    fn capture(
        &self,
        workspace: &str,
        attempt: &TaskEvidenceAttempt,
        observed_tree: &TaskEvidenceTree,
        after_copy: impl FnOnce(),
    ) -> Result<TaskEvidenceArtifact> {
        attempt.validate()?;
        observed_tree.validate()?;
        if observed_tree != &attempt.prepared_tree {
            bail!("task evidence tree changed since preparation");
        }
        // Resolve before pinning: the Windows runtime initializer can request DELETE
        // access, which conflicts with the ancestor pins retained below.
        let evidence = self.evidence_root(workspace, false)?;
        let incoming = evidence
            .child("incoming", false)?
            .child(&attempt.lease_generation, false)?
            .child(&attempt.attempt_id, false)?;
        let mut source = files::StableFile::open(&incoming, ARTIFACT_FILE)?;
        let bytes = source.read_bounded(&incoming, MAX_TASK_EVIDENCE_ARTIFACT_BYTES)?;
        let artifact = artifact_for_bytes(&bytes)?;
        let sealed = evidence
            .child("sealed", true)?
            .child(&attempt.attempt_id, true)?;

        match files::StableFile::open(&sealed, ARTIFACT_FILE) {
            Ok(mut existing) => {
                let existing_bytes =
                    existing.read_bounded(&sealed, MAX_TASK_EVIDENCE_ARTIFACT_BYTES)?;
                if artifact_for_bytes(&existing_bytes)? != artifact {
                    bail!("sealed task evidence bytes do not match the incoming retry");
                }
                after_copy();
                let repeated = source.read_bounded(&incoming, MAX_TASK_EVIDENCE_ARTIFACT_BYTES)?;
                if artifact_for_bytes(&repeated)? != artifact {
                    bail!("task evidence artifact changed during retry verification");
                }
                existing.verify(&sealed)?;
                incoming.verify()?;
                return Ok(artifact);
            }
            Err(error) if is_not_found(&error) => {}
            Err(error) => return Err(error),
        }

        // One fixed scratch name per reserved attempt also bounds crash leftovers.
        // The application holds the pool mutation permit through capture and commit.
        let mut temporary = files::TemporaryFile::create(&sealed, "capture")?;
        temporary.write_all(&bytes)?;
        after_copy();
        // A second read plus full handle identity (ctime/Windows ChangeTime included)
        // detects replacement and same-size/mtime-restored changes during capture.
        let repeated = source.read_bounded(&incoming, MAX_TASK_EVIDENCE_ARTIFACT_BYTES)?;
        if artifact_for_bytes(&repeated)? != artifact {
            bail!("task evidence artifact changed during capture");
        }
        incoming.verify()?;
        sealed.verify()?;
        temporary.publish_noreplace(ARTIFACT_FILE)?;
        source.verify(&incoming)?;
        verify_artifact(&sealed, &artifact)?;
        incoming.verify()?;
        Ok(artifact)
    }
}

impl TaskEvidenceArtifactPort for FilesystemTaskEvidenceArtifactAdapter {
    fn prepare_lease_incoming_root(
        &self,
        workspace: &str,
        lease_generation: &str,
    ) -> Result<PathBuf> {
        let incoming = self.incoming(workspace, lease_generation, true)?;
        incoming.verify()?;
        Ok(incoming.path().to_path_buf())
    }

    fn prepare_artifact(&self, workspace: &str, attempt: &TaskEvidenceAttempt) -> Result<PathBuf> {
        attempt.validate()?;
        let directory = self
            .incoming(workspace, &attempt.lease_generation, true)?
            .child(&attempt.attempt_id, true)?;
        files::create_empty_if_missing(&directory, ARTIFACT_FILE)?;
        directory.sync()?;
        directory.file_path(ARTIFACT_FILE)
    }

    fn observe_and_seal(
        &self,
        workspace: &str,
        attempt: &TaskEvidenceAttempt,
        observed_tree: &TaskEvidenceTree,
    ) -> Result<TaskEvidenceArtifact> {
        self.capture(workspace, attempt, observed_tree, || {})
    }

    fn verify_sealed(
        &self,
        workspace: &str,
        attempt: &TaskEvidenceAttempt,
        artifact: &TaskEvidenceArtifact,
    ) -> Result<()> {
        artifact.validate()?;
        self.sealed(workspace, attempt, false)
            .and_then(|directory| verify_artifact(&directory, artifact))
            .map_err(|error| {
                if is_not_found(&error) {
                    TaskEvidenceError::ArtifactMissing.into()
                } else {
                    error
                }
            })
    }
}

fn artifact_for_bytes(bytes: &[u8]) -> Result<TaskEvidenceArtifact> {
    let artifact = TaskEvidenceArtifact {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        size_bytes: u64::try_from(bytes.len()).map_err(|_| anyhow!("artifact size overflow"))?,
    };
    artifact.validate()?;
    Ok(artifact)
}

fn verify_artifact(
    directory: &PinnedPrivateDirectory,
    expected: &TaskEvidenceArtifact,
) -> Result<()> {
    let bytes = files::StableFile::open(directory, ARTIFACT_FILE)?
        .read_bounded(directory, MAX_TASK_EVIDENCE_ARTIFACT_BYTES)?;
    if artifact_for_bytes(&bytes)? != *expected {
        bail!("sealed task evidence bytes do not match the immutable receipt");
    }
    directory.verify()
}

fn validate_host_id(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("task evidence requires a host-generated 64-character lowercase hex identifier");
    }
    Ok(())
}

fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}
