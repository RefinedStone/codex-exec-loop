// No model turns. Requests and store changes are the synchronization signals.
import { describe, expect, it } from "vitest";

import { EMPTY_KERNEL_VIEW, KernelConnection, type KernelView } from "../src/renderer/kernel/kernelConnection";
import { JsonRpcClient } from "../src/renderer/rpc/jsonRpcClient";
import { taskEvidenceFixture } from "./fixtures/taskEvidence";

interface Sent {
  id: number;
  method: string;
  params: { query?: { type: string }; command?: unknown; idempotency_key?: string };
}

const PLANNING = {
  type: "planning",
  status: "ready_with_task",
  queue_summary: "1 task",
  next_task: { rank: 1, task_id: "task-1", direction_title: "K3", title: "Ship K3-6", status: "in_progress", priority: 10 },
  active_tasks: [{ rank: 1, task_id: "task-1", direction_title: "K3", title: "Ship K3-6", status: "in_progress", priority: 10 }],
  proposed_tasks: [],
  skipped_tasks: [],
};

function defaultAnswer(type: string): unknown {
  switch (type) {
    case "thread_activity": return { type, threads: [] };
    case "auto_follow": return { type, max_turns: 3, paused: false };
    case "planning": return PLANNING;
    default: return { type: "parallel", enabled: false };
  }
}

function harness() {
  const sent: Sent[] = [];
  const listeners = new Set<() => void>();
  const changed = () => { for (const listener of listeners) listener(); };
  const client = new JsonRpcClient((text) => {
    const request: Sent = JSON.parse(text);
    sent.push(request);
    changed();
  });
  let view: KernelView = EMPTY_KERNEL_VIEW;
  const connection = new KernelConnection(client, {
    getKernel: () => view,
    setKernel: (next) => { view = next; changed(); },
  });
  const when = (condition: () => boolean) => {
    if (condition()) return Promise.resolve();
    return new Promise<void>((resolve, reject) => {
      const check = () => {
        if (!condition()) return;
        clearTimeout(timeout);
        listeners.delete(check);
        resolve();
      };
      const timeout = setTimeout(() => {
        listeners.delete(check);
        reject(new Error("Expected request or store change did not arrive"));
      }, 2_000);
      listeners.add(check);
    });
  };
  const reply = (request: Sent | undefined, result: unknown) => {
    if (!request) throw new Error("Request is missing");
    client.handleText(JSON.stringify({ jsonrpc: "2.0", id: request.id, result }));
  };
  const fail = (request: Sent | undefined, code: number, message: string) => {
    if (!request) throw new Error("Request is missing");
    client.handleText(JSON.stringify({ jsonrpc: "2.0", id: request.id, error: { code, message } }));
  };
  const requests = (method: string) => sent.filter((message) => message.method === method);
  const query = (type: string) => requests("akra/kernel/query").findLast((message) => message.params.query?.type === type);
  const attachAndAnswer = async () => {
    const before = requests("akra/kernel/query").length;
    const commands = requests("akra/kernel/command").length;
    const queried = when(() => requests("akra/kernel/query").length === before + 4);
    reply(requests("akra/kernel/attach").at(-1), { client_id: "1", protocol_version: 1, kernel: "codex-exec-loop-abc" });
    await queried;
    const refreshed = when(() => requests("akra/kernel/command").length === commands + 1);
    for (const request of requests("akra/kernel/query").slice(before)) reply(request, defaultAnswer(request.params.query?.type ?? ""));
    await refreshed;
    reply(requests("akra/kernel/command").at(-1), { processed: true });
  };
  return { connection, reply, fail, requests, query, attachAndAnswer, when, view: () => view };
}

