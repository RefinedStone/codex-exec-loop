import { describe, expect, it } from "vitest";
import { EMPTY_KERNEL_VIEW, type IssueDecisionAttention } from "../src/renderer/kernel/kernelConnection";
import { issueDecisionAttentionEvents } from "../src/renderer/session/attention";

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
});
