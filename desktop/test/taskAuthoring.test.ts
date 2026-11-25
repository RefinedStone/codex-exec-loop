import { describe, expect, it } from "vitest";
import { TaskAuthoringController, type TaskAuthoringContext } from "../src/renderer/kernel/taskAuthoring";
import { JsonRpcClient } from "../src/renderer/rpc/jsonRpcClient";
import type { CommandRequest, TaskFields } from "../src/renderer/kernel/protocol";

interface Request { id: number; method: string; params: Record<string, unknown> }
const fields: TaskFields = {
  title: "Reviewed task", description: "Only the chosen text", direction_id: "dir-1",
  status: "ready", base_priority: 14, dynamic_priority_delta: 3, priority_reason: "keep",
  depends_on: ["dependency"], blocked_by: ["blocker"],
  acceptance: [{ id: "criterion-1", scenario: "Result is visible", channel: "screen", essential: true }],
};
function harness() {
  const sent: Request[] = [];
  let context: TaskAuthoringContext = { workspace: "/a", kernel: "repo-a", attached: true, epoch: 1 };
  const client = new JsonRpcClient((text) => sent.push(JSON.parse(text) as Request));
  const controller = new TaskAuthoringController(client, () => context);
  const answer = (at: number, result: unknown) => client.handleText(JSON.stringify({ id: sent[at]?.id, result }));
  return { controller, client, sent, answer, move: (patch: Partial<TaskAuthoringContext>) => { context = { ...context, ...patch }; } };
}
const settled = () => new Promise<void>((resolve) => queueMicrotask(resolve));

