import { describe, expect, it } from "vitest";
import { newAttentionCount, nextAttentionThread } from "../src/renderer/kernel/attentionNavigation";
import type { AttentionItem } from "../src/shared/kernel-protocol/generated";
const item = (id: string, subject = id): AttentionItem => ({ id, revision: "1", cause: "approval_pending", subject: { kind: "thread", id: subject }, summary: id, next: [] });
describe("attention navigation", () => {
  it("moves in authoritative order, wraps, deduplicates and ignores non-thread items", () => {
    const rows = [item("a"), item("a2", "a"), { ...item("x"), subject: { kind: "task", id: "x" } } as AttentionItem, item("b")];
    expect(nextAttentionThread(rows, null)).toBe("a");
    expect(nextAttentionThread(rows, "a")).toBe("b");
    expect(nextAttentionThread(rows, "b")).toBe("a");
    expect(nextAttentionThread([], "a")).toBeNull();
  });
  it("does not toast all items on first attach or failed read, counts unique changed revisions", () => {
    expect(newAttentionCount(null, [item("a")])).toBe(0);
    expect(newAttentionCount([item("a")], null)).toBe(0);
    expect(newAttentionCount([item("a")], [item("a"), item("b"), item("b")])).toBe(1);
    expect(newAttentionCount([item("a")], [{ ...item("a"), revision: "2" }])).toBe(1);
  });
});
