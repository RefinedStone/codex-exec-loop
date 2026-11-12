import { describe, expect, it } from "vitest";

import { KERNEL_METHOD, type CommandRequest, type CommandResponse, type KernelCommand } from "../src/renderer/kernel/protocol";
import { sendKernelCommand } from "../src/renderer/kernel/sendKernelCommand";
import { JsonRpcClient } from "../src/renderer/rpc/jsonRpcClient";

interface SentRequest {
  readonly id: number;
  readonly method: string;
  readonly params: CommandRequest;
}

const commands = {
  load_sessions: { type: "load_sessions" },
  refresh_planning: { type: "refresh_planning" },
  rename_session: { type: "rename_session", thread_id: "thread-1", name: "Renamed" },
  submit_prompt: { type: "submit_prompt", thread_id: "thread-1", prompt: "Hello" },
  steer_turn: { type: "steer_turn", thread_id: "thread-1", expected_turn_id: "turn-1", prompt: "Stop here" },
  stop_all: { type: "stop_all" },
  decide_approval: { type: "decide_approval", approval_id: "approval-1", server_request_id: "1", decision: "decline" },
  set_auto_follow_max_turns: { type: "set_auto_follow_max_turns", value: 3 },
  pause_auto_follow: { type: "pause_auto_follow" },
  reset_planning: { type: "reset_planning", target: "queue", confirm: true },
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
    if (command.type === "load_sessions" || command.type === "refresh_planning") {
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