describe("task authoring", () => {
  it("reads the authority snapshot without a mutation and rejects late results after a repository switch", async () => {
    const h = harness();
    const read = h.controller.read("/a");
    expect(h.sent[0]?.params).toEqual({ query: { type: "list_tasks" } });
    h.move({ workspace: "/b", kernel: "repo-b", epoch: 2 });
    h.answer(0, { type: "list_tasks", available: true, planning_revision: 7, tasks: [] });
    await expect(read).rejects.toThrow("저장소 연결");
    expect(h.sent).toHaveLength(1);
  });

  it("coalesces duplicate confirmation, keeps source fields, and saves new tasks without starting work", async () => {
    const h = harness();
    const first = h.controller.save("/a", null, fields, 7);
    const duplicate = h.controller.save("/a", null, fields, 7);
    expect(h.sent).toHaveLength(1);
    const request = h.sent[0]?.params as unknown as CommandRequest;
    expect(request.idempotency_key).toBeTruthy();
    expect(request.command).toEqual({ type: "create_task", task: { ...fields, status: "awaiting_user" } });
    h.answer(0, { processed: true, result: { type: "task_created", task_id: "original-task", planning_revision: 8 } });
    expect(await first).toEqual({ kind: "saved", taskId: "original-task", planningRevision: 8 });
    expect(await duplicate).toEqual(await first);
  });

  it("preserves untouched fields and uses the exact read revision for operator updates", async () => {
    const h = harness();
    const result = h.controller.save("/a", "task-1", { ...fields, title: "Updated title" }, 7);
    expect((h.sent[0]?.params as unknown as CommandRequest).command).toEqual({
      type: "update_task", task_id: "task-1", task: { ...fields, title: "Updated title" }, expected_planning_revision: 7,
    });
    h.answer(0, { processed: true, rejection: { reason: "planning_revision_stale", message: "planning changed", next: [] } });
    expect(await result).toEqual({ kind: "rejected", message: "planning changed" });
    expect(h.sent).toHaveLength(1);
  });

  it("recovers a lost create reply by the same key and its own durable operation without another mutation", async () => {
    const h = harness();
    const create = h.controller.save("/a", null, fields, 7);
    const original = structuredClone(h.sent[0]?.params);
    h.client.reset("connection lost");
    expect((await create).kind).toBe("uncertain");
    const recovery = h.controller.recover("/a");
    expect(h.sent[1]?.params).toEqual({ query: { type: "history", after: 0, limit: 500 } });
    const key = (original as unknown as CommandRequest).idempotency_key;
    h.answer(1, { type: "history", next_after: 21, records: [
      { sequence: 21, event: { kind: "command_received", op_id: 21, command: "create_task", idempotency_key: key } },
    ] });
    await settled(); await settled();
    h.answer(2, {
      type: "operation", op_id: 21, state: "done", records: [
        { sequence: 21, recorded_at: "now", event: { kind: "command_received", op_id: 21, command: "create_task", idempotency_key: key } },
        { sequence: 22, recorded_at: "now", event: { kind: "command_completed", op_id: 21, result: "done", task_mutation: { task_id: "original-task", planning_revision: 8 } } },
      ],
    });
    expect(await recovery).toEqual({ kind: "saved", taskId: "original-task", planningRevision: 8 });
    expect(h.sent.filter((r) => r.method === "akra/kernel/command")).toHaveLength(1);
  });

  it.each(["wrong-key", "missing-receipt", "wrong-operation"])("never guesses a task after replay with %s", async (problem) => {
    const h = harness();
    const save = h.controller.save("/a", null, fields, 7);
    const key = (h.sent[0]?.params as unknown as CommandRequest).idempotency_key;
    h.answer(0, { processed: true, operation: { op_id: 1, state: "accepted", replayed: true } });
    await settled(); await settled();
    h.answer(1, {
      type: "operation", op_id: problem === "wrong-operation" ? 2 : 1, state: "done",
      records: [
        { sequence: 1, event: { kind: "command_received", op_id: 1, command: "create_task", idempotency_key: problem === "wrong-key" ? "another" : key } },
        { sequence: 2, event: { kind: "command_completed", op_id: 1, result: "done",
          ...(problem === "missing-receipt" ? {} : { task_mutation: { task_id: "unrelated", planning_revision: 9 } }) } },
      ],
    });
    expect((await save).kind).toBe("uncertain");
    const changed = await h.controller.save("/a", null, { ...fields, title: "different" }, 7);
    expect(changed.kind).toBe("uncertain");
    expect(h.sent).toHaveLength(2);
  });

  it("keeps a completed reply scoped to the original repository across a switch", async () => {
    const h = harness();
    const save = h.controller.save("/a", null, fields, 7);
    h.move({ workspace: "/b", kernel: "repo-b", epoch: 2 });
    h.answer(0, { processed: true, result: { type: "task_created", task_id: "task-a", planning_revision: 8 } });
    await expect(save).rejects.toThrow("저장소 연결");
    expect(() => h.controller.recover("/a")).toThrow("커널");
    h.move({ workspace: "/a", kernel: "repo-a", epoch: 3 });
    expect(await h.controller.recover("/a")).toEqual({ kind: "saved", taskId: "task-a", planningRevision: 8 });
    expect(h.sent).toHaveLength(1);
  });

  it("blocks active task edits and invalid criteria without a request", async () => {
    const h = harness();
    expect((await h.controller.save("/a", "task-1", { ...fields, status: "in_progress" }, 7)).kind).toBe("rejected");
    expect((await h.controller.save("/a", null, { ...fields, acceptance: [{ id: "c", scenario: "", channel: "screen", essential: true }] }, 7)).kind).toBe("rejected");
    expect(h.sent).toHaveLength(0);
  });
  it("keeps an unknown save unresolved when its key has expired from history", async () => {
    const h = harness();
    const save = h.controller.save("/a", null, fields, 7);
    h.client.reset("lost reply");
    expect((await save).kind).toBe("uncertain");
    const recovery = h.controller.recover("/a");
    h.answer(1, { type: "history", next_after: 10001, records: [] });
    expect((await recovery).kind).toBe("uncertain");
    expect(h.sent.filter((r) => r.method === "akra/kernel/command")).toHaveLength(1);
  });

  it.each([
    { processed: false, result: { type: "task_created", task_id: "bad", planning_revision: 8 } },
    { processed: true, result: { type: "task_updated", task_id: "bad", planning_revision: 8 } },
  ])("does not treat a malformed direct result as a saved create", async (response) => {
    const h = harness();
    const save = h.controller.save("/a", null, fields, 7);
    h.answer(0, response);
    expect((await save).kind).toBe("uncertain");
  });

});
