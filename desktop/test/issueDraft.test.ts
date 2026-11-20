// 이슈 초안 패널(#2426)의 규칙이다: 구간 옮기기, 승인 비교, 게시 상태 구분.
import { describe, expect, it } from "vitest";

import {
  addSpan,
  intentStatus,
  type IssueDraftApproval,
  type IssueDraftSpan,
  issueDraftTarget,
  publishStatus,
  sameApproval,
  shiftSpans,
  validationErrorText,
} from "../src/renderer/work/issueDraft";

const span = (field: "title" | "body", start: number, end: number): IssueDraftSpan => ({ field, start, end, class: "secret", decision: null });

describe("issue draft spans", () => {
  it("keeps a span before the edit, moves a span after it, and drops a span the edit touched", () => {
    const before = "aaa SECRET bbb TOKEN ccc";
    const spans = [span("body", 4, 10), span("body", 15, 20), span("title", 15, 20)];

    // "bbb" -> "bbbbb": 두 구간의 사이를 고쳤다.
    expect(shiftSpans(spans, "body", before, "aaa SECRET bbbbb TOKEN ccc")).toEqual([span("body", 4, 10), span("body", 17, 22), span("title", 15, 20)]);
    // 첫 구간의 안을 고쳤다.
    expect(shiftSpans(spans, "body", before, "aaa SECET bbb TOKEN ccc")).toEqual([span("body", 14, 19), span("title", 15, 20)]);
    // 글의 앞에 더했다.
    expect(shiftSpans(spans, "body", before, `x${before}`)).toEqual([span("body", 5, 11), span("body", 16, 21), span("title", 15, 20)]);
  });

  it("adds only a non-empty span that does not overlap another span of the same field", () => {
    const spans = [span("body", 4, 10)];
    expect(addSpan(spans, span("body", 10, 12))).toHaveLength(2);
    expect(addSpan(spans, span("title", 4, 10))).toHaveLength(2);
    expect(addSpan(spans, span("body", 9, 12))).toBeNull();
    expect(addSpan(spans, span("body", 12, 12))).toBeNull();
  });
});

describe("issue draft approval and status", () => {
  const approval: IssueDraftApproval = { sharedCopyHash: { version: 1, sha256: "aa" }, policyVersion: "p1", templateVersion: null };

  it("an approval matches only the same shared copy, policy version, and template version", () => {
    expect(sameApproval(approval, { ...approval, sharedCopyHash: { version: 1, sha256: "aa" } })).toBe(true);
    expect(sameApproval(approval, { ...approval, sharedCopyHash: { version: 1, sha256: "bb" } })).toBe(false);
    expect(sameApproval(approval, { ...approval, policyVersion: "p2" })).toBe(false);
    expect(sameApproval(approval, { ...approval, templateVersion: "t1" })).toBe(false);
  });

  it("publish answers map to distinct states", () => {
    const intent = { intentId: "i1", state: "needs_confirmation" as const };
    expect(publishStatus({ status: "confirmed", intent: { intentId: "i1", state: "confirmed" } })).toEqual({ kind: "confirmed" });
    expect(publishStatus({ status: "needs_confirmation", intent })).toEqual({ kind: "needs_confirmation", intentId: "i1" });
    expect(publishStatus({ status: "conflict" })).toEqual({ kind: "external_change" });
    expect(publishStatus({ status: "held", intent })).toEqual({ kind: "held" });
    expect(publishStatus({ status: "refused", reason: "policy_version" })).toEqual({ kind: "refused", reason: "policy_version" });
    expect(publishStatus({ status: "blocked", preview: { status: "invalid_span", spanIndex: 0 } })).toBeNull();
  });

  it("an unresolved journal intent needs confirmation and an ended intent does not block a new edit", () => {
    expect(intentStatus(null)).toBeNull();
    expect(intentStatus({ intentId: "i1", state: "sent" })).toEqual({ kind: "needs_confirmation", intentId: "i1" });
    expect(intentStatus({ intentId: "i1", state: "confirmed" })).toEqual({ kind: "confirmed" });
    expect(intentStatus({ intentId: "i1", state: "abandoned" })).toBeNull();
    expect(intentStatus({ intentId: "i1", state: "conflict" })).toBeNull();
  });

  it("builds the target from owner/name and names the failed field", () => {
    expect(issueDraftTarget("/work/app", "sample/widgets", 7)).toEqual({ cwd: "/work/app", owner: "sample", repository: "widgets", number: 7 });
    expect(issueDraftTarget("/work/app", "widgets", 7)).toBeNull();
    expect(validationErrorText({ field: "required_section", label: "Impact", kind: "missing" })).toBe('필수 섹션 "Impact": 섹션이 없습니다');
    expect(validationErrorText({ field: "title", label: null, kind: "empty" })).toBe("제목: 값이 비어 있습니다");
  });
});