describe("kernel connection", () => {
  it("replaces malformed evidence with unavailable instead of retaining an earlier Pass", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const attached = h.connection.show("/repo");
    await h.attachAndAnswer();
    await attached;
    h.connection.handleEvent({ event: { ...PLANNING, revision: 7, task_evidence: taskEvidenceFixture() } });
    expect(h.view().planning?.task_evidence?.tasks[0]?.criteria[0]?.state).toBe("pass");
    const malformed = taskEvidenceFixture();
    malformed.evidence_revision = Number.MAX_SAFE_INTEGER + 1;
    h.connection.handleEvent({ event: { ...PLANNING, revision: 7, task_evidence: malformed } });
    expect(h.view().planning?.task_evidence).toEqual({ unavailable: "invalid", tasks: [], truncated: false });
    expect(h.view().planning?.queue_summary).toBe("1 task");
    h.connection.handleEvent({ event: PLANNING });
    expect(h.view().planning).not.toHaveProperty("task_evidence");
  });

  it("attaches and reads the view without keying the read command", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const attached = h.connection.show("/repo/worktree");
    expect(h.requests("akra/kernel/attach")[0]?.params).toEqual({ workspace: "/repo/worktree", client_name: "akra-desktop" });
    expect(h.view().phase).toBe("attaching");
    await h.attachAndAnswer();
    await attached;
    expect(h.view()).toMatchObject({
      phase: "attached", kernel: "codex-exec-loop-abc", autoFollow: { max_turns: 3, paused: false },
      parallel: { enabled: false }, planning: { status: "ready_with_task", queue_summary: "1 task" },
    });
    expect(h.view().planning).not.toHaveProperty("type");
    expect(h.requests("akra/kernel/command").map((request) => request.params)).toEqual([{ command: { type: "refresh_planning" } }]);
  });

  it("keeps an event that arrived while the same query was in flight", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const attached = h.connection.show("/repo");
    const queried = h.when(() => h.requests("akra/kernel/query").length === 4);
    h.reply(h.requests("akra/kernel/attach")[0], { client_id: "1", protocol_version: 1, kernel: "repo" });
    await queried;
    const running = { thread_id: "thread-1", running: true, auto_follow_active: false };
    h.connection.handleEvent({ event: { type: "thread_activity", threads: [running] } });
    const refreshed = h.when(() => h.requests("akra/kernel/command").length === 1);
    for (const type of ["thread_activity", "auto_follow", "planning", "parallel"]) h.reply(h.query(type), defaultAnswer(type));
    await refreshed;
    h.reply(h.requests("akra/kernel/command")[0], { processed: true });
    await attached;
    expect(h.view().threads).toEqual([running]);
  });

  it("reads the whole view again after a resync", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    void h.connection.show("/repo");
    await h.attachAndAnswer();
    const before = h.requests("akra/kernel/query").length;
    h.connection.handleEvent({ event: { type: "resync" } });
    expect(h.requests("akra/kernel/query").length - before).toBe(4);
  });

  it("explains a refused attach when the TUI holds the repository", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const attached = h.connection.show("/repo");
    h.fail(h.requests("akra/kernel/attach")[0], -32012, "the repository kernel runs in another process (pid 42)");
    await attached;
    expect(h.view().phase).toBe("failed");
    expect(h.view().message).toContain("pid 42");
  });

  it("detaches when the panel closes and ignores later events", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    void h.connection.show("/repo");
    await h.attachAndAnswer();
    const closed = h.connection.show(null);
    h.connection.handleEvent({ event: { type: "auto_follow", max_turns: 9, paused: true } });
    expect(h.requests("akra/kernel/detach")).toHaveLength(1);
    h.reply(h.requests("akra/kernel/detach")[0], {});
    await closed;
    expect(h.view()).toEqual(EMPTY_KERNEL_VIEW);
  });

  it("reattaches and sends a keyless refresh after reconnection", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    void h.connection.show("/repo");
    await h.attachAndAnswer();
    h.connection.connectionChanged(false);
    expect(h.view()).toMatchObject({ phase: "idle", workspace: "/repo" });
    h.connection.connectionChanged(true);
    expect(h.requests("akra/kernel/attach")).toHaveLength(2);
    await h.attachAndAnswer();
    expect(h.view().phase).toBe("attached");
    expect(h.requests("akra/kernel/command").at(-1)?.params).toEqual({ command: { type: "refresh_planning" } });
  });

  it.each([
    ["codex-exec-loop-abc", { planning: { queue_summary: "1 task" } }],
    ["other-repo", { autoFollow: null, planning: null }],
  ])("retains prior state only for the same repository: %s", async (kernel, expected) => {
    const h = harness();
    h.connection.connectionChanged(true);
    void h.connection.show("/repo");
    await h.attachAndAnswer();
    void h.connection.show("/other");
    expect(h.view()).toMatchObject({
      phase: "attaching", workspace: "/other", kernel: "codex-exec-loop-abc", autoFollow: { max_turns: 3 },
    });
    const attached = h.when(() => h.view().phase === "attached");
    h.reply(h.requests("akra/kernel/attach").at(-1), { client_id: "2", protocol_version: 1, kernel });
    await attached;
    expect(h.view()).toMatchObject({ phase: "attached", kernel, ...expected });
  });

  it("stays attached when reading the view fails after attach", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const attached = h.connection.show("/repo");
    const queried = h.when(() => h.requests("akra/kernel/query").length === 4);
    h.reply(h.requests("akra/kernel/attach")[0], { client_id: "1", protocol_version: 1, kernel: "repo" });
    await queried;
    h.fail(h.query("planning"), -32013, "the kernel did not answer in time");
    for (const type of ["thread_activity", "auto_follow", "parallel"]) h.reply(h.query(type), defaultAnswer(type));
    await attached;
    expect(h.view().phase).toBe("attached");
    expect(h.view().message).toContain("did not answer in time");
  });

  it("reports a failed resync read", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    void h.connection.show("/repo");
    await h.attachAndAnswer();
    const before = h.requests("akra/kernel/query").length;
    const failed = h.when(() => h.view().message !== null);
    h.connection.handleEvent({ event: { type: "resync" } });
    for (const request of h.requests("akra/kernel/query").slice(before)) h.fail(request, -32012, "the kernel stopped");
    await failed;
    expect(h.view()).toMatchObject({ phase: "attached", message: expect.stringContaining("the kernel stopped") });
  });

  it("ignores a late attach answer for a folder the panel left", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const first = h.connection.show("/first");
    void h.connection.show("/second");
    h.reply(h.requests("akra/kernel/attach")[0], { client_id: "1", protocol_version: 1, kernel: "first" });
    await first;
    expect(h.view()).toMatchObject({ phase: "attaching", workspace: "/second" });
    const attached = h.when(() => h.view().phase === "attached");
    h.reply(h.requests("akra/kernel/attach")[1], { client_id: "2", protocol_version: 1, kernel: "second" });
    await attached;
    expect(h.view()).toMatchObject({ phase: "attached", kernel: "second" });
  });
});

