// 이슈 초안 패널(#2426)의 규칙이다: 구간 옮기기, 승인 비교, 게시 상태 구분.
import { describe, expect, it } from "vitest";

import {
  addSpan,
  expectedLabels,
  issueReadinessDisplay,
  type IssueDraftReadiness,
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
    const isolation = validationErrorText({ field: "code_validator", label: "team-rules", kind: "isolation_unavailable" });
    expect(isolation).toContain("게시가 차단됩니다");
    expect(isolation).toContain("Ubuntu 22.04 + bubblewrap 0.13.1");
    expect(isolation).toContain("격리 없는 실행은 지원하지 않습니다");
  });
});

describe("server-owned draft readiness", () => {
  const result: IssueDraftReadiness = {
    status: "assessed",
    assessment: {
      previewOnly: true, readiness: "needs_info", summary: "서버가 정한 보완 안내",
      missingSections: ["완료 기준", "범위"],
      errors: [{ field: "required_section", label: "완료 기준", kind: "missing" }],
      riskClass: "high", riskSummary: "서버의 위험 설명",
      effectiveMode: "observe", assumedMode: "propose", modeSummary: "서버의 예상 수준 설명",
      expectedGates: [
        { gate: "merge", action: "human", summary: "먼저 온 통합", actionSummary: "사람 확인" },
        { gate: "start", action: "auto", summary: "나중 온 시작", actionSummary: "정책상 자동" },
      ],
      reasons: [{ code: "criteria_missing", summary: "서버가 정한 다음 보완점" }],
      policyVersion: "automation", disclosurePolicyVersion: "disclosure",
      sharedCopyHash: { version: 1, sha256: "shared-copy" },
    },
  };
  it("preserves the server's summaries and gate order without deriving a decision", () => {
    const display = issueReadinessDisplay(result, false);
    expect(display).toBe(result);
    if (display.status !== "assessed") throw new Error("expected the server assessment");
    expect(display.assessment.summary).toBe("서버가 정한 보완 안내");
    expect(display.assessment.riskSummary).toBe("서버의 위험 설명");
    expect(display.assessment.modeSummary).toBe("서버의 예상 수준 설명");
    expect(display.assessment.reasons.map((reason) => reason.summary)).toEqual(["서버가 정한 다음 보완점"]);
    expect(display.assessment.expectedGates.map((gate) => gate.summary)).toEqual(["먼저 온 통합", "나중 온 시작"]);
  });

  it("hides a previously assessed answer while the current draft is pending", () => {
    expect(issueReadinessDisplay(result, true)).toEqual({ status: "pending" });
    expect(issueReadinessDisplay(undefined, true)).toEqual({ status: "pending" });
  });

  it("preserves the server failure and distinguishes a missing answer without inventing gates", () => {
    const blocked: IssueDraftReadiness = {
      status: "cannot_assess", reason: "policy_blocked",
      summary: "서버의 정책 오류 설명", nextSummary: "서버의 복구 안내",
      policyError: { scope: "project", field: "policy", reason: "invalid_toml" },
    };
    const display = issueReadinessDisplay(blocked, false);
    expect(display).toBe(blocked);
    expect(display).not.toHaveProperty("assessment");
    expect(issueReadinessDisplay(blocked, true)).toEqual({ status: "pending" });
    expect(issueReadinessDisplay(undefined, false)).toEqual({ status: "unavailable" });
  });

  it("only separates the user's hypothetical label input and does not add conventional labels", () => {
    expect(expectedLabels("team:automate, SECURITY, team:manual")).toEqual(["team:automate", "SECURITY", "team:manual"]);
    expect(expectedLabels(" , ")).toEqual([]);
    expect(expectedLabels("custom:hold")).toEqual(["custom:hold"]);
  });
});
