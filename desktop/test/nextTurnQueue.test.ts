import { afterEach, describe, expect, it, vi } from "vitest";
import { NextTurnQueue, QUEUED_INPUT_TIMEOUT_MS } from "../src/renderer/session/nextTurnQueue";

const settle = async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); };
function harness() {
  const pending: Array<{ threadId: string; text: string; resolve: (turnId: string) => void; reject: (error: Error) => void }> = [];
  let activeThread = "t1";
  const queue = new NextTurnQueue((threadId, text) => new Promise<string>((resolve, reject) => {
    pending.push({ threadId, text, resolve, reject });
  }), (threadId) => activeThread === threadId);
  return { queue, pending, select: (threadId: string) => { activeThread = threadId; } };
}
afterEach(() => vi.useRealTimers());

describe("window next-turn queue", () => {
  it("waits for the exact turn then serializes reservations without steering or duplicate sends", async () => {
    const { queue, pending } = harness();
    queue.enqueue("t1", "first", "active");
    queue.enqueue("t1", "second", "active");
    queue.terminal("t1", "unrelated", "completed");
    expect(pending).toHaveLength(0);
    queue.terminal("t1", "active", "completed");
    queue.terminal("t1", "active", "completed");
    expect(pending).toHaveLength(1);
    pending[0]!.resolve("next");
    await settle();
    expect(queue.snapshot("t1").waitingTurnId).toBe("next");
    queue.terminal("t1", "active", "completed");
    expect(pending).toHaveLength(1);
    queue.terminal("t1", "next", "completed");
    expect(pending.map((value) => value.text)).toEqual(["first", "second"]);
    pending[1]!.resolve("last");
    await settle();
    queue.dispose();
  });

  it("correlates a terminal received before its start response", async () => {
    const { queue, pending } = harness();
    queue.enqueue("t1", "first", null);
    queue.enqueue("t1", "second", null);
    queue.terminal("t1", "early", "completed");
    expect(pending).toHaveLength(1);
    pending[0]!.resolve("early");
    await settle();
    expect(pending).toHaveLength(2);
    pending[1]!.resolve("last");
    await settle();
    queue.dispose();
  });

  it.each(["failed", "interrupted"] as const)("pauses on %s and requires explicit continuation", async (status) => {
    const { queue, pending } = harness();
    queue.enqueue("t1", "after", "active");
    queue.terminal("t1", "active", status);
    expect(pending).toHaveLength(0);
    expect(queue.snapshot("t1").paused).not.toBeNull();
    queue.resume("t1", null);
    expect(pending).toHaveLength(1);
    pending[0]!.resolve("last");
    await settle();
    queue.dispose();
  });

  it.each(["completed", "failed", "interrupted"] as const)("resumes explicitly after %s even while the transcript still reports that turn active", async (status) => {
    const { queue, pending } = harness();
    queue.enqueue("t1", "after terminal", "active");
    queue.pause("t1", "paused for review");
    queue.terminal("t1", "active", status);
    expect(pending).toHaveLength(0);
    queue.resume("t1", "active");
    expect(pending).toHaveLength(1);
    expect(queue.snapshot("t1").paused).toBeNull();
    pending[0]!.resolve("next");
    await settle();
    queue.terminal("t1", "active", status);
    expect(pending).toHaveLength(1);
    expect(queue.snapshot("t1").waitingTurnId).toBe("next");
    queue.dispose();
  });

  it("keeps waiting when explicit resume names a turn that has not terminated", async () => {
    const { queue, pending } = harness();
    queue.enqueue("t1", "after active", "active");
    queue.pause("t1", "paused for review");
    queue.resume("t1", "active");
    expect(pending).toHaveLength(0);
    expect(queue.snapshot("t1").waitingTurnId).toBe("active");
    queue.terminal("t1", "active", "completed");
    expect(pending).toHaveLength(1);
    pending[0]!.resolve("next");
    await settle();
    queue.dispose();
  });

  it("keeps reservations on their original thread and holds them across navigation", async () => {
    const { queue, pending, select } = harness();
    queue.enqueue("t1", "belongs to first", "active");
    queue.pauseAll("switched");
    select("t2");
    queue.terminal("t1", "active", "completed");
    queue.enqueue("t2", "second thread", "other");
    expect(pending).toHaveLength(0);
    select("t1");
    queue.resume("t1", null);
    expect(pending[0]).toMatchObject({ threadId: "t1", text: "belongs to first" });
    pending[0]!.resolve("last");
    await settle();
    queue.dispose();
  });

  it("edits, removes and reorders only unsent inputs", () => {
    const { queue } = harness();
    queue.enqueue("t1", "first", "active");
    queue.enqueue("t1", "second", "active");
    const [first, second] = queue.snapshot("t1").items;
    queue.edit("t1", second!.id, "edited");
    queue.move("t1", second!.id, -1);
    expect(queue.snapshot("t1").items.map((value) => value.text)).toEqual(["edited", "first"]);
    queue.remove("t1", first!.id);
    expect(queue.snapshot("t1").items).toHaveLength(1);
    queue.dispose();
  });

  it("holds unknown delivery and does not retry on reconnect or late response", async () => {
    vi.useFakeTimers();
    const { queue, pending } = harness();
    queue.enqueue("t1", "maybe sent", null);
    queue.enqueue("t1", "later", null);
    await vi.advanceTimersByTimeAsync(QUEUED_INPUT_TIMEOUT_MS);
    expect(queue.snapshot("t1").items[0]?.state).toBe("unknown");
    expect(() => queue.resume("t1", null)).toThrow("결과 불명");
    pending[0]!.resolve("late");
    queue.terminal("t1", "late", "completed");
    await settle();
    expect(pending).toHaveLength(1);
    queue.dispose();
  });

  it("does not dispatch or restore state after the window is disposed", async () => {
    const { queue, pending } = harness();
    queue.enqueue("t1", "first", null);
    queue.enqueue("t1", "second", null);
    queue.dispose();
    pending[0]!.resolve("late");
    queue.terminal("t1", "late", "completed");
    await settle();
    expect(pending).toHaveLength(1);
    expect(queue.snapshot("t1").items).toEqual([]);
  });

  it("bounds reservations and distinguishes same thread ids on independent hosts", () => {
    const first = harness(); const second = harness();
    first.queue.enqueue("t1", "host one", "active");
    expect(second.queue.snapshot("t1").items).toEqual([]);
    expect(() => first.queue.enqueue("t1", " ", "active")).toThrow();
    expect(() => first.queue.enqueue("t1", "x".repeat(32_001), "active")).toThrow();
    for (let index = 1; index < 20; index++) first.queue.enqueue("t1", String(index), "active");
    expect(() => first.queue.enqueue("t1", "overflow", "active")).toThrow("20개");
    first.queue.dispose(); second.queue.dispose();
  });
});
