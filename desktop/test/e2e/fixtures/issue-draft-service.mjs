import { createHash } from "node:crypto";
import { readinessFixture } from "./issue-readiness.mjs";
import { reviewRequest } from "./draft-review.mjs";

// 시험용 가짜 값이다. 실제 비밀이 아니다.
export const SECRET = "synthetic-secret-0000";
export const LOG_LINE = "synthetic-log-1111";

/** 가짜 이슈 서비스이다. 원격 글 한 건, 정책 판, 게시 의도 하나를 가진다. */
export const readiness = readinessFixture();
export const issue = {
  remote: { title: "Old title", body: "## Steps\nold steps\n" },
  policyVersion: "p1",
  intent: null,
  /** 다음 게시가 외부 변경을 만나게 한다. */
  externalEdit: false,
  confirmLogs: false,
  requests: [],
};
const fieldHash = (copy) => ({ version: 1, sha256: createHash("sha256").update(`${copy.title}\n${copy.body}`).digest("hex") });

function preview(text) {
  const unanswered = text.spans.flatMap((span, index) => ((span.class === null || (span.class === "log" && issue.confirmLogs)) && span.decision === null ? [index] : []));
  if (unanswered.length > 0) return { status: "needs_confirmation", spanIndexes: unanswered };
  const sharedCopy = { title: text.title, body: text.body };
  const markers = [];
  // 뒤의 구간부터 바꾸면 앞 구간의 위치가 그대로이다.
  const ordered = text.spans.map((span, index) => ({ ...span, index })).sort((left, right) => right.start - left.start);
  for (const span of ordered) {
    const masks = span.decision === "restorable_mask" || (span.decision === null && span.class === "log");
    const marker = `[[masked:${span.index + 1}]]`;
    if (masks) markers.unshift({ marker, field: span.field });
    const value = sharedCopy[span.field];
    sharedCopy[span.field] = value.slice(0, span.start) + (masks ? marker : "[removed]") + value.slice(span.end);
  }
  const errors = sharedCopy.body.includes("## Impact") ? [] : [{ field: "required_section", label: "Impact", kind: "missing" }];
  return {
    status: "ready",
    sharedCopy,
    markers,
    errors,
    validatorAbsent: false,
    publishable: errors.length === 0,
    approval: { sharedCopyHash: fieldHash(sharedCopy), policyVersion: issue.policyVersion, templateVersion: "t1" },
  };
}

export function answerIssue(method, params) {
  issue.requests.push({ method, params });
  if (method === "akra/issue/open") {
    return {
      ...issue.remote,
      editBasis: fieldHash(issue.remote),
      template: { status: "resolved", path: ".github/ISSUE_TEMPLATE/bug.md", reason: null, requiredInputs: [{ label: "Steps", kind: "section", options: [] }], unsupportedInputs: [] },
      requiredSections: ["Impact"],
      spanActions: { secret: "remove", name: "restorable_mask", path: "restorable_mask", log: "restorable_mask" },
      intent: issue.intent,
    };
  }
  if (method === "akra/issue/review") return reviewRequest(params, preview);
  if (method === "akra/issue/readinessBarrier") return { ok: true };
  if (method === "akra/issue/preview") return readiness.reply(preview(params.text));
  if (method === "akra/issue/validation/cancel") {
    // #2858: 실행 중인 검증만 취소된다. 결과가 정해진 뒤의 취소는 거절이다.
    const held = issue.heldPublish;
    if (!held || held.finished || held.validationId !== params.validationId) return { cancelled: false };
    held.resolve("cancelled");
    return { cancelled: true };
  }
  if (method === "akra/issue/publish" && issue.holdValidation) {
    issue.holdValidation = false;
    return new Promise((resolve) => {
      issue.heldPublish = { validationId: params.validationId, finished: issue.validationFinished, resolve: (outcome) => {
        issue.heldPublish = null;
        const current = preview(params.text);
        const errors = [...(current.errors ?? []), { field: "code_validator", label: "lint", kind: outcome }];
        resolve({ status: "blocked", preview: { ...current, errors, publishable: false } });
      } };
    });
  }
  if (method === "akra/issue/publish") {
    const current = preview(params.text);
    if (current.status !== "ready" || !current.publishable) return { status: "blocked", preview: current };
    if (params.approval.sharedCopyHash.sha256 !== current.approval.sharedCopyHash.sha256) return { status: "refused", reason: "shared_copy" };
    if (params.approval.policyVersion !== issue.policyVersion) return { status: "refused", reason: "policy_version" };
    if (issue.externalEdit) {
      issue.externalEdit = false;
      issue.remote = { title: "Edited elsewhere", body: "## Steps\nedited by another person\n" };
    }
    if (params.editBasis.sha256 !== fieldHash(issue.remote).sha256) return { status: "conflict" };
    if (current.markers.length > 0 && !params.sharedCopyOnly) return { status: "held", intent: { intentId: "held-1", state: "held" } };
    // 쓰기는 되었지만 답을 잃은 경우이다.
    issue.remote = current.sharedCopy;
    issue.intent = { intentId: "intent-1", state: "needs_confirmation" };
    return { status: "needs_confirmation", intent: issue.intent };
  }
  if (method === "akra/issue/recheck") {
    if (issue.intent) issue.intent = { ...issue.intent, state: "confirmed" };
    return { intent: issue.intent };
  }
  return { intent: { intentId: params.intentId, state: "abandoned" } };
}
