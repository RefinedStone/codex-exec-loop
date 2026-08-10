use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};

use crate::application::port::outbound::pr_validation_rollout_evidence_port::{
    PR_VALIDATION_ROLLOUT_EVIDENCE_MAX_BYTES, PR_VALIDATION_ROLLOUT_EVIDENCE_MAX_DOCUMENTS,
    PrValidationRolloutEvidenceDocument, PrValidationRolloutEvidencePort,
};

const ARTIFACTS_RELATIVE_PATH: &str = "docs/validation/artifacts";
const ROLLOUT_ARTIFACT_PREFIX: &str = "post-merge-validation-rollout-";
const ROLLOUT_ARTIFACT_FILE: &str = "evidence.json";
const CANONICAL_ROLLOUT_ARTIFACT_FILE: &str = "pr-validation-rollout-evidence.json";

#[derive(Debug, Default)]
pub struct FilesystemPrValidationRolloutEvidenceAdapter;

impl FilesystemPrValidationRolloutEvidenceAdapter {
    pub fn new() -> Self {
        Self
    }

    fn candidate_paths(workspace_root: &Path) -> Result<Vec<PathBuf>> {
        let artifacts_root = workspace_root.join(ARTIFACTS_RELATIVE_PATH);
        if !artifacts_root.exists() {
            return Ok(Vec::new());
        }
        let canonical_artifacts_root = fs::canonicalize(&artifacts_root).with_context(|| {
            format!(
                "failed to canonicalize rollout evidence root {}",
                artifacts_root.display()
            )
        })?;
        let mut candidates = Vec::new();
        let canonical = artifacts_root.join(CANONICAL_ROLLOUT_ARTIFACT_FILE);
        if canonical.is_file() {
            candidates.push(canonical);
        }
        let mut historical = Vec::new();
        for entry in fs::read_dir(&artifacts_root).with_context(|| {
            format!(
                "failed to enumerate rollout evidence root {}",
                artifacts_root.display()
            )
        })? {
            let entry = entry.context("failed to inspect a rollout evidence directory entry")?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(ROLLOUT_ARTIFACT_PREFIX))
            {
                let path = entry.path().join(ROLLOUT_ARTIFACT_FILE);
                if path.is_file() {
                    historical.push(path);
                }
            }
        }
        historical.sort_by(|left, right| right.cmp(left));
        candidates.extend(historical);
        candidates.truncate(PR_VALIDATION_ROLLOUT_EVIDENCE_MAX_DOCUMENTS);

        candidates
            .into_iter()
            .map(|candidate| {
                let canonical = fs::canonicalize(&candidate).with_context(|| {
                    format!(
                        "failed to canonicalize rollout evidence candidate {}",
                        candidate.display()
                    )
                })?;
                if !canonical.starts_with(&canonical_artifacts_root) {
                    bail!("rollout evidence candidate escaped the repository artifact root");
                }
                Ok(canonical)
            })
            .collect()
    }

    fn load_document(
        artifacts_root: &Path,
        candidate: &Path,
    ) -> Result<PrValidationRolloutEvidenceDocument> {
        let metadata = fs::metadata(candidate).with_context(|| {
            format!(
                "failed to inspect rollout evidence candidate {}",
                candidate.display()
            )
        })?;
        if metadata.len() > PR_VALIDATION_ROLLOUT_EVIDENCE_MAX_BYTES {
            bail!("rollout evidence document exceeds the bounded read contract");
        }
        let modified = metadata
            .modified()
            .context("rollout evidence document has no modification timestamp")?;
        let observed_at: DateTime<Utc> = modified.into();
        candidate
            .strip_prefix(artifacts_root)
            .context("rollout evidence candidate lost its artifact-root binding")?;
        let content = fs::read(candidate).with_context(|| {
            format!(
                "failed to read rollout evidence candidate {}",
                candidate.display()
            )
        })?;
        if content.len() as u64 > PR_VALIDATION_ROLLOUT_EVIDENCE_MAX_BYTES {
            bail!("rollout evidence document exceeded the bounded read contract while reading");
        }
        Ok(PrValidationRolloutEvidenceDocument {
            observed_at: observed_at.to_rfc3339(),
            content,
        })
    }
}

impl PrValidationRolloutEvidencePort for FilesystemPrValidationRolloutEvidenceAdapter {
    fn load_documents(
        &self,
        workspace_dir: &str,
    ) -> Result<Vec<PrValidationRolloutEvidenceDocument>> {
        let workspace_root = fs::canonicalize(workspace_dir).with_context(|| {
            format!("failed to canonicalize evidence workspace {workspace_dir}")
        })?;
        let artifacts_root = fs::canonicalize(workspace_root.join(ARTIFACTS_RELATIVE_PATH))
            .unwrap_or_else(|_| workspace_root.join(ARTIFACTS_RELATIVE_PATH));
        Self::candidate_paths(&workspace_root)?
            .iter()
            .map(|candidate| Self::load_document(&artifacts_root, candidate))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn fixture_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "akra-rollout-evidence-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(ARTIFACTS_RELATIVE_PATH)).expect("fixture root should create");
        root
    }

    #[test]
    fn adapter_reads_only_bounded_repository_artifacts() {
        let root = fixture_root("bounded");
        let artifacts = root.join(ARTIFACTS_RELATIVE_PATH);
        let historical = artifacts.join("post-merge-validation-rollout-2026-08-10");
        fs::create_dir_all(&historical).expect("historical directory should create");
        fs::write(
            historical.join(ROLLOUT_ARTIFACT_FILE),
            b"{\"schemaVersion\":1}",
        )
        .expect("historical evidence should write");
        fs::write(
            artifacts.join(CANONICAL_ROLLOUT_ARTIFACT_FILE),
            b"{\"schemaVersion\":1,\"canonical\":true}",
        )
        .expect("canonical evidence should write");
        fs::write(artifacts.join("unrelated.json"), b"secret")
            .expect("unrelated file should write");

        let documents = FilesystemPrValidationRolloutEvidenceAdapter::new()
            .load_documents(root.to_string_lossy().as_ref())
            .expect("documents should load");

        assert_eq!(documents.len(), 2);
        assert!(
            documents[0]
                .content
                .windows(9)
                .any(|value| value == b"canonical")
        );
        assert!(
            documents
                .iter()
                .all(|document| document.content != b"secret")
        );
        fs::remove_dir_all(root).expect("fixture should clean up");
    }

    #[test]
    fn oversized_evidence_is_rejected_before_reading() {
        let root = fixture_root("oversized");
        let artifacts = root.join(ARTIFACTS_RELATIVE_PATH);
        let candidate = artifacts.join(CANONICAL_ROLLOUT_ARTIFACT_FILE);
        let file = fs::File::create(&candidate).expect("oversized evidence should create");
        file.set_len(PR_VALIDATION_ROLLOUT_EVIDENCE_MAX_BYTES + 1)
            .expect("oversized fixture should resize");

        let error = FilesystemPrValidationRolloutEvidenceAdapter::new()
            .load_documents(root.to_string_lossy().as_ref())
            .expect_err("oversized evidence must fail closed");

        assert!(error.to_string().contains("bounded read contract"));
        fs::remove_dir_all(root).expect("fixture should clean up");
    }
}
