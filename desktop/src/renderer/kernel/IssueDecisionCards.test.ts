import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { IssueDecisionCards } from "./IssueDecisionCards";
import { decisionDraftIdentity } from "./issueInteractions";
import { arbiterFact, arbiterRequest } from "../../../test/fixtures/issueArbiter";

describe("arbiter decision cards", () => {
  it("keeps a selected answer and note enabled in a human Wait card", () => {
    const request = { ...arbiterRequest(), arbiter_wait: { reason: "low_confidence" as const } };
    const answer = vi.fn(async () => {});
    const html = renderToStaticMarkup(createElement(IssueDecisionCards, {
      decisions: [request], history: [], busy: false, onAnswer: answer, onDraftChange: vi.fn(),
      drafts: new Map([[decisionDraftIdentity(request), { option: "approve", note: "검토하던 메모" }]]),
    }));
    expect(html).toContain("사람 답변 대기");
    expect(html).toContain("판단 확신이 기준에 미치지 못했습니다");
    expect(html).toContain("검토하던 메모");
    expect(html.match(/<input[^>]*value="approve"[^>]*>/)?.[0]).toContain("checked");
    expect(html.match(/<button[^>]*data-testid="issue-decision-answer"[^>]*>/)?.[0]).not.toContain("disabled");
    expect(html).toContain("시스템 확인 창에서 한 번 더 확인합니다");
    expect(answer).not.toHaveBeenCalled();
  });

  it("shows the original arbiter or Human answer origin with and without visible historical text", () => {
    const request = { ...arbiterRequest(), state: "answered" as const };
    for (const answer_by of ["arbiter", "human"] as const) {
      for (const visible of [true, false]) {
        const html = renderToStaticMarkup(createElement(IssueDecisionCards, {
          decisions: [], history: [{ decision: { ...arbiterFact(request), answer_by, option: "approve" }, ...(visible ? { request } : {}) }],
          busy: false, onAnswer: vi.fn(async () => {}), onDraftChange: vi.fn(), drafts: new Map(),
        }));
        expect(html).toContain(answer_by === "arbiter" ? "중재자가 답변했습니다" : "사람이 답변했습니다");
        expect(html).not.toContain('data-testid="issue-decision-answer"');
        expect(html).not.toContain('data-testid="issue-arbiter-wait"');
      }
    }
  });
});
