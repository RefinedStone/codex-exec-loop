import { describe, expect, it } from "vitest";
import { isTaskEvidenceState } from "../src/renderer/kernel/taskEvidenceValidation";
import { taskEvidenceFixture } from "./fixtures/taskEvidence";
import { taskVerificationFixture } from "./fixtures/taskVerification";

type Snapshot = ReturnType<typeof taskVerificationFixture>;
function withHost(snapshot: Snapshot, host: unknown) {
  const task = snapshot.tasks[0]!;
  return { ...snapshot, tasks: [{ ...task, criteria: [{ ...task.criteria[0]!, host_verification: host }] }] };
}

describe("host verification wire metadata", () => {
  it("keeps legacy evidence and current host execution separate, including empty output", () => {
    const snapshot = taskVerificationFixture();
    expect(isTaskEvidenceState(taskEvidenceFixture())).toBe(true);
    expect(isTaskEvidenceState(snapshot)).toBe(true);
    const entry = snapshot.tasks[0]!.criteria[0]!;
    entry.state = "fail";
    entry.evidence.evidence.status = "fail";
    expect(isTaskEvidenceState(snapshot)).toBe(true);
    expect(entry.host_verification.state).toBe("passed");
    snapshot.tasks[0]!.source.tree.head_commit_sha = "9".repeat(40);
    expect(isTaskEvidenceState(snapshot)).toBe(true);
  });

  it("allows pending with no attempt and a running reservation without inventing a receipt", () => {
    const snapshot = taskVerificationFixture();
    const host = snapshot.tasks[0]!.criteria[0]!.host_verification;
    expect(isTaskEvidenceState(withHost(snapshot, { phase: "pending", state: "pending" }))).toBe(true);
    expect(isTaskEvidenceState(withHost(snapshot, { attempt: host.attempt, phase: "pending", state: "pending" }))).toBe(true);
    expect(isTaskEvidenceState(withHost(snapshot, { attempt: host.attempt, phase: "running", state: "running" }))).toBe(true);
    expect(isTaskEvidenceState(withHost(snapshot, { phase: "running", state: "running" }))).toBe(false);
    expect(isTaskEvidenceState(withHost(snapshot, { ...host, phase: "running" }))).toBe(false);
    expect(isTaskEvidenceState(withHost(snapshot, { ...host, receipt: undefined }))).toBe(false);
  });

  it("requires executed exit zero, complete capture and current artifact/tree for Passed", () => {
    const snapshot = taskVerificationFixture();
    const host = snapshot.tasks[0]!.criteria[0]!.host_verification;
    const fact = host.receipt.verification;
    const observation = fact.result.observation;
    const changedFact = (changes: object) => withHost(snapshot, {
      ...host, receipt: { ...host.receipt, verification: { ...fact, ...changes } },
    });
    for (const changes of [
      { artifact: undefined }, { artifact: { ...fact.artifact, size_bytes: 0 } },
      { manifest: undefined },
      { manifest: { ...fact.manifest, source: { ...fact.manifest.source, tree_sha: "9".repeat(40) } } },
      { result: { kind: "executed", observation: { ...observation, outcome: { kind: "exited", exit_code: 1 } } } },
      { result: { kind: "executed", observation: { ...observation, stdout: { ...observation.stdout, complete: false } } } },
      { result: { kind: "executed", observation: { ...observation, stderr: { ...observation.stderr, complete: false } } } },
    ]) expect(isTaskEvidenceState(changedFact(changes))).toBe(false);
    host.state = "failed";
    expect(isTaskEvidenceState(snapshot)).toBe(false);
    observation.outcome.exit_code = 1;
    expect(isTaskEvidenceState(snapshot)).toBe(true);
  });

  it("preserves typed failed outcomes and monotonic duration when UTC moves backwards", () => {
    const snapshot = taskVerificationFixture();
    const host = snapshot.tasks[0]!.criteria[0]!.host_verification;
    const fact = host.receipt.verification;
    const observation = { ...fact.result.observation, finished_at: "2026-11-22T23:59:59Z" };
    for (const kind of ["timed_out", "cancelled", "output_limit", "signalled", "io_failure"]) {
      const value = withHost(snapshot, { ...host, state: "failed", receipt: {
        ...host.receipt, verification: { ...fact, result: { kind: "executed", observation: { ...observation, outcome: { kind } } } },
      } });
      expect(isTaskEvidenceState(value)).toBe(true);
    }
    expect(isTaskEvidenceState(withHost(snapshot, { ...host, receipt: { ...host.receipt, verification: {
      ...fact, result: { kind: "executed", observation },
    } } }))).toBe(true);
  });

  it("keeps NotRun and Interrupted distinct from execution and permits unavailable historical capture", () => {
    const snapshot = taskVerificationFixture();
    const host = snapshot.tasks[0]!.criteria[0]!.host_verification;
    for (const result of [
      ...["unavailable", "invalid_input", "stale", "scope_changed", "lease_changed", "configuration_changed", "spawn_failed", "cancelled"]
        .map((reason) => ({ kind: "not_run", reason })),
      { kind: "interrupted" },
    ]) {
      const fact = { ...host.receipt.verification, manifest: undefined, artifact: undefined, result };
      const value = { ...host, state: "not_run", receipt: { ...host.receipt, verification: fact } };
      expect(isTaskEvidenceState(withHost(snapshot, value))).toBe(true);
      expect(isTaskEvidenceState(withHost(snapshot, { ...value, state: "passed" }))).toBe(false);
      expect(isTaskEvidenceState(withHost(snapshot, { ...value, receipt: { ...value.receipt,
        verification: { ...fact, artifact: host.receipt.verification.artifact },
      } }))).toBe(false);
    }
    const unavailable = { ...host, state: "unavailable", unavailable: "artifact_missing", receipt: {
      ...host.receipt, verification: { ...host.receipt.verification, artifact: undefined },
    } };
    expect(isTaskEvidenceState(withHost(snapshot, unavailable))).toBe(true);
    expect(isTaskEvidenceState(withHost(snapshot, { ...unavailable, unavailable: undefined }))).toBe(false);
  });

  it("preserves original receipt coordinates for Stale without reviving Passed", () => {
    const snapshot = taskVerificationFixture();
    const task = snapshot.tasks[0]!;
    const entry = task.criteria[0]!;
    const original = entry.host_verification.receipt.verification.attempt.scope.scope_id;
    task.scope = { ...task.scope, scope_id: "9".repeat(64) };
    entry.state = "stale";
    entry.host_verification.state = "stale";
    expect(isTaskEvidenceState(snapshot)).toBe(true);
    expect(entry.host_verification.receipt.verification.attempt.scope.scope_id).toBe(original);
    entry.host_verification.state = "passed";
    expect(isTaskEvidenceState(snapshot)).toBe(false);
  });

  it("retains historical host facts after the user removes the recipe or changes the criterion channel", () => {
    const snapshot = taskVerificationFixture();
    const task = snapshot.tasks[0]!;
    const entry = task.criteria[0]!;
    for (const channel of ["command", "data", "http", "screen"]) {
      const value = (state: string) => ({ ...snapshot, tasks: [{ ...task,
        scope: { ...task.scope, scope_id: "9".repeat(64) },
        criteria: [{ ...entry, state: "stale", criterion: { ...entry.criterion, channel, verification_recipe: undefined },
          host_verification: { ...entry.host_verification, state },
        }],
      }] });
      expect(isTaskEvidenceState(value("stale"))).toBe(true);
      expect(isTaskEvidenceState(value("passed"))).toBe(false);
    }
  });

  it.each(["unavailable", "artifact_missing", "stale"])("rejects current host results while task observation is %s", (unavailable) => {
    const snapshot = taskVerificationFixture();
    const task = snapshot.tasks[0]!;
    const entry = task.criteria[0]!;
    const host = entry.host_verification;
    const value = (host_verification: unknown) => ({ ...snapshot, tasks: [{ ...task, unavailable,
      criteria: [{ ...entry, state: undefined, host_verification }],
    }] });
    expect(isTaskEvidenceState(value(host))).toBe(false);
    expect(isTaskEvidenceState(value({ ...host, state: "failed", receipt: { ...host.receipt,
      verification: { ...host.receipt.verification, result: { kind: "executed", observation: {
        ...host.receipt.verification.result.observation, outcome: { kind: "exited", exit_code: 1 },
      } } },
    } }))).toBe(false);
    expect(isTaskEvidenceState(value({ attempt: host.attempt, phase: "running", state: "running" }))).toBe(false);
    expect(isTaskEvidenceState(value({ ...host, state: "stale" }))).toBe(true);
    expect(isTaskEvidenceState(value({ ...host, state: "unavailable", unavailable }))).toBe(true);
  });

  it("rejects private payloads, mismatched receipt identity and invalid capture bounds", () => {
    const snapshot = taskVerificationFixture();
    const host = snapshot.tasks[0]!.criteria[0]!.host_verification;
    const fact = host.receipt.verification;
    const badFacts = [
      { ...fact, artifact_path: "/private/capture" },
      { ...fact, attempt: { ...fact.attempt, owner_token: "secret" } },
      { ...fact, attempt: { ...fact.attempt, session_key: "private-session" } },
      { ...fact, manifest: { ...fact.manifest, argv: ["private"] } },
      { ...fact, artifact: { ...fact.artifact, size_bytes: 8 * 1024 * 1024 + 1 } },
      { ...fact, result: { ...fact.result, observation: { ...fact.result.observation,
        stdout: { ...fact.result.observation.stdout, text: "raw output" },
      } } },
      { ...fact, result: { ...fact.result, observation: { ...fact.result.observation,
        stderr: { ...fact.result.observation.stderr, size_bytes: 1024 * 1024 + 1 },
      } } },
      { ...fact, result: { ...fact.result, observation: { ...fact.result.observation,
        stdout: { ...fact.result.observation.stdout, sha256: "a".repeat(64) },
      } } },
    ];
    for (const verification of badFacts) expect(isTaskEvidenceState(withHost(snapshot, {
      ...host, receipt: { ...host.receipt, verification },
    }))).toBe(false);
    for (const receipt of [
      { ...host.receipt, event_id: "task-verification-finished:other" },
      { ...host.receipt, evidence_revision: 7 },
      { ...host.receipt, verification: { ...fact, attempt: { ...fact.attempt, requested_at: "2026-11-23T00:01:00Z" } } },
    ]) expect(isTaskEvidenceState(withHost(snapshot, { ...host, receipt }))).toBe(false);
  });

  it("rejects line terminators and mixed Git object formats in the frozen manifest", () => {
    for (const width of [40, 64]) {
      const snapshot = taskVerificationFixture();
      const task = snapshot.tasks[0]!;
      const entry = task.criteria[0]!;
      const manifest = entry.host_verification.receipt.verification.manifest;
      manifest.base_commit_sha = "2".repeat(width);
      manifest.base_tree_sha = "3".repeat(width);
      manifest.source.head_commit_sha = "c".repeat(width);
      manifest.source.tree_sha = "d".repeat(width);
      task.source.tree = { ...manifest.source };
      entry.evidence.evidence.recorded_tree = { ...manifest.source };
      expect(isTaskEvidenceState(snapshot)).toBe(true);
      for (const suffix of ["\n", "\r", "\u2028", "\u2029"]) {
        manifest.base_commit_sha = "2".repeat(width - 1) + suffix;
        expect(isTaskEvidenceState(snapshot)).toBe(false);
      }
      manifest.base_commit_sha = "2".repeat(width === 40 ? 64 : 40);
      expect(isTaskEvidenceState(snapshot)).toBe(false);
    }
  });
});
