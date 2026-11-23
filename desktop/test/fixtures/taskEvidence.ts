/** 합성 증거 metadata다. 실제 artifact, 경로, 모델 출력은 담지 않는다. */
export function taskEvidenceFixture() {
  const scope = {
    task_id: "task-1", scope_id: "a".repeat(64), acceptance_revision: 3, criteria_sha256: "b".repeat(64),
  };
  const tree = { head_commit_sha: "c".repeat(40), tree_sha: "d".repeat(40), worktree_clean: false };
  return {
    planning_revision: 7, evidence_revision: 4, truncated: false,
    tasks: [{
      scope,
      source: { kind: "running_lease", tree },
      criteria: [{
        criterion: { id: "criterion with spaces", scenario: "The expected output exists.", channel: "command", essential: true },
        state: "pass",
        evidence: {
          event_id: "criterion-evidence:fixture", evidence_revision: 4,
          evidence: {
            scope: { ...scope }, criterion_id: "criterion with spaces", attempt_id: "e".repeat(64), attempt: 1,
            status: "pass", artifact: { sha256: "f".repeat(64), size_bytes: 42 },
            recorded_tree: { ...tree }, by: "fixture-worker", recorded_at: "2026-11-23T00:00:01Z",
          },
        },
      }],
    }],
  };
}
