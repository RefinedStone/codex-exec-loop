import { describe, expect, it, vi } from "vitest";
import { EMPTY_FORK_SNAPSHOT, ForkOutcomeJournal } from "../src/renderer/session/forkOutcomeJournal";

function fixture() {
  const data = new Map<string, string>();
  const storage = {
    get length() { return data.size; },
    key: (index: number) => [...data.keys()][index] ?? null,
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => { data.set(key, value); },
    removeItem: (key: string) => { data.delete(key); },
  };
  const journal = () => new ForkOutcomeJournal(() => storage);
  return { data, storage, journal };
}

describe("fork outcome journal", () => {
  it("records uncertainty before the RPC and keeps it on reload until a known outcome", async () => {
    const { journal } = fixture();
    const first = journal();
    const id = (await first.begin("source"))!;
    const restarted = journal();
    expect(restarted.has("source")).toBe(true);
    expect(restarted.has("other")).toBe(false);
    first.resolve("source", id);
    expect(restarted.has("source")).toBe(false);
  });

  it("retains an earlier uncertain attempt when a later retry has a definitive result", async () => {
    const { journal } = fixture();
    const first = journal();
    const second = journal();
    const old = (await first.begin("source"))!;
    expect(await second.begin("source")).toBeNull();
    const current = (await second.begin("source", second.snapshot("source")))!;
    expect(current).not.toBe(old);
    second.resolve("source", current);
    expect(journal().has("source")).toBe(true);
    first.resolve("source", old);
    expect(journal().has("source")).toBe(false);
  });

  it("cannot erase a newer attempt with an older response", async () => {
    const { journal } = fixture();
    const first = journal();
    const second = journal();
    const old = (await first.begin("source"))!;
    const current = (await second.begin("source", second.snapshot("source")))!;
    first.resolve("source", old);
    expect(journal().has("source")).toBe(true);
    second.resolve("source", current);
    expect(first.has("source")).toBe(false);
  });

  it("rejects changed approval snapshots, including a concurrent admission", async () => {
    const { journal } = fixture();
    const first = journal();
    const second = journal();
    const approved = second.snapshot("source");
    expect(approved).toBe(EMPTY_FORK_SNAPSHOT);
    const outcomes = await Promise.all([first.begin("source", approved), second.begin("source", approved)]);
    expect(outcomes.filter(Boolean)).toHaveLength(1);
    expect(outcomes.filter(id => id === null)).toHaveLength(1);
    expect(journal().has("source")).toBe(true);
  });

  it("preserves legacy uncertainty and isolates similar source ids", async () => {
    const { data, journal } = fixture();
    data.set("akra.fork-pending.v1:source", "old");
    const first = journal();
    expect(first.has("source")).toBe(true);
    expect(first.has("source:other")).toBe(false);
    const id = (await first.begin("source", first.snapshot("source")))!;
    first.resolve("source", id);
    expect(journal().has("source")).toBe(true);
  });

  it("fails closed when the storage is unavailable", async () => {
    const journal = new ForkOutcomeJournal(() => { throw new Error("storage unavailable"); });
    expect(journal.has("source")).toBe(true);
    await expect(journal.begin("source")).rejects.toThrow("storage unavailable");
  });
});

it("refuses a renderer admission without cross-window locks and writes no marker", async () => {
  const { storage, data } = fixture();
  vi.stubGlobal("window", { localStorage: storage, navigator: {} });
  try {
    const journal = new ForkOutcomeJournal();
    await expect(journal.begin("source")).rejects.toThrow("admission lock unavailable");
    expect(data.size).toBe(0);
  } finally {
    vi.unstubAllGlobals();
  }
});
