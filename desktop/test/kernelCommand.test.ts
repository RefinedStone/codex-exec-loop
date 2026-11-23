import { describe, expect, it } from "vitest";

import { KERNEL_METHOD, type CommandRequest, type CommandResponse, type KernelCommand } from "../src/renderer/kernel/protocol";
import { sendKernelCommand } from "../src/renderer/kernel/sendKernelCommand";
import { JsonRpcClient } from "../src/renderer/rpc/jsonRpcClient";

interface SentRequest {
  readonly id: number;
  readonly method: string;
  readonly params: CommandRequest;
}

const issue = {
  provider: "github",
  server_id: "github.com",
  repository_id: "owner/repository",
  issue_id: "42",
  project_scope: { workspace_dir: "/workspace" },
} as const;

const commands = {
  submit_delegation: { type: "submit_delegation", instruction: "Summarize the draft", idempotency_key: "delegation-1" },
  cancel_delegation: { type: "cancel_delegation", id: "delegation-1" },
  checkpoint_delegation: { type: "checkpoint_delegation", id: "delegation-1", token: "fixture", message: "In progress" },
  report_delegation_result: { type: "report_delegation_result", id: "delegation-1", token: "fixture", result: "Finished" },
  retry_delegation: { type: "retry_delegation", id: "delegation-1" },
  load_sessions: { type: "load_sessions" },
  refresh_planning: { type: "refresh_planning" },
  prepare_evidence: {
    type: "prepare_evidence", request_id: "prepare-1", task_id: "task-1", criterion_id: "criterion-1",
    scope_id: "a".repeat(64), expected_acceptance_revision: 1,
  },
  record_evidence: { type: "record_evidence", attempt_id: "b".repeat(64), status: "pass" },
  waive_criterion: {
    type: "waive_criterion", request_id: "waiver-1", task_id: "task-1", criterion_id: "criterion-1",
    scope_id: "a".repeat(64), expected_acceptance_revision: 1, reason: "Explicitly exempt this criterion.",
  },
  set_issue_automation_mode: { type: "set_issue_automation_mode", mode: "observe" },
  set_issue_autonomy: { type: "set_issue_autonomy", command_id: "autonomy-1", level: "propose" },
  sync_issues: { type: "sync_issues" },
  answer_issue_decision: { type: "answer_issue_decision", decision_id: "decision-1", option: "continue", note: "Shared note" },
  hold_issue: { type: "hold_issue", issue, command_id: "hold-1" },
  release_issue: { type: "release_issue", issue, command_id: "release-1" },
  steer_issue: { type: "steer_issue", issue, command_id: "steer-1", message: "Check the migration" },
  take_over_issue: { type: "take_over_issue", issue, command_id: "take-over-1" },
  retriage_issue: { type: "retriage_issue", issue, command_id: "retriage-1" },
  cancel_issue_work: { type: "cancel_issue_work", issue, command_id: "cancel-1", confirm: true },
  track_issue: { type: "track_issue", issue, command_id: "track-1" },
  untrack_issue: { type: "untrack_issue", issue, command_id: "untrack-1" },
  rename_session: { type: "rename_session", thread_id: "thread-1", name: "Renamed" },
  submit_prompt: { type: "submit_prompt", thread_id: "thread-1", prompt: "Hello" },
  steer_turn: { type: "steer_turn", thread_id: "thread-1", expected_turn_id: "turn-1", prompt: "Stop here" },
  stop_all: { type: "stop_all" },
  enable_parallel: { type: "enable_parallel" },
  disable_parallel: { type: "disable_parallel" },
  decide_approval: { type: "decide_approval", approval_id: "approval-1", server_request_id: "1", decision: "decline" },
  set_auto_follow_max_turns: { type: "set_auto_follow_max_turns", value: 3 },
  pause_auto_follow: { type: "pause_auto_follow" },
  reset_planning: { type: "reset_planning", target: "queue", confirm: true },
  create_task: { type: "create_task", task: { title: "Task" } },
  update_task: { type: "update_task", task_id: "task-1", task: { title: "Task" }, expected_planning_revision: 7 },
  delete_task: { type: "delete_task", task_id: "task-1", expected_planning_revision: 7, confirm: true },
  create_direction: { type: "create_direction", direction: { title: "Release", success_criteria: ["Shipped"] } },
  update_direction: {
    type: "update_direction",
    direction_id: "dir-release",
    direction: { title: "Release", success_criteria: ["Shipped"] },
    expected_planning_revision: 7,
  },
  delete_direction: { type: "delete_direction", direction_id: "dir-release", expected_planning_revision: 7, confirm: true },
} as const satisfies Record<KernelCommand["type"], KernelCommand>;

function harness() {
  const sent: SentRequest[] = [];
  const client = new JsonRpcClient((text) => {
    const request: SentRequest = JSON.parse(text);
    sent.push(request);
  });
  const answer = (request: SentRequest, result: unknown) =>
    client.handleText(JSON.stringify({ jsonrpc: "2.0", id: request.id, result }));
  return { client, sent, answer };
}

describe("kernel command sending", () => {
  it.each(Object.values(commands))("preserves $type and keys only mutations", async (command) => {
    const { client, sent, answer } = harness();
    const result = sendKernelCommand(client, command);
    const request = sent[0];
    if (!request) throw new Error("Command was not sent");
    expect(request.method).toBe(KERNEL_METHOD.command);
    expect(request.params.command).toEqual(command);
    if (command.type === "load_sessions" || command.type === "refresh_planning" || command.type === "sync_issues") {
      expect(request.params).not.toHaveProperty("idempotency_key");
    } else {
      expect(request.params.idempotency_key).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    }
    answer(request, { processed: true });
    await expect(result).resolves.toEqual({ processed: true });
  });

  it("gives independent actions distinct keys even when their content matches", async () => {
    const { client, sent, answer } = harness();
    const first = sendKernelCommand(client, commands.pause_auto_follow);
    const second = sendKernelCommand(client, commands.pause_auto_follow);
    expect(sent).toHaveLength(2);
    expect(new Set(sent.map((request) => request.params.idempotency_key)).size).toBe(2);
    for (const request of sent) answer(request, { processed: true });
    await Promise.all([first, second]);
  });

  it("rejects a disconnected action without replaying it and keys the next action separately", async () => {
    const { client, sent, answer } = harness();
    const interrupted = sendKernelCommand(client, commands.pause_auto_follow);
    const rejected = expect(interrupted).rejects.toThrow("disconnected");
    client.reset("disconnected");
    await rejected;
    expect(sent).toHaveLength(1);
    expect(client.pendingCount).toBe(0);

    const next = sendKernelCommand(client, commands.pause_auto_follow);
    const request = sent[1];
    if (!request) throw new Error("New action was not sent");
    expect(request.params.idempotency_key).not.toBe(sent[0]?.params.idempotency_key);
    answer(request, { processed: true });
    await next;
  });

  it("returns an admission receipt without treating it as command completion", async () => {
    const { client, sent, answer } = harness();
    const result = sendKernelCommand(client, commands.submit_prompt);
    const request = sent[0];
    if (!request) throw new Error("Command was not sent");
    const receipt = { processed: true, operation: { op_id: 1, state: "accepted", replayed: false, next: { query: "history", after: 1 } } } satisfies CommandResponse;
    answer(request, receipt);
    await expect(result).resolves.toEqual(receipt);
  });
});
