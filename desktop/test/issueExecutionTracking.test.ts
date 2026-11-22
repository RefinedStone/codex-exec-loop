import { describe, expect, it } from "vitest";
import { hasValidIssueExecutionTracking } from "../src/renderer/kernel/issuePresentationValidation";

const execution = () => ({
  status: "ready", work_order_id: "work-order-fixture", revision: 41,
  state: "done", state_label: "서버가 정한 통합 상태",
  execution_attempts: 2, execution_attempt_limit: 3, verification_attempts: 4,
  review_pull_request_url: "https://github.com/sample/widgets/pull/9",
  terminal_class: "contract_recovery",
  recovery: { reason: "contract_recovery", summary: "서버 종료 이유", next_summary: "서버 다음 행동", requires_human: true },
});
const integration = {
  record_id: "integration-1", source_sha: "a".repeat(40), integration_sha: "b".repeat(40),
  pull_request_url: "https://github.com/sample/widgets/pull/9",
};
const entries = () => Array.from({ length: 20 }, (_, index) => ({
  sequence: index + 101, recorded_at: "2026-11-22T09:00:00Z", kind: "issue_execution_progressed", summary: "서버 기록 " + index,
}));

describe("issue execution presentation contract", () => {
  it("accepts integration, external Open and a recovery reason together without reclassifying them", () => {
    expect(hasValidIssueExecutionTracking({
      execution: execution(),
      feedback: { revision: 2, publication_attempts: 5, external_state: "open", integration, reason: "feedback_unconfirmed" },
      timeline: { status: "ready", entries: entries(), has_older: true },
    })).toBe(true);
  });

  it("keeps unavailable, omitted records and explicit nullable ready fields distinct", () => {
    expect(hasValidIssueExecutionTracking({})).toBe(true);
    expect(hasValidIssueExecutionTracking({ execution: { status: "unavailable", reason: "read_failed" }, timeline: { status: "unavailable" } })).toBe(true);
    expect(hasValidIssueExecutionTracking({ execution: { status: "unavailable", reason: "work_id_mismatch" } })).toBe(true);
    expect(hasValidIssueExecutionTracking({
      execution: { ...execution(), execution_attempt_limit: null, review_pull_request_url: null, terminal_class: null, recovery: null },
    })).toBe(true);
    expect(hasValidIssueExecutionTracking({ execution: null })).toBe(false);
    expect(hasValidIssueExecutionTracking({ timeline: null })).toBe(false);
    expect(hasValidIssueExecutionTracking({ execution: { status: "unavailable", reason: "assume_done" } })).toBe(false);
  });

  it.each([
    ["execution_attempts", -1], ["execution_attempts", 1.5], ["execution_attempts", 0x1_0000_0000],
    ["execution_attempt_limit", undefined], ["verification_attempts", "4"],
    ["review_pull_request_url", undefined], ["terminal_class", undefined], ["recovery", undefined],
    ["state_label", null], ["state", "closed"],
  ])("rejects malformed %s instead of rendering a guessed execution result", (key, value) => {
    expect(hasValidIssueExecutionTracking({ execution: { ...execution(), [key as string]: value } })).toBe(false);
  });

  it("rejects incomplete server recovery sentences and unsupported feedback facts", () => {
    expect(hasValidIssueExecutionTracking({ execution: { ...execution(), recovery: { reason: "contract_recovery", requires_human: true } } })).toBe(false);
    expect(hasValidIssueExecutionTracking({ feedback: { revision: 1, publication_attempts: -1 } })).toBe(false);
    expect(hasValidIssueExecutionTracking({ feedback: { revision: 1, publication_attempts: 1, external_state: "merged" } })).toBe(false);
    expect(hasValidIssueExecutionTracking({ feedback: { revision: 1, publication_attempts: 1, reason: "retry_automatically" } })).toBe(false);
  });

  it("rejects execution terminal and recovery codes outside the generated wire enums", () => {
    expect(hasValidIssueExecutionTracking({ execution: { ...execution(), terminal_class: "done" } })).toBe(false);
    expect(hasValidIssueExecutionTracking({ execution: { ...execution(), recovery: { ...execution().recovery, reason: "post_merge_contract_failed" } } })).toBe(false);
  });

  it("requires at most twenty ordered unique entries without sorting or truncating the response", () => {
    const timeline = { status: "ready", entries: entries(), has_older: true };
    expect(hasValidIssueExecutionTracking({ timeline })).toBe(true);
    expect(hasValidIssueExecutionTracking({ timeline: { ...timeline, entries: [...entries(), { ...entries()[0]!, sequence: 121 }] } })).toBe(false);
    expect(hasValidIssueExecutionTracking({ timeline: { ...timeline, entries: entries().reverse() } })).toBe(false);
    expect(hasValidIssueExecutionTracking({ timeline: { ...timeline, entries: [entries()[0], entries()[0]] } })).toBe(false);
    expect(hasValidIssueExecutionTracking({ timeline: { ...timeline, entries: [{ ...entries()[0], summary: null }] } })).toBe(false);
  });
});
