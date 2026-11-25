import { describe, expect, it } from "vitest";
import { isTaskEvidenceState } from "../src/renderer/kernel/taskEvidenceValidation";
import { verificationBlockReason, verificationTarget, type TaskVerificationRequestView } from "../src/renderer/kernel/taskVerificationActions";
import { taskVerificationFixture } from "./fixtures/taskVerification";

function validated(value: unknown) {
  if (!isTaskEvidenceState(value)) throw new Error("Invalid test fixture");
  return value;
}

describe("host verification request eligibility", () => {
  it("allows only the fixed recipe in a current running lease and preserves unknown-request retry", () => {
    const fixture = taskVerificationFixture();
    const snapshot = validated(fixture);
    const target = verificationTarget(snapshot.tasks[0]!, snapshot.tasks[0]!.criteria[0]!.criterion.id);
    expect(verificationBlockReason(snapshot, "current", target)).toBeNull();
    expect(verificationBlockReason(snapshot, "stale", target)).not.toBeNull();
    expect(verificationBlockReason(snapshot, "current", { ...target, expected_acceptance_revision: 2 })).not.toBeNull();
    expect(verificationBlockReason({ ...snapshot, tasks: [] }, "current", target)).not.toBeNull();
    const queued = { ...snapshot, tasks: [{ ...snapshot.tasks[0]!, source: { ...snapshot.tasks[0]!.source!, kind: "queued_source" as const } }] };
    expect(verificationBlockReason(queued, "current", target)).not.toBeNull();
    const unknown: TaskVerificationRequestView = { status: "unknown", requestId: "same", attemptId: null, message: "unknown" };
    expect(verificationBlockReason(snapshot, "current", target, unknown)).toBeNull();
    expect(verificationBlockReason(snapshot, "current", target, { ...unknown, status: "sending" })).not.toBeNull();
    expect(verificationBlockReason(snapshot, "current", target, { ...unknown, status: "accepted", attemptId: "new" })).not.toBeNull();
    expect(verificationBlockReason(snapshot, "current", target, { ...unknown, status: "accepted" })).not.toBeNull();
    expect(verificationBlockReason(snapshot, "current", target, {
      ...unknown, status: "accepted", attemptId: fixture.tasks[0]!.criteria[0]!.host_verification.attempt.attempt_id,
    })).toBeNull();
  });
});
