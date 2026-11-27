use super::*;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::application::port::outbound::parallel_mode_runtime_port::{
    ParallelModeRuntimePort, ParallelWorkerTreeRequest,
};
use crate::application::port::outbound::task_verification_runner_port::{
    PreparedTaskVerification, TaskVerificationPrepareRequest, TaskVerificationRunOutcome,
    TaskVerificationRunnerPort,
};
use crate::domain::planning::evidence::TaskEvidenceTree;
use crate::domain::planning::verification::{
    TaskVerificationExecutionOutcome, TaskVerificationObservation, TaskVerificationRecipe,
};

struct Fixture {
    root: PathBuf,
    base: String,
}

impl Fixture {
    fn new() -> Self {
        Self::with_format("sha1")
    }
    fn with_format(format: &str) -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-task-verification-test-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        git(
            &root,
            &[
                "init",
                "-q",
                "-b",
                "main",
                &format!("--object-format={format}"),
            ],
        );
        git(&root, &["config", "user.name", "Host Verification Fixture"]);
        git(&root, &["config", "user.email", "fixture@example.invalid"]);
        git(&root, &["config", "core.autocrlf", "false"]);
        git(&root, &["config", "commit.gpgSign", "false"]);
        fs::write(root.join("README.md"), "base text\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "base"]);
        let base = git(&root, &["rev-parse", "HEAD"]);
        Self { root, base }
    }
    fn request<'a>(&'a self, source: &'a TaskEvidenceTree) -> TaskVerificationPrepareRequest<'a> {
        TaskVerificationPrepareRequest {
            workspace_directory: self.root.to_str().unwrap(),
            recipe: TaskVerificationRecipe::GitDiffCheckV1,
            base_commit_sha: &self.base,
            source,
        }
    }
    fn prepare(&self, source: &TaskEvidenceTree) -> Box<dyn PreparedTaskVerification> {
        let started = Instant::now();
        GitTaskVerificationRunnerAdapter::new()
            .prepare(self.request(source), &|| false)
            .unwrap_or_else(|reason| {
                panic!(
                    "fixed recipe preparation failed: reason={reason:?}; elapsed_ms={}",
                    started.elapsed().as_millis()
                )
            })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git(root: &Path, arguments: &[&str]) -> String {
    let output = crate::test_utils::git_command()
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn executed(
    outcome: TaskVerificationRunOutcome,
) -> (TaskVerificationObservation, Vec<u8>, Vec<u8>) {
    match outcome {
        TaskVerificationRunOutcome::Executed {
            observation,
            stdout,
            stderr,
        } => {
            observation.validate().unwrap();
            assert_eq!(observation.stdout.sha256, digest(&stdout));
            assert_eq!(observation.stderr.sha256, digest(&stderr));
            (*observation, stdout, stderr)
        }
        TaskVerificationRunOutcome::NotRun { reason } => {
            panic!("fixed recipe did not execute: {reason:?}")
        }
    }
}

#[test]
fn a_real_dirty_snapshot_is_checked_and_the_source_index_is_preserved() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("README.md"), "uncommitted clean change\n").unwrap();
    let index = fs::read(fixture.root.join(".git/index")).unwrap();
    let snapshot = GitParallelModeRuntimeAdapter::new()
        .snapshot_parallel_worker_tree(ParallelWorkerTreeRequest {
            workspace_directory: fixture.root.to_str().unwrap(),
            expected_branch_name: "main",
            expected_base_commit_sha: &fixture.base,
            expected_head_commit_sha: &fixture.base,
        })
        .unwrap();
    let source = TaskEvidenceTree {
        head_commit_sha: snapshot.head_commit_sha,
        tree_sha: snapshot.tree_sha,
        worktree_clean: snapshot.worktree_clean,
    };
    assert!(!source.worktree_clean);
    let (observation, _, _) = executed(fixture.prepare(&source).run(&|| false));
    assert_eq!(
        observation.outcome,
        TaskVerificationExecutionOutcome::Exited { exit_code: 0 }
    );
    assert_eq!(fs::read(fixture.root.join(".git/index")).unwrap(), index);
    assert_eq!(
        fs::read(fixture.root.join("README.md")).unwrap(),
        b"uncommitted clean change\n"
    );
}
