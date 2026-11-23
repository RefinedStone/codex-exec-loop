import { describe, expect, it } from "vitest";
import { hasValidPlanningEvidence, isTaskEvidenceState } from "../src/renderer/kernel/taskEvidenceValidation";
import { taskEvidenceFixture } from "./fixtures/taskEvidence";

describe("task evidence wire metadata", () => {
  it("keeps optional legacy planning and exact observed metadata", () => {
    expect(hasValidPlanningEvidence({ revision: 7 })).toBe(true);
    expect(hasValidPlanningEvidence({ revision: 7, task_evidence: null })).toBe(true);
    expect(isTaskEvidenceState(taskEvidenceFixture())).toBe(true);
    const sameTree = taskEvidenceFixture();
    sameTree.tasks[0]!.source.tree.head_commit_sha = "9".repeat(40);
    expect(isTaskEvidenceState(sameTree)).toBe(true);
    expect(hasValidPlanningEvidence({ revision: 7, task_evidence: taskEvidenceFixture() })).toBe(true);
    expect(hasValidPlanningEvidence({ revision: 8, task_evidence: taskEvidenceFixture() })).toBe(false);
  });

  it("keeps Pending without a lease and distinguishes unavailable observation", () => {
    const snapshot = taskEvidenceFixture();
    const task = snapshot.tasks[0]!;
    const criterion = task.criteria[0]!;
    const pending = { ...task, source: undefined, criteria: [{ criterion: criterion.criterion, state: "pending" }] };
    expect(isTaskEvidenceState({ ...snapshot, tasks: [pending] })).toBe(true);
    const unavailable = { ...task, source: undefined, unavailable: "unavailable", criteria: [{ ...criterion, state: undefined }] };
    expect(isTaskEvidenceState({ ...snapshot, tasks: [unavailable] })).toBe(true);
    expect(isTaskEvidenceState({ ...snapshot, tasks: [{ ...unavailable, criteria: [criterion] }] })).toBe(false);
    expect(isTaskEvidenceState({ unavailable: "unavailable", tasks: [], truncated: false })).toBe(true);
    expect(isTaskEvidenceState({ ...snapshot, unavailable: "unavailable" })).toBe(false);
  });

  it("keeps the old receipt scope for Stale and never upgrades it to current Pass", () => {
    const snapshot = taskEvidenceFixture();
    const task = snapshot.tasks[0]!;
    const current = { ...task.scope, scope_id: "1".repeat(64) };
    const changed = { ...task, scope: current, source: undefined, criteria: [{ ...task.criteria[0]!, state: "stale" }] };
    expect(isTaskEvidenceState({ ...snapshot, tasks: [changed] })).toBe(true);
    expect(changed.criteria[0]!.evidence.evidence.scope.scope_id).toBe(task.scope.scope_id);
    expect(isTaskEvidenceState({ ...snapshot, tasks: [{ ...changed, source: task.source, criteria: task.criteria }] })).toBe(false);
  });

  it("allows only a Human waiver in the exact acceptance scope without requiring a source", () => {
    const snapshot = taskEvidenceFixture();
    const task = snapshot.tasks[0]!;
    const entry = {
      criterion: task.criteria[0]!.criterion, state: "waived",
      waiver: {
        event_id: "criterion-waiver:fixture", evidence_revision: 4,
        waiver: { scope: task.scope, criterion_id: task.criteria[0]!.criterion.id, reason_digest: "1".repeat(64), by: "human", recorded_at: "2026-11-23T00:00:02Z" },
      },
    };
    const waived = { ...task, source: undefined, criteria: [entry] };
    expect(isTaskEvidenceState({ ...snapshot, tasks: [waived] })).toBe(true);
    for (const actor of ["worker", "system", "agent"]) {
      entry.waiver.waiver.by = actor;
      expect(isTaskEvidenceState({ ...snapshot, tasks: [waived] })).toBe(false);
    }
    entry.waiver.waiver.by = "human";
    expect(isTaskEvidenceState({ ...snapshot, tasks: [{ ...waived, scope: { ...task.scope, scope_id: "2".repeat(64) } }] })).toBe(false);
  });

  it("rejects line terminators in Git object ids and SHA-256 tokens", () => {
    for (const width of [40, 64]) {
      const valid = taskEvidenceFixture();
      valid.tasks[0]!.source.tree.head_commit_sha = "a".repeat(width);
      expect(isTaskEvidenceState(valid)).toBe(true);
      for (const suffix of ["\n", "\r", "\u2028", "\u2029"]) {
        for (const digits of [width - 1, width]) {
          const invalid = taskEvidenceFixture();
          invalid.tasks[0]!.source.tree.head_commit_sha = "a".repeat(digits) + suffix;
          expect(isTaskEvidenceState(invalid)).toBe(false);
        }
      }
    }
    for (const suffix of ["\n", "\r", "\u2028", "\u2029"]) {
      const invalid = taskEvidenceFixture();
      invalid.tasks[0]!.criteria[0]!.evidence.evidence.artifact.sha256 = "a".repeat(64) + suffix;
      expect(isTaskEvidenceState(invalid)).toBe(false);
    }
  });

  it("rejects malformed bounds, duplicate coordinates, private payloads, and unknown states", () => {
    const snapshot = taskEvidenceFixture();
    const task = snapshot.tasks[0]!;
    const criterion = task.criteria[0]!;
    const withTask = (changed: unknown) => ({ ...snapshot, tasks: [changed] });
    const withEvidence = (changed: unknown) => withTask({ ...task, criteria: [{ ...criterion, evidence: { ...criterion.evidence, evidence: changed } }] });
    const evidence = criterion.evidence.evidence;
    for (const invalid of [
      { ...snapshot, evidence_revision: Number.MAX_SAFE_INTEGER + 1 },
      { ...snapshot, tasks: [task, task] },
      { ...snapshot, tasks: Array.from({ length: 257 }, () => task) },
      { ...snapshot, truncated: true },
      { ...snapshot, truncated: true, next_task_id: "another-task" },
      withTask({ ...task, scope: { ...task.scope, scope_id: "A".repeat(64) } }),
      withTask({ ...task, criteria: [criterion, criterion] }),
      withTask({ ...task, criteria: [{ ...criterion, state: "verified" }] }),
      withEvidence({ ...evidence, attempt: 65 }),
      withEvidence({ ...evidence, artifact: { ...evidence.artifact, size_bytes: 0 } }),
      withEvidence({ ...evidence, artifact: { ...evidence.artifact, size_bytes: 8 * 1024 * 1024 + 1 } }),
      withEvidence({ ...evidence, artifact: { ...evidence.artifact, sha256: "short" } }),
      withEvidence({ ...evidence, recorded_tree: { ...evidence.recorded_tree, tree_sha: "9".repeat(40) } }),
      withEvidence({ ...evidence, artifact_path: "/private/path" }),
      withEvidence({ ...evidence, note: "private note" }),
      withEvidence({ ...evidence, scope: { ...task.scope, task_id: "foreign-task" } }),
    ]) expect(isTaskEvidenceState(invalid)).toBe(false);
    expect(isTaskEvidenceState({ ...snapshot, truncated: true, next_task_id: task.scope.task_id })).toBe(true);
  });
});
