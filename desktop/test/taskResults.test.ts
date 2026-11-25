import { describe, expect, it } from "vitest";
import { TaskAuthoringController, type TaskAuthoringContext } from "../src/renderer/kernel/taskAuthoring";
import { JsonRpcClient } from "../src/renderer/rpc/jsonRpcClient";

function harness() {
  const sent: Array<{ id: number; method: string; params: unknown }> = [];
  let context: TaskAuthoringContext = { workspace: "/a", kernel: "kernel-a", attached: true, epoch: 1 };
  const rpc = new JsonRpcClient((frame) => sent.push(JSON.parse(frame)));
  const controller = new TaskAuthoringController(rpc, () => context);
  return { controller, sent, move: (next: Partial<TaskAuthoringContext>) => { context = { ...context, ...next }; },
    answer: (result: unknown) => rpc.handleText(JSON.stringify({ id: sent.at(-1)?.id, result })) };
}

describe("task result reads", () => {
  it("uses the exact task ID without starting a command or model turn", async () => {
    const h = harness();
    const pending = h.controller.readResult("/a", "completed-task");
    expect(h.sent).toEqual([{ jsonrpc: "2.0", id: expect.any(Number), method: "akra/kernel/query", params: { query: { type: "task_result", task_id: "completed-task" } } }]);
    h.answer({ type: "task_result", task_id: "completed-task", status: "missing" });
    expect((await pending).status).toBe("missing");
    expect(h.sent).toHaveLength(1);
  });
  it("rejects another task's response even when its title or branch would match", async () => {
    const h = harness();
    const pending = h.controller.readResult("/a", "task-a");
    h.answer({ type: "task_result", task_id: "task-b", status: "available" });
    await expect(pending).rejects.toThrow("작업 ID");
  });
  it("rejects task fields with a different identity inside an otherwise matching response", async () => {
    const h = harness();
    const pending = h.controller.readResult("/a", "task-a");
    h.answer({ type: "task_result", task_id: "task-a", status: "available", task: { task_id: "task-b" } });
    await expect(pending).rejects.toThrow("작업 ID");
  });
  it.each([{ workspace: "/b", kernel: "kernel-b", epoch: 2 }, { attached: false, epoch: 2 }, { epoch: 2 }])(
    "discards a response after workspace or connection identity changes: %j", async (change) => {
      const h = harness();
      const pending = h.controller.readResult("/a", "task-a");
      h.move(change);
      h.answer({ type: "task_result", task_id: "task-a", status: "available" });
      await expect(pending).rejects.toThrow("저장소 연결");
    },
  );
});
