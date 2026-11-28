use super::*;
use crate::application::port::inbound::task_evidence_query_port::{
    PreviewKind, PreviewTarget, TaskEvidenceQueryPort,
};

#[test]
fn preview_masks_only_matching_sealed_bytes_and_rejects_a_changed_identity() {
    let f = Fixture::new("sealed-preview");
    let attempt = f.prepare("preview-1");
    let files = crate::composition::task_evidence::artifacts();
    let path = files.prepare_artifact(&f.workspace, &attempt).unwrap();
    let token = "ghp_0123456789abcdefghijklmnop";
    std::fs::write(
        &path,
        format!("Step completed\nAuthorization: Bearer {token}"),
    )
    .unwrap();
    let mut commit = f.record_commit(&attempt);
    commit.artifact = files
        .observe_and_seal(&f.workspace, &attempt, &commit.recorded_tree)
        .unwrap();
    let receipt = f
        .adapter
        .record_evidence(&f.workspace, &commit)
        .unwrap()
        .value;
    let target = PreviewTarget {
        kind: PreviewKind::Worker,
        task_id: attempt.scope.task_id.clone(),
        criterion_id: attempt.criterion_id.clone(),
        scope_id: attempt.scope.scope_id.clone(),
        revision: attempt.scope.acceptance_revision,
        event_id: receipt.event_id.clone(),
        sha256: commit.artifact.sha256.clone(),
    };
    let service = crate::composition::task_evidence::build(&f.workspace);
    let body = service.sanitized_preview(&target).unwrap();
    assert!(body.contains("Step completed"));
    assert!(!body.contains(token));
    assert!(
        !serde_json::to_string(&f.adapter.pending_events(&f.workspace, 1).unwrap())
            .unwrap()
            .contains(token)
    );
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&f.workspace)
            .unwrap();
    let sealed = std::path::Path::new(&location.runtime_dir)
        .join("evidence/sealed")
        .join(&attempt.attempt_id)
        .join("artifact");
    std::fs::write(sealed, format!("different-secret-{token}")).unwrap();
    assert_eq!(
        service.sanitized_preview(&target).unwrap_err().to_string(),
        TaskEvidenceError::Unavailable.to_string()
    );
}
