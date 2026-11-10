// 커널 패널의 연결 시험이다 (plan 18 K3-6). 서버 역할은 손으로 쓴 응답으로 대신한다 (턴 없음).
import { describe, expect, it } from "vitest";

import { EMPTY_KERNEL_VIEW, KernelConnection, type KernelView } from "../src/renderer/kernel/kernelConnection";
import { JsonRpcClient } from "../src/renderer/rpc/jsonRpcClient";

interface Sent {
  id: number;
  method: string;
  params: Record<string, unknown>;
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

const PLANNING = {
  type: "planning",
  status: "ready_with_task",
  queue_summary: "1 task",
  next_task: { rank: 1, task_id: "task-1", direction_title: "K3", title: "Ship K3-6", status: "in_progress", priority: 10 },
  active_tasks: [{ rank: 1, task_id: "task-1", direction_title: "K3", title: "Ship K3-6", status: "in_progress", priority: 10 }],
  proposed_tasks: [],
  skipped_tasks: [],
};

function harness() {
  const sent: Sent[] = [];
  const client = new JsonRpcClient((text) => sent.push(JSON.parse(text) as Sent));
  let view: KernelView = EMPTY_KERNEL_VIEW;
  const connection = new KernelConnection(client, { getKernel: () => view, setKernel: (next) => (view = next) });
  const reply = (request: Sent | undefined, result: unknown) =>
    client.handleText(JSON.stringify({ jsonrpc: "2.0", id: request?.id, result }));
  const fail = (request: Sent | undefined, code: number, message: string) =>
    client.handleText(JSON.stringify({ jsonrpc: "2.0", id: request?.id, error: { code, message } }));
  const requests = (method: string) => sent.filter((message) => message.method === method);
  const query = (type: string) => requests("akra/kernel/query").find((message) => (message.params.query as { type: string }).type === type);
  /** 붙기 응답 뒤 네 조회에 답한다. */
  const attachAndAnswer = async (answers: Record<string, unknown> = {}) => {
    reply(requests("akra/kernel/attach").at(-1), { client_id: "1", protocol_version: 0, kernel: "codex-exec-loop-abc" });
    await flush();
    for (const type of ["thread_activity", "auto_follow", "planning", "parallel"]) {
      reply(query(type), answers[type] ?? defaultAnswer(type));
    }
    await flush();
  };
  return { sent, connection, reply, fail, requests, query, attachAndAnswer, view: () => view };
}

function defaultAnswer(type: string): unknown {
  switch (type) {
    case "thread_activity":
      return { type, threads: [] };
    case "auto_follow":
      return { type, max_turns: 3, paused: false };
    case "planning":
      return PLANNING;
    default:
      return { type: "parallel", enabled: false };
  }
}

describe("kernel connection", () => {
  it("attaches to the open folder's repository kernel and reads the view", async () => {
    const { connection, requests, attachAndAnswer, view } = harness();
    connection.connectionChanged(true);
    void connection.show("/repo/worktree");

    expect(requests("akra/kernel/attach")[0]?.params).toEqual({ workspace: "/repo/worktree", client_name: "akra-desktop" });
    expect(view().phase).toBe("attaching");
    await attachAndAnswer();

    expect(view()).toMatchObject({
      phase: "attached",
      kernel: "codex-exec-loop-abc",
      autoFollow: { max_turns: 3, paused: false },
      parallel: { enabled: false },
      planning: { status: "ready_with_task", queue_summary: "1 task" },
    });
    expect(view().planning).not.toHaveProperty("type");
    expect(requests("akra/kernel/command").map((message) => message.params.command)).toEqual([{ type: "refresh_planning" }]);
  });

  it("keeps an event that arrived while the same query was in flight", async () => {
    const { connection, reply, requests, query, view } = harness();
    connection.connectionChanged(true);
    void connection.show("/repo");
    reply(requests("akra/kernel/attach")[0], { client_id: "1", protocol_version: 0, kernel: "repo" });
    await flush();

    const running = { thread_id: "thread-1", running: true, auto_follow_active: false };
    connection.handleEvent({ event: { type: "thread_activity", threads: [running] } });
    reply(query("thread_activity"), { type: "thread_activity", threads: [] });
    await flush();

    expect(view().threads).toEqual([running]);
  });

  it("reads the whole view again after a resync", async () => {
    const { connection, requests, attachAndAnswer } = harness();
    connection.connectionChanged(true);
    void connection.show("/repo");
    await attachAndAnswer();
    const before = requests("akra/kernel/query").length;

    connection.handleEvent({ event: { type: "resync" } });

    expect(requests("akra/kernel/query").length - before).toBe(4);
  });

  it("explains a refused attach when the TUI holds the repository", async () => {
    const { connection, fail, requests, view } = harness();
    connection.connectionChanged(true);
    void connection.show("/repo");

    fail(requests("akra/kernel/attach")[0], -32012, "the repository kernel runs in another process (pid 42)");
    await flush();

    expect(view().phase).toBe("failed");
    expect(view().message).toContain("TUI");
    expect(view().message).toContain("pid 42");
  });

  it("detaches when the panel closes and ignores later events", async () => {
    const { connection, requests, attachAndAnswer, view } = harness();
    connection.connectionChanged(true);
    void connection.show("/repo");
    await attachAndAnswer();

    void connection.show(null);
    connection.handleEvent({ event: { type: "auto_follow", max_turns: 9, paused: true } });

    expect(requests("akra/kernel/detach")).toHaveLength(1);
    expect(view()).toEqual(EMPTY_KERNEL_VIEW);
  });

  it("attaches again after the Gateway connection comes back", async () => {
    const { connection, requests, attachAndAnswer, view } = harness();
    connection.connectionChanged(true);
    void connection.show("/repo");
    await attachAndAnswer();

    connection.connectionChanged(false);
    expect(view()).toMatchObject({ phase: "idle", workspace: "/repo" });
    connection.connectionChanged(true);

    expect(requests("akra/kernel/attach")).toHaveLength(2);
    await attachAndAnswer();
    expect(view().phase).toBe("attached");
  });

  it("keeps the view when another folder of the same repository attaches to the same kernel", async () => {
    const { connection, reply, requests, attachAndAnswer, view } = harness();
    connection.connectionChanged(true);
    void connection.show("/repo");
    await attachAndAnswer();

    void connection.show("/repo/sub");
    expect(view()).toMatchObject({ phase: "attaching", workspace: "/repo/sub", kernel: "codex-exec-loop-abc", autoFollow: { max_turns: 3 } });
    reply(requests("akra/kernel/attach").at(-1), { client_id: "2", protocol_version: 0, kernel: "codex-exec-loop-abc" });
    await flush();

    expect(view()).toMatchObject({ phase: "attached", workspace: "/repo/sub", planning: { queue_summary: "1 task" } });
  });

  it("clears the view when the new folder belongs to another kernel", async () => {
    const { connection, reply, requests, attachAndAnswer, view } = harness();
    connection.connectionChanged(true);
    void connection.show("/repo");
    await attachAndAnswer();

    void connection.show("/other");
    reply(requests("akra/kernel/attach").at(-1), { client_id: "2", protocol_version: 0, kernel: "other-repo" });
    await flush();

    expect(view()).toMatchObject({ phase: "attached", kernel: "other-repo", autoFollow: null, planning: null });
  });

  it("stays attached when reading the view fails after attach", async () => {
    const { connection, reply, fail, requests, query, view } = harness();
    connection.connectionChanged(true);
    void connection.show("/repo");
    reply(requests("akra/kernel/attach")[0], { client_id: "1", protocol_version: 0, kernel: "repo" });
    await flush();

    fail(query("planning"), -32013, "the kernel did not answer in time");
    for (const type of ["thread_activity", "auto_follow", "parallel"]) reply(query(type), defaultAnswer(type));
    await flush();

    expect(view().phase).toBe("attached");
    expect(view().message).toContain("did not answer in time");
  });

  it("reports a failed resync read instead of leaving a rejected promise", async () => {
    const { connection, fail, requests, attachAndAnswer, view } = harness();
    connection.connectionChanged(true);
    void connection.show("/repo");
    await attachAndAnswer();
    const before = requests("akra/kernel/query").length;

    connection.handleEvent({ event: { type: "resync" } });
    for (const message of requests("akra/kernel/query").slice(before)) fail(message, -32012, "the kernel stopped");
    await flush();

    expect(view()).toMatchObject({ phase: "attached", message: expect.stringContaining("the kernel stopped") });
  });

  it("ignores a late attach answer for a folder the panel left", async () => {
    const { connection, reply, requests, view } = harness();
    connection.connectionChanged(true);
    void connection.show("/first");
    void connection.show("/second");

    reply(requests("akra/kernel/attach")[0], { client_id: "1", protocol_version: 0, kernel: "first" });
    await flush();
    expect(view()).toMatchObject({ phase: "attaching", workspace: "/second" });

    reply(requests("akra/kernel/attach")[1], { client_id: "2", protocol_version: 0, kernel: "second" });
    await flush();
    expect(view()).toMatchObject({ phase: "attached", kernel: "second" });
  });
});
