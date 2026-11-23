import { describe, expect, it } from "vitest";
import { ISSUE_ARBITER_WAIT_TEXT, currentIssueArbiterNotice, issueDecisionAnswerOrigin } from "../src/renderer/kernel/issueArbiter";
import { issueDecisionDeadline } from "../src/renderer/kernel/issueDecisionDeadline";
import { decisionDraftIdentity } from "../src/renderer/kernel/issueInteractions";
import { hasValidIssueInteractions } from "../src/renderer/kernel/issuePresentationValidation";
import { arbiterDetail, arbiterEvent, arbiterFact, arbiterRequest } from "./fixtures/issueArbiter";
import type { IssueArbiterWaitReason } from "../src/renderer/kernel/protocol";

describe("arbiter display and current notification proof", () => {
  it("keeps the original request, timeout and draft identity while showing every closed Wait reason", () => {
    const request = arbiterRequest();
    for (const reason of Object.keys(ISSUE_ARBITER_WAIT_TEXT) as IssueArbiterWaitReason[]) {
      const waiting = { ...request, arbiter_wait: { reason } };
      expect(hasValidIssueInteractions(arbiterDetail(waiting), request.subject.issue)).toBe(true);
      expect(decisionDraftIdentity(waiting)).toBe(decisionDraftIdentity(request));
      expect(waiting.on_timeout).toBe("arbiter");
      expect(waiting.revision).toBe(request.revision);
      expect(issueDecisionDeadline(waiting, Date.parse(request.due_at!))?.afterDeadline).toContain("사람의 답변을 기다립니다");
    }
    expect(issueDecisionDeadline(request)?.afterDeadline).toContain("중재자가 검토합니다");
    expect(issueDecisionAnswerOrigin("arbiter")).toBe("중재자가 답변했습니다.");
    expect(issueDecisionAnswerOrigin("human")).toBe("사람이 답변했습니다.");
  });

  it("rejects unknown or extra Wait fields and Wait on terminal or non-Arbiter requests", () => {
    const request = arbiterRequest();
    const waiting = { ...request, arbiter_wait: { reason: "low_confidence" } };
    for (const candidate of [
      { ...waiting, arbiter_wait: { reason: "invented" } },
      { ...waiting, arbiter_wait: { reason: "low_confidence", model_text: "private" } },
      { ...waiting, state: "answered" }, { ...waiting, on_timeout: "wait" }, { ...waiting, on_timeout: "default" },
    ]) expect(hasValidIssueInteractions({ open_decisions: [candidate] }, request.subject.issue)).toBe(false);
    expect(hasValidIssueInteractions(arbiterDetail(request), request.subject.issue)).toBe(true);
  });

  it("requires current matching Wait metadata and never infers Open from an older event", () => {
    const request = arbiterRequest();
    const event = arbiterEvent(request);
    const waiting = arbiterDetail({ ...request, arbiter_wait: { reason: "low_confidence" } });
    expect(currentIssueArbiterNotice(event, waiting)).toBe("waiting");
    expect(currentIssueArbiterNotice(event, arbiterDetail(request))).toBeNull();
    expect(currentIssueArbiterNotice(event, { ...waiting, revision: 7 })).toBeNull();
    expect(currentIssueArbiterNotice(event, { ...waiting, open_decisions: [] })).toBeNull();
    expect(currentIssueArbiterNotice(event, arbiterDetail({ ...request, basis: { ...request.basis, authorization_epoch: 1 },
      arbiter_wait: { reason: "low_confidence" } }))).toBeNull();
    const human = { ...arbiterFact(request), state: "answered" as const, answer_by: "human" as const, option: "approve" };
    expect(currentIssueArbiterNotice(event, { ...waiting, revision: 9, open_decisions: [], decision_history: [{ decision: human }] })).toBeNull();
  });

  it("announces Applied only from the current by:arbiter history, preserving a Human-first result", () => {
    const request = arbiterRequest();
    const decision = { ...arbiterFact(request), state: "answered" as const, answer_by: "arbiter" as const, option: "approve" };
    const event = { ...arbiterEvent(request), decision, outcome: { kind: "applied" as const } };
    const detail = { ...arbiterDetail(request), open_decisions: [], decision_history: [{ decision }] };
    expect(currentIssueArbiterNotice(event, detail)).toBe("applied");
    expect(currentIssueArbiterNotice(event, { ...detail, decision_history: [{ decision: { ...decision, answer_by: "human" } }] })).toBeNull();
    expect(currentIssueArbiterNotice({ ...event, outcome: { kind: "discarded" } }, detail)).toBeNull();
    expect(currentIssueArbiterNotice({ ...event, outcome: { kind: "superseded" } }, detail)).toBeNull();
  });
});
