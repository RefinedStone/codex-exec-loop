import { describe, expect, it } from "vitest";
import { EMPTY_KERNEL_VIEW, type IssueDecisionAttention } from "../src/renderer/kernel/kernelConnection";
import { isCurrentIssueDecisionAttention, issueDecisionAttentionEvents } from "../src/renderer/session/attention";

const event: IssueDecisionAttention = {
  workspace: "/repo", kernel: "repo", decisionId: "decision-1",
  issue: { provider: "github", server_id: "github.com", repository_id: "R_repo", issue_id: "I_1", project_scope: { workspace_dir: "/repo" } },
};
const state = (events: IssueDecisionAttention[] = []) => ({
  connection: { phase: "connected" as const, endpoint: "fixture" },
  kernel: { ...EMPTY_KERNEL_VIEW, phase: "attached" as const, kernel: "repo", workspace: "/repo", issueAttention: events },
});

describe("private issue attention", () => {
  it("reports only a newly received decision and suppresses duplicate and reconnect population", () => {
    expect(issueDecisionAttentionEvents(state(), state([event]))).toEqual([event]);
    expect(issueDecisionAttentionEvents(state([event]), state([{ ...event }]))).toEqual([]);
    expect(issueDecisionAttentionEvents({ ...state(), connection: { phase: "disconnected", endpoint: "fixture" } }, state([event]))).toEqual([]);
    expect(issueDecisionAttentionEvents({ ...state(), kernel: { ...EMPTY_KERNEL_VIEW, phase: "attaching" } }, state([event]))).toEqual([]);
  });

  it("notifies once for a new durable reminder while keeping the same decision target", () => {
    const reminder = { ...event, reminderId: "issue-decision-reminder:1" };
    expect(issueDecisionAttentionEvents(state([event]), state([reminder]))).toEqual([reminder]);
    expect(issueDecisionAttentionEvents(state([reminder]), state([{ ...reminder }]))).toEqual([]);
    expect(issueDecisionAttentionEvents({ ...state([event]), connection: { phase: "disconnected", endpoint: "fixture" } }, state([reminder]))).toEqual([]);
    expect(issueDecisionAttentionEvents(state([reminder]), state())).toEqual([]);
  });

  it.each(["waiting", "applied"] as const)("reports one verified arbiter %s event without inventing an open decision", (outcome) => {
    const notice = { ...event, arbiter: { eventId: "arbiter-finished-1", outcome } };
    const verified = { ...state(), kernel: { ...state().kernel, issueArbiterNotices: [notice] } };
    expect(verified.kernel.issueAttention).toEqual([]);
    expect(issueDecisionAttentionEvents(state(), verified)).toEqual([notice]);
    expect(issueDecisionAttentionEvents(verified, { ...verified, kernel: { ...verified.kernel, issueArbiterNotices: [{ ...notice }] } })).toEqual([]);
    expect(issueDecisionAttentionEvents(verified, state())).toEqual([]);
    expect(issueDecisionAttentionEvents({ ...state(), connection: { phase: "disconnected", endpoint: "fixture" } }, verified)).toEqual([]);
  });

  it("revokes an older request or reminder while the focus check is pending and keeps only a current arbiter notice", () => {
    const reminder = { ...event, reminderId: "reminder-1" };
    expect(isCurrentIssueDecisionAttention(state([event]), event)).toBe(true);
    expect(isCurrentIssueDecisionAttention(state([reminder]), event)).toBe(false);
    expect(isCurrentIssueDecisionAttention(state([reminder]), reminder)).toBe(true);
    const applied = { ...event, arbiter: { eventId: "applied-1", outcome: "applied" as const } };
    const resolved = { ...state(), kernel: { ...state().kernel, issueArbiterNotices: [applied] } };
    expect(isCurrentIssueDecisionAttention(resolved, event)).toBe(false);
    expect(isCurrentIssueDecisionAttention(resolved, reminder)).toBe(false);
    expect(isCurrentIssueDecisionAttention(resolved, applied)).toBe(true);
    expect(isCurrentIssueDecisionAttention(state(), applied)).toBe(false);
    expect(isCurrentIssueDecisionAttention({ ...resolved, kernel: { ...resolved.kernel, phase: "attaching" } }, applied)).toBe(false);
    expect(isCurrentIssueDecisionAttention({ ...resolved, kernel: { ...resolved.kernel, workspace: "/other" } }, applied)).toBe(false);
  });
});
