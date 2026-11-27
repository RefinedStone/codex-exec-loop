use super::*;
use crate::acceptance::{TaskAcceptanceChannel, TaskAcceptanceCriterion};

fn criterion(id: &str) -> TaskAcceptanceCriterion {
    TaskAcceptanceCriterion {
        id: id.into(),
        scenario: "The expected result is observable".into(),
        channel: TaskAcceptanceChannel::Command,
        essential: true,
        verification_recipe: None,
    }
}

#[test]
fn evidence_criteria_hash_preserves_e1_noop_order_and_tracks_meaning() {
    let first = criterion("criterion one");
    let second = criterion("criterion two");
    let original = criteria_sha256(&[first.clone(), second.clone()]).unwrap();
    assert_eq!(original, criteria_sha256(&[second.clone(), first]).unwrap());
    let mut changed = second;
    changed.essential = false;
    assert_ne!(
        original,
        criteria_sha256(&[criterion("criterion one"), changed]).unwrap()
    );
    assert!(validate_evidence_identifier("criterion with internal spaces").is_ok());
    assert!(validate_evidence_identifier("  ").is_err());
}

#[test]
fn evidence_domain_rejects_unsafe_tokens_and_javascript_revision_overflow() {
    for token in ["a".repeat(63), "A".repeat(64), "g".repeat(64)] {
        assert!(validate_evidence_token(&token).is_err());
    }
    let mut scope = TaskEvidenceScope {
        task_id: "task-1".into(),
        scope_id: "a".repeat(64),
        acceptance_revision: MAX_TASK_EVIDENCE_REVISION,
        criteria_sha256: "b".repeat(64),
    };
    scope.validate().unwrap();
    scope.acceptance_revision += 1;
    assert_eq!(scope.validate(), Err(TaskEvidenceError::Limit));
}

#[test]
fn evidence_artifact_requires_nonempty_bounded_host_hash_and_closed_shape() {
    let mut artifact = TaskEvidenceArtifact {
        sha256: "c".repeat(64),
        size_bytes: 1,
    };
    artifact.validate().unwrap();
    artifact.size_bytes = 0;
    assert_eq!(artifact.validate(), Err(TaskEvidenceError::ArtifactMissing));
    artifact.size_bytes = MAX_TASK_EVIDENCE_ARTIFACT_BYTES + 1;
    assert_eq!(artifact.validate(), Err(TaskEvidenceError::Limit));
    assert!(
        serde_json::from_value::<TaskEvidenceArtifact>(serde_json::json!({
            "sha256": "c".repeat(64), "size_bytes": 1, "path": "/arbitrary"
        }))
        .is_err()
    );
}

#[test]
fn evidence_notes_and_waivers_have_one_canonical_bounded_value() {
    assert_eq!(normalize_evidence_note(Some("  \n  ")).unwrap(), None);
    assert_eq!(
        normalize_evidence_note(Some(" result \n")).unwrap(),
        Some("result".into())
    );
    assert_eq!(
        normalize_waiver_reason(" reason \n").unwrap(),
        normalize_waiver_reason("reason").unwrap()
    );
    assert!(normalize_waiver_reason(" ").is_err());
    assert_eq!(
        normalize_evidence_note(Some(&"a".repeat(MAX_TASK_EVIDENCE_NOTE_BYTES + 1))),
        Err(TaskEvidenceError::Limit)
    );
}
