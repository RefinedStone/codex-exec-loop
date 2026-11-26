import { afterEach, describe, expect, it, vi } from "vitest";
import { JsonRpcError } from "../src/renderer/rpc/jsonRpcClient";
import { COMPACTION_RESPONSE_TIMEOUT_MS, COMPACTION_COMPLETION_TIMEOUT_MS, ThreadCompactionController, type CompactionContext } from "../src/renderer/session/threadCompaction";

const controllers: ThreadCompactionController[] = [];
afterEach(() => { for (const controller of controllers.splice(0)) controller.dispose(); vi.useRealTimers(); });
const flush = async () => { await Promise.resolve(); await Promise.resolve(); };
const compactItem = { type: "contextCompaction", id: "compact-item-1" };
const turn = (id: string, status: string, items: unknown[] = []) => ({ id, status, items });

function fixture(reload: (threadId: string, turnId: string, status: string) => Promise<boolean> = async () => true) {
  const context: CompactionContext = { threadId: "thread-a", connected: true, writable: true, loaded: true,
    turnInProgress: false, submissionInFlight: false, knownTurnIds: ["old-turn"] };
  const calls: Array<{ method: string; params: unknown; resolve(value: unknown): void; reject(error: unknown): void }> = [];
  const controller = new ThreadCompactionController({ context: () => context, reload, request<T>(method: string, params: unknown) {
    return new Promise<T>((resolve, reject) => calls.push({ method, params, resolve: value => resolve(value as T), reject }));
  } });
  controllers.push(controller);
  const notify = (method: string, params: Record<string, unknown> = {}) => controller.observe(method, { threadId: "thread-a", turnId: "compact-turn", ...params });
  return { controller, context, calls, notify };
}