describe("explicit parallel execution control", () => {
  it("only sends an explicit action, waits for state, and keeps newer events over a stale read", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const attached = h.connection.show("/repo");
    await h.attachAndAnswer();
    await attached;
    expect(h.requests("akra/kernel/command")).toHaveLength(1);

    const before = h.requests("akra/kernel/query").length;
    const started = h.connection.setParallelEnabled(true);
    const command = h.requests("akra/kernel/command").at(-1);
    expect(command?.params).toMatchObject({ command: { type: "enable_parallel" }, idempotency_key: expect.any(String) });
    expect(h.view().parallel?.enabled).toBe(false);
    expect(h.view().parallelControl.pending).toBe("enable");
    await h.connection.setParallelEnabled(false);
    expect(h.requests("akra/kernel/command")).toHaveLength(2);

    h.reply(command, { processed: true, operation: { op_id: 41, state: "accepted", next: { query: "history", after: 41 } } });
    await h.when(() => h.requests("akra/kernel/query").length === before + 1);
    expect(h.view().parallel?.enabled).toBe(false);
    expect(h.view().parallelControl.notice).toContain("접수");
    h.connection.handleEvent({ event: { type: "parallel", enabled: true, status: "ready" } });
    h.reply(h.query("parallel"), { type: "parallel", enabled: false, status: "old" });
    await started;
    expect(h.view().parallel).toMatchObject({ enabled: true, status: "ready" });
    expect(h.view().parallelControl).toMatchObject({ pending: null, error: null, uncertain: false });
  });

  it("shows a readiness failure after acceptance without claiming that execution started", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const attached = h.connection.show("/repo");
    await h.attachAndAnswer();
    await attached;
    const before = h.requests("akra/kernel/query").length;
    const started = h.connection.setParallelEnabled(true);
    h.reply(h.requests("akra/kernel/command").at(-1), { processed: true });
    await h.when(() => h.requests("akra/kernel/query").length === before + 1);
    h.reply(h.query("parallel"), { type: "parallel", enabled: false, status: "public repository requires parent opt-in" });
    await started;
    expect(h.view().parallel).toMatchObject({ enabled: false, status: "public repository requires parent opt-in" });
    expect(h.view().parallelControl.notice).toContain("접수");
    expect(h.requests("akra/kernel/command")).toHaveLength(2);
  });

  it.each([
    { processed: false, rejection: { reason: "kernel_unreachable", message: "runtime unavailable", next: [] } },
    { processed: true, failure: "stop failed" },
    { processed: false },
  ])("does not infer disabled state from a failed command: %j", async (response) => {
    const h = harness();
    h.connection.connectionChanged(true);
    const attached = h.connection.show("/repo");
    await h.attachAndAnswer();
    await attached;
    h.connection.handleEvent({ event: { type: "parallel", enabled: true } });
    const stopped = h.connection.setParallelEnabled(false);
    h.reply(h.requests("akra/kernel/command").at(-1), response);
    await stopped;
    expect(h.view().parallel?.enabled).toBe(true);
    expect(h.view().parallelControl).toMatchObject({ pending: null, uncertain: true });
    expect(h.view().parallelControl.error).toContain("확인하지 못했습니다");
    await h.connection.setParallelEnabled(false);
    expect(h.requests("akra/kernel/command")).toHaveLength(2);
    h.connection.handleEvent({ event: { type: "parallel", enabled: false } });
    expect(h.view().parallelControl.uncertain).toBe(false);
    expect(h.view().parallelControl.error).toBeNull();
    expect(h.view().parallel?.enabled).toBe(false);
  });

  it("marks a lost state read as uncertain and never automatically repeats the accepted action", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const attached = h.connection.show("/repo");
    await h.attachAndAnswer();
    await attached;
    const before = h.requests("akra/kernel/query").length;
    const started = h.connection.setParallelEnabled(true);
    h.reply(h.requests("akra/kernel/command").at(-1), { processed: true });
    await h.when(() => h.requests("akra/kernel/query").length === before + 1);
    h.fail(h.query("parallel"), -32012, "state read interrupted");
    await started;
    expect(h.view().parallel?.enabled).toBe(false);
    expect(h.view().parallelControl).toMatchObject({ pending: null, uncertain: true });
    expect(h.view().parallelControl.error).toContain("state read interrupted");
    expect(h.requests("akra/kernel/command")).toHaveLength(2);
  });

  it("does not let an older overview clear uncertainty after a command is rejected out of order", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const attached = h.connection.show("/repo");
    await h.attachAndAnswer();
    await attached;
    const before = h.requests("akra/kernel/query").length;
    const refreshing = h.connection.refresh();
    const oldReads = h.requests("akra/kernel/query").slice(before);
    expect(oldReads).toHaveLength(4);

    const started = h.connection.setParallelEnabled(true);
    // The Gateway rejects a full request queue immediately, before older queued reads finish.
    h.fail(h.requests("akra/kernel/command").at(-1), -32012, "too many kernel requests are waiting");
    await started;
    expect(h.view().parallelControl.uncertain).toBe(true);
    for (const request of oldReads) h.reply(request, defaultAnswer(request.params.query?.type ?? ""));
    await h.when(() => h.requests("akra/kernel/command").length === 3);
    h.reply(h.requests("akra/kernel/command").at(-1), { processed: true });
    await refreshing;

    expect(h.view().parallel?.enabled).toBe(false);
    expect(h.view().parallelControl).toMatchObject({ pending: null, uncertain: true });
    expect(h.view().parallelControl.error).toContain("too many kernel requests are waiting");
    await h.connection.setParallelEnabled(true);
    expect(h.requests("akra/kernel/command")).toHaveLength(3);
  });

  it.each(["event", "resync"] as const)("ignores a late state-read failure after a newer %s confirms state", async (source) => {
    const h = harness();
    h.connection.connectionChanged(true);
    const attached = h.connection.show("/repo");
    await h.attachAndAnswer();
    await attached;
    const before = h.requests("akra/kernel/query").length;
    const started = h.connection.setParallelEnabled(true);
    h.reply(h.requests("akra/kernel/command").at(-1), { processed: true });
    await h.when(() => h.requests("akra/kernel/query").length === before + 1);
    const commandRead = h.query("parallel");
    if (source === "event") {
      h.connection.handleEvent({ event: { type: "parallel", enabled: true, status: "ready" } });
    } else {
      h.connection.handleEvent({ event: { type: "resync" } });
      for (const request of h.requests("akra/kernel/query").slice(before + 1)) {
        h.reply(request, request.params.query?.type === "parallel"
          ? { type: "parallel", enabled: true, status: "ready" } : defaultAnswer(request.params.query?.type ?? ""));
      }
      await h.when(() => h.view().parallel?.enabled === true);
    }
    h.fail(commandRead, -32012, "outdated state read failed");
    await started;
    expect(h.view().parallel).toMatchObject({ enabled: true, status: "ready" });
    expect(h.view().parallelControl).toMatchObject({ pending: null, uncertain: false, error: null });
  });

  it("keeps state uncertain when resync supersedes the command read and then fails", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const attached = h.connection.show("/repo");
    await h.attachAndAnswer();
    await attached;
    const before = h.requests("akra/kernel/query").length;
    const started = h.connection.setParallelEnabled(true);
    h.reply(h.requests("akra/kernel/command").at(-1), { processed: true });
    await h.when(() => h.requests("akra/kernel/query").length === before + 1);
    const commandRead = h.query("parallel");
    h.connection.handleEvent({ event: { type: "resync" } });
    const resyncReads = h.requests("akra/kernel/query").slice(before + 1);
    expect(resyncReads).toHaveLength(4);
    for (const request of resyncReads) {
      if (request.params.query?.type === "parallel") h.fail(request, -32012, "resync state unavailable");
      else h.reply(request, defaultAnswer(request.params.query?.type ?? ""));
    }
    await h.when(() => h.view().message?.includes("resync state unavailable") === true);
    h.reply(commandRead, { type: "parallel", enabled: true });
    await started;
    expect(h.view().parallel?.enabled).toBe(false);
    expect(h.view().parallelControl).toMatchObject({ pending: null, uncertain: true });
    await h.connection.setParallelEnabled(true);
    expect(h.requests("akra/kernel/command")).toHaveLength(2);
    h.connection.handleEvent({ event: { type: "parallel", enabled: false, status: "entry blocked" } });
    expect(h.view().parallelControl.uncertain).toBe(false);
  });

  it("discards a command reply after changing repositories and does not replay it", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const attached = h.connection.show("/repo");
    await h.attachAndAnswer();
    await attached;
    const started = h.connection.setParallelEnabled(true);
    const command = h.requests("akra/kernel/command").at(-1);
    const changed = h.connection.show("/other-repo");
    h.reply(command, { processed: true });
    await started;
    expect(h.view().parallelControl).toMatchObject({ pending: null, notice: null, error: null });
    await h.attachAndAnswer();
    await changed;
    expect(h.view().parallel?.enabled).toBe(false);
    expect(h.requests("akra/kernel/command").filter((request) =>
      (request.params.command as { type: string })?.type === "enable_parallel")).toHaveLength(1);
  });
});
