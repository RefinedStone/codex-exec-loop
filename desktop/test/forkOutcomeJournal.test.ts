import { describe, expect, it } from "vitest";
import { ForkOutcomeJournal } from "../src/renderer/session/forkOutcomeJournal";

function fixture() {
  const data = new Map<string, string>();
  const storage = {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => { data.set(key, value); },
    removeItem: (key: string) => { data.delete(key); },
  };
  return { data, storage };
}

describe("fork outcome journal", () => {
  it("records uncertainty before the RPC and keeps it on reload until a known outcome", () => {
    const { storage } = fixture();
    const first = new ForkOutcomeJournal(() => storage);
    const id = first.begin("source");
    const restarted = new ForkOutcomeJournal(() => storage);
    expect(restarted.has("source")).toBe(true);
    expect(restarted.has("other")).toBe(false);
    first.resolve("source", id);
    expect(restarted.has("source")).toBe(false);
  });

  it("cannot erase a newer attempt with an older response", () => {
    const { storage } = fixture();
    const first = new ForkOutcomeJournal(() => storage);
    const second = new ForkOutcomeJournal(() => storage);
    const old = first.begin("source");
    const current = second.begin("source");
    first.resolve("source", old);
    expect(new ForkOutcomeJournal(() => storage).has("source")).toBe(true);
    second.resolve("source", current);
    expect(first.has("source")).toBe(false);
  });

  it("fails closed when the storage write is unavailable", () => {
    const journal = new ForkOutcomeJournal(() => { throw new Error("storage unavailable"); });
    expect(journal.has("source")).toBe(true);
    expect(() => journal.begin("source")).toThrow("storage unavailable");
  });
});