describe("engine manual compaction contract", () => {
  it("treats an empty response only as acceptance and waits for item evidence plus terminal success", async () => {
    const f = fixture();
    f.controller.start("thread-a");
    expect(f.calls.map(call => [call.method, call.params])).toEqual([["thread/compact/start", { threadId: "thread-a" }]]);
    f.calls[0]!.resolve({}); await flush();
    expect(f.controller.snapshot("thread-a").phase).toBe("running");
    f.notify("turn/started", { turn: turn("compact-turn", "inProgress") });
    f.notify("item/started", { item: compactItem });
    f.notify("item/completed", { item: compactItem });
    expect(f.controller.isBlocking("thread-a")).toBe(true);
    f.notify("turn/completed", { turn: turn("compact-turn", "completed") });
    expect(f.controller.snapshot("thread-a").phase).toBe("completed");
    expect(f.controller.isBlocking("thread-a")).toBe(false);
  });

  it("does not overwrite observed completion with a later response", async () => {
    const f = fixture(); f.controller.start("thread-a");
    f.notify("item/completed", { item: compactItem });
    f.notify("turn/completed", { turn: turn("compact-turn", "completed") });
    f.calls[0]!.resolve({}); await flush();
    expect(f.controller.snapshot("thread-a").phase).toBe("completed");
  });

  it("does not call the model boundary for active, queued, read-only, unloaded or offline contexts", () => {
    for (const patch of [{ turnInProgress: true }, { submissionInFlight: true }, { writable: false }, { loaded: false }, { connected: false }]) {
      const f = fixture(); Object.assign(f.context, patch);
      expect(() => f.controller.start("thread-a")).toThrow();
      expect(f.calls).toHaveLength(0);
    }
    const f = fixture(); f.controller.start("thread-a");
    expect(() => f.controller.start("thread-a")).toThrow("결과");
    expect(f.calls).toHaveLength(1);
  });

  it("ignores other threads, previous turns and mismatched compaction item IDs", async () => {
    const f = fixture(); f.controller.start("thread-a"); f.calls[0]!.resolve({}); await flush();
    f.notify("item/completed", { threadId: "thread-b", item: compactItem });
    f.notify("item/completed", { turnId: "old-turn", item: compactItem });
    f.notify("item/started", { item: compactItem });
    f.notify("item/completed", { item: { ...compactItem, id: "different-item" } });
    f.notify("turn/completed", { turn: turn("compact-turn", "completed") });
    expect(f.controller.snapshot("thread-a").phase).toBe("unknown");
  });

  it("blocks immediately on active notifications before the transcript frame is updated", () => {
    const f = fixture();
    f.notify("turn/started", { turn: turn("active-turn", "inProgress"), turnId: "active-turn" });
    expect(() => f.controller.start("thread-a")).toThrow("진행 중");
    f.notify("turn/completed", { turn: turn("different-turn", "completed"), turnId: "different-turn" });
    expect(() => f.controller.start("thread-a")).toThrow("진행 중");
    f.notify("turn/completed", { turn: turn("active-turn", "completed"), turnId: "active-turn" });
    f.notify("thread/status/changed", { status: { type: "active", activeFlags: [] } });
    expect(() => f.controller.start("thread-a")).toThrow("진행 중");
    // 일부 엔진/재접속에서 turn/started와 후속 idle 알림 없이 terminal만 관측할 수 있다.
    f.notify("turn/completed", { turn: turn("status-only-turn", "completed"), turnId: "status-only-turn" });
    f.controller.start("thread-a");
    expect(f.calls).toHaveLength(1);
  });

  it("reports failure even when an item completed before the turn failed", () => {
    const f = fixture(); f.controller.start("thread-a");
    f.notify("item/completed", { item: compactItem });
    f.notify("turn/completed", { turn: turn("compact-turn", "failed", [compactItem]) });
    expect(f.controller.snapshot("thread-a").phase).toBe("failed");
    expect(f.controller.isBlocking("thread-a")).toBe(false);
  });

  it("distinguishes unsupported methods, engine rejection and transport ambiguity", async () => {
    for (const [error, expected] of [[new JsonRpcError(-32601, "unknown method"), "unsupported"], [new JsonRpcError(-32602, "bad state"), "failed"], [new Error("connection lost"), "unknown"]] as const) {
      const f = fixture(); f.controller.start("thread-a"); f.calls[0]!.reject(error); await flush();
      expect(f.controller.snapshot("thread-a").phase).toBe(expected);
    }
  });

  it("ends response/completion waits with unknown and never automatically repeats a model request", async () => {
    vi.useFakeTimers();
    const f = fixture(); f.controller.start("thread-a");
    await vi.advanceTimersByTimeAsync(COMPACTION_RESPONSE_TIMEOUT_MS);
    expect(f.controller.snapshot("thread-a").phase).toBe("unknown");
    expect(f.calls).toHaveLength(1);
    f.calls[0]!.resolve({}); await flush();
    expect(f.controller.snapshot("thread-a").phase).toBe("running");
    await vi.advanceTimersByTimeAsync(COMPACTION_COMPLETION_TIMEOUT_MS);
    expect(f.controller.snapshot("thread-a").phase).toBe("unknown");
    expect(f.calls).toHaveLength(1);
    expect(f.controller.isBlocking("thread-a")).toBe(true);
  });

  it("ignores old acknowledgements after disconnect and recovers only through a matching read", async () => {
    const f = fixture(); f.controller.start("thread-a");
    f.controller.connectionChanged(false); f.controller.connectionChanged(true);
    f.calls[0]!.resolve({}); await flush();
    f.notify("item/completed", { item: compactItem });
    expect(f.controller.snapshot("thread-a").phase).toBe("unknown");
    const checking = f.controller.refresh("thread-a");
    expect(f.calls[1]).toMatchObject({ method: "thread/turns/list", params: { threadId: "thread-a", limit: 20, sortDirection: "desc", itemsView: "full" } });
    f.calls[1]!.resolve({ data: [turn("compact-turn", "completed", [compactItem]), turn("old-turn", "completed")] });
    await checking;
    expect(f.controller.snapshot("thread-a").phase).toBe("completed");
    expect(f.calls.filter(call => call.method === "thread/compact/start")).toHaveLength(1);
  });

  it("does not guess a result from an old or ambiguous compact turn", async () => {
    const f = fixture(); f.controller.start("thread-a"); f.controller.connectionChanged(false); f.controller.connectionChanged(true);
    const checking = f.controller.refresh("thread-a");
    f.calls[1]!.resolve({ data: [turn("unknown-old-turn", "completed", [compactItem])] });
    await checking;
    expect(f.controller.snapshot("thread-a").phase).toBe("unknown");
    expect(f.controller.isBlocking("thread-a")).toBe(true);
  });

  it("does not replace a known turn ID with a different compaction found in history", async () => {
    const f = fixture(); f.controller.start("thread-a");
    f.notify("turn/started", { turn: turn("compact-turn", "inProgress") });
    f.controller.connectionChanged(false); f.controller.connectionChanged(true);
    const checking = f.controller.refresh("thread-a");
    f.calls[1]!.resolve({ data: [turn("unrelated-compact", "completed", [compactItem]), turn("old-turn", "completed")] });
    await checking;
    expect(f.controller.snapshot("thread-a").phase).toBe("unknown");
    expect(f.controller.isBlocking("thread-a")).toBe(true);
  });

  it("keeps unknown until the freshly loaded current conversation confirms the compact result", async () => {
    const reload = vi.fn(async () => false);
    const f = fixture(reload); f.controller.start("thread-a");
    const checking = f.controller.refresh("thread-a");
    f.calls[1]!.resolve({ data: [turn("compact-turn", "completed", [compactItem]), turn("old-turn", "completed")] });
    await checking;
    expect(reload).toHaveBeenCalledWith("thread-a", "compact-turn", "completed");
    expect(f.controller.snapshot("thread-a").phase).toBe("unknown");
    expect(f.controller.isBlocking("thread-a")).toBe(true);
  });

  it("does not steer the completed compact turn while its transcript frame is still in progress", () => {
    const f = fixture(); f.controller.start("thread-a");
    f.context.knownTurnIds = ["old-turn", "compact-turn"]; f.context.turnInProgress = true;
    f.notify("item/completed", { item: compactItem });
    f.notify("turn/completed", { turn: turn("compact-turn", "completed") });
    expect(f.controller.snapshot("thread-a").phase).toBe("completed");
    expect(f.controller.isBlocking("thread-a")).toBe(true);
    f.context.turnInProgress = false;
    expect(f.controller.isBlocking("thread-a")).toBe(false);
  });

  it("accepts the deprecated notification only for an already observed current turn and its terminal result", () => {
    const f = fixture(); f.controller.start("thread-a");
    f.notify("thread/compacted");
    expect(f.controller.snapshot("thread-a").phase).toBe("requesting");
    f.notify("turn/started", { turn: turn("compact-turn", "inProgress") });
    f.notify("thread/compacted");
    f.notify("turn/completed", { turn: turn("compact-turn", "completed") });
    expect(f.controller.snapshot("thread-a").phase).toBe("completed");
  });
});
