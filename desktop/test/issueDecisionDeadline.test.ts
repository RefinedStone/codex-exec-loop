import { afterEach, describe, expect, it, vi } from "vitest";
import { issueDecisionDeadline } from "../src/renderer/kernel/issueDecisionDeadline";
import { decisionDraftIdentity } from "../src/renderer/kernel/issueInteractions";
import { hasValidIssueInteractions } from "../src/renderer/kernel/issuePresentationValidation";
import type { IssueDecisionRequestState } from "../src/renderer/kernel/protocol";

const request = (): IssueDecisionRequestState => ({
  id: "decision-start", kind: "start_approval", gate: "start", state: "open", revision: 4,
  subject: { kind: "issue", issue: { provider: "github", server_id: "github.com", repository_id: "R_repo", issue_id: "I_1", project_scope: { workspace_dir: "/repo" } } },
  basis: { policy_version: "policy-1", gate_revision: 1, authorization_epoch: 0 },
  question: "작업을 시작할까요?",
  options: [{ id: "start", label: "현재 계획으로 시작", effect: "구현 작업을 시작합니다." }],
  recommended: "start", on_timeout: "wait", due_at: "2026-11-24T09:00:00Z",
});
const due = Date.parse(request().due_at!);
afterEach(() => vi.restoreAllMocks());

describe("decision deadline display", () => {
  it("uses only the server query time even when the client clock is far ahead", () => {
    vi.spyOn(Date, "now").mockReturnValue(due + 365 * 86_400_000);
    const shown = issueDecisionDeadline(request(), due - 7_200_000)!;
    expect(shown.remaining).toBe("2시간 남음");
    expect(shown.asOf).toBe("2026-11-24T07:00:00.000Z");
    expect(shown.dueAt).toBe("2026-11-24T09:00:00.000Z");
  });

  it.each([[-1, "1분 미만 남음"], [0, "기한 지남"], [1, "기한 지남"]] as const)("keeps the exact deadline boundary at offset %s", (offset, remaining) => {
    const pending = request();
    const before = JSON.stringify(pending);
    expect(issueDecisionDeadline(pending, due + offset)?.remaining).toBe(remaining);
    expect(issueDecisionDeadline(pending, due + offset)?.afterDeadline).toContain("사람의 답변을 기다립니다");
    expect(JSON.stringify(pending)).toBe(before);
    expect(pending.state).toBe("open");
  });

  it("shows the server's recommended option for Default without selecting or answering it", () => {
    const pending = { ...request(), on_timeout: "default" as const };
    expect(issueDecisionDeadline(pending, due - 60_000)?.afterDeadline).toContain("‘현재 계획으로 시작’ 선택을 적용합니다");
    expect(pending.state).toBe("open");
    expect(issueDecisionDeadline({ ...pending, recommended: null }, due)?.afterDeadline).not.toContain("선택을 적용합니다");
  });

  it.each([undefined, null, -1, Number.NaN, Number.POSITIVE_INFINITY, 1.5, Number.MAX_SAFE_INTEGER])("does not substitute a client clock for missing or invalid query time %s", (asOf) => {
    const shown = issueDecisionDeadline(request(), asOf)!;
    expect(shown.asOf).toBeNull();
    expect(shown.remaining).toBe("조회 시각 미확인");
  });

  it("preserves legacy requests without adding a guessed deadline", () => {
    expect(issueDecisionDeadline({ ...request(), due_at: null }, due)).toBeNull();
    expect(issueDecisionDeadline({ ...request(), due_at: "invalid" }, due)).toBeNull();
  });

  it("keeps a decision draft identity through a later query and rejects invalid wire query times", () => {
    const pending = request();
    const originalKey = decisionDraftIdentity(pending);
    const later = structuredClone(pending);
    expect(issueDecisionDeadline(pending, due - 7_200_000)?.remaining).not.toBe(issueDecisionDeadline(later, due)?.remaining);
    expect(decisionDraftIdentity(later)).toBe(originalKey);
    const detail = { open_decisions: [pending], as_of_unix_ms: due };
    expect(hasValidIssueInteractions(detail, pending.subject.issue)).toBe(true);
    expect(hasValidIssueInteractions({ ...detail, as_of_unix_ms: "server-time" }, pending.subject.issue)).toBe(false);
    expect(hasValidIssueInteractions({ ...detail, as_of_unix_ms: -1 }, pending.subject.issue)).toBe(false);
  });
});
