import { describe, expect, it } from "vitest";
import { issueDeliveryReview } from "../src/renderer/kernel/issueDeliveryReview";
import { decisionDraftIdentity } from "../src/renderer/kernel/issueInteractions";
import { hasValidIssueInteractions } from "../src/renderer/kernel/issuePresentationValidation";
import type { IssueDecisionRequestState } from "../src/renderer/kernel/protocol";

const request = (): IssueDecisionRequestState => ({
  id: "delivery-1", kind: "risk_exception", gate: "delivery", state: "open", revision: 5,
  subject: { kind: "issue", issue: { provider: "github", server_id: "github.com", repository_id: "R_repo", issue_id: "I_1", project_scope: { workspace_dir: "/repo" } } },
  basis: { policy_version: "policy-1", gate_revision: 4, authorization_epoch: 0, work_order_id: "work-1",
    delivery: { base_commit_sha: "a".repeat(40), source_commit_sha: "b".repeat(40) } },
  delivery_review: { changed_files: 51, changed_lines: 1_001, reasons: ["protected_path", "diff_too_large"] },
  question: "현재 변경을 제출할까요?", options: [{ id: "approve", label: "제출 승인", effect: "검토한 변경을 제출합니다." }],
  recommended: null, on_timeout: "wait", due_at: null,
});
const valid = (value: unknown) => hasValidIssueInteractions({ open_decisions: [value] }, request().subject.issue);

describe("frozen delivery review", () => {
  it("shows the stored range, aggregate size and every server reason without deciding a gate", () => {
    const pending = request();
    const before = structuredClone(pending);
    expect(issueDeliveryReview(pending)).toEqual({
      baseSha: "a".repeat(40), sourceSha: "b".repeat(40), files: "51개", lines: "1,001줄 (추가 + 삭제)",
      reasons: [{ code: "protected_path", label: "보호 경로 변경" }, { code: "diff_too_large", label: "변경량 한도 초과" }],
    });
    expect(pending).toEqual(before);
    expect(valid(pending)).toBe(true);
  });

  it("keeps binary line counts unknown instead of displaying zero", () => {
    const pending: IssueDecisionRequestState = { ...request(),
      delivery_review: { changed_files: 1, changed_lines: null, reasons: ["binary_diff"] } };
    expect(issueDeliveryReview(pending)?.lines).toBe("미확인 · 바이너리 포함");
    expect(valid(pending)).toBe(true);
  });

  it("keeps a draft for a repeated snapshot and invalidates it when either frozen commit changes", () => {
    const pending = request();
    const key = decisionDraftIdentity(pending);
    expect(decisionDraftIdentity(structuredClone(pending))).toBe(key);
    for (const field of ["base_commit_sha", "source_commit_sha"] as const) {
      const changed = { ...pending, basis: { ...pending.basis,
        delivery: { ...pending.basis.delivery!, [field]: "c".repeat(40) } } };
      expect(decisionDraftIdentity(changed)).not.toBe(key);
    }
    const historical = { ...pending, state: "superseded" as const };
    expect(issueDeliveryReview(historical)?.sourceSha).toBe("b".repeat(40));
  });

  it("preserves a legacy Start card and only displays work-bound Delivery risk decisions", () => {
    const start = { ...request(), kind: "start_approval" as const, gate: "start" as const,
      basis: { policy_version: "policy-1", gate_revision: 1 }, delivery_review: undefined };
    expect(issueDeliveryReview(start)).toBeNull();
    expect(valid(start)).toBe(true);
    for (const candidate of [
      { ...request(), kind: "start_approval" as const },
      { ...request(), gate: "start" as const },
      { ...request(), basis: { ...request().basis, work_order_id: null } },
    ]) {
      expect(issueDeliveryReview(candidate)).toBeNull();
      expect(valid(candidate)).toBe(false);
    }
  });

  it("rejects incomplete or invalid review facts before showing an answer form", () => {
    const pending = request();
    const review = pending.delivery_review!;
    for (const delivery_review of [undefined, null,
      { ...review, changed_files: 0 }, { ...review, changed_files: Number.MAX_SAFE_INTEGER + 1 },
      { ...review, changed_lines: -1 }, { ...review, changed_lines: 1.5 }, { ...review, changed_lines: null },
      { ...review, reasons: [] }, { ...review, reasons: ["invented"] },
      { ...review, reasons: ["protected_path", "protected_path"] },
      { ...review, reasons: ["binary_diff"] },
    ]) expect(valid({ ...pending, delivery_review })).toBe(false);
    for (const delivery of [undefined, null,
      { base_commit_sha: "branch/main", source_commit_sha: "b".repeat(40) },
      { base_commit_sha: "a".repeat(40), source_commit_sha: "a".repeat(40) },
      { base_commit_sha: "a".repeat(40), source_commit_sha: "b".repeat(64) },
    ]) expect(valid({ ...pending, basis: { ...pending.basis, delivery } })).toBe(false);
    expect(valid({ ...pending, on_timeout: "default" })).toBe(false);
  });
});
