use super::*;
use crate::domain::kernel_event::KernelEvent;
use crate::domain::planning::evidence::{TaskEvidenceEvent, TaskEvidenceTree};
use crate::domain::planning::verification::{
    TaskVerificationExecutionOutcome, TaskVerificationObservation, TaskVerificationStream,
};

pub(crate) fn sample_task_verification_event() -> TaskEvidenceEvent {
    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    TaskEvidenceEvent::VerificationFinished {
        receipt: Box::new(TaskVerificationReceipt {
            event_id: format!("task-verification-finished:{}", "a".repeat(64)),
            evidence_revision: 6,
            attempt: TaskVerificationAttempt {
                attempt_id: "a".repeat(64),
                request_id: "private-request".into(),
                scope: TaskEvidenceScope {
                    task_id: "task-1".into(),
                    scope_id: "b".repeat(64),
                    acceptance_revision: 3,
                    criteria_sha256: "c".repeat(64),
                },
                criterion_id: "criterion with spaces".into(),
                recipe: TaskVerificationRecipe::GitDiffCheckV1,
                attempt: 2,
                slot_id: "private-slot".into(),
                lease_generation: "d".repeat(64),
                session_key: "private-session".into(),
                requested_at: "2026-11-23T00:00:00Z".into(),
            },
            manifest: Some(TaskVerificationExecutionManifest {
                recipe: TaskVerificationRecipe::GitDiffCheckV1,
                base_commit_sha: "1".repeat(40),
                base_tree_sha: "2".repeat(40),
                source: TaskEvidenceTree {
                    head_commit_sha: "3".repeat(40),
                    tree_sha: "4".repeat(40),
                    worktree_clean: false,
                },
                git_binary_sha256: "5".repeat(64),
                command_sha256: "6".repeat(64),
                environment_sha256: "7".repeat(64),
                configuration_sha256: "8".repeat(64),
            }),
            result: TaskVerificationResult::Executed {
                observation: Box::new(TaskVerificationObservation {
                    outcome: TaskVerificationExecutionOutcome::Exited { exit_code: 0 },
                    started_at: "2026-11-23T00:00:01Z".into(),
                    finished_at: "2026-11-23T00:00:01.050Z".into(),
                    duration_ms: 50,
                    stdout: TaskVerificationStream {
                        sha256: EMPTY_SHA256.into(),
                        size_bytes: 0,
                        complete: true,
                    },
                    stderr: TaskVerificationStream {
                        sha256: EMPTY_SHA256.into(),
                        size_bytes: 0,
                        complete: true,
                    },
                }),
            },
            artifact: Some(TaskEvidenceArtifact {
                sha256: "9".repeat(64),
                size_bytes: 512,
            }),
            finished_at: "2026-11-23T00:00:01.050Z".into(),
        }),
    }
}

#[test]
fn host_fact_keeps_execution_metadata_and_omits_private_capabilities_and_contents() {
    let event = sample_task_verification_event();
    event.validate().unwrap();
    let record = KernelEvent::task_evidence(&event);
    assert_eq!(record.task_evidence_event_id(), Some(event.event_id()));
    assert_eq!(record.issue_work_event_id(), None);
    assert_eq!(record.op_id(), None);
    assert_eq!(record.thread_id(), "");
    let encoded = serde_json::to_value(&record).unwrap();
    assert_eq!(encoded["kind"], "task_verification_finished");
    assert_eq!(
        encoded["verification"]["result"]["observation"]["outcome"]["exit_code"],
        0
    );
    assert_eq!(
        encoded["verification"]["result"]["observation"]["stdout"]["size_bytes"],
        0
    );
    for private in [
        "request_id",
        "slot_id",
        "lease_generation",
        "session_key",
        "owner_token",
        "run_token",
        "artifact_path",
        "argv",
        "private-session",
        "private-request",
        "private-slot",
    ] {
        assert!(!encoded.to_string().contains(private), "{private}");
    }
    assert_eq!(
        serde_json::from_value::<KernelEvent>(encoded).unwrap(),
        record
    );
}
