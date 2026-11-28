import { describe, expect, it } from "vitest";
import { attentionPreferenceKey, isAttentionPreferenceEvent, readAttentionPreference, writeAttentionPreference } from "../src/renderer/kernel/attentionPreferences";
import type { AttentionItem } from "../src/shared/kernel-protocol/generated";
const item: AttentionItem = { id: "task:1", revision: "3", cause: "task_proposed", subject: { kind: "task", id: "1" }, summary: "task", next: [] };

describe("local attention preferences", () => {
  it("keeps independent fields, kernel and revision scoped across readers", () => {
    const data = new Map<string, string>();
    const storage = { getItem: (key: string) => data.get(key) ?? null, setItem: (key: string, value: string) => { data.set(key, value); }, removeItem: (key: string) => { data.delete(key); } };
    expect(writeAttentionPreference(storage, "repo", item, "read", true)).toBe(true);
    expect(writeAttentionPreference(storage, "repo", item, "pinned", true)).toBe(true);
    expect(writeAttentionPreference(storage, "repo", item, "snoozedUntil", 123)).toBe(true);
    expect(readAttentionPreference(storage, "repo", item)).toEqual({ read: true, pinned: true, snoozedUntil: 123 });
    expect(readAttentionPreference(storage, "other", item).read).toBe(false);
    expect(readAttentionPreference(storage, "repo", { ...item, revision: "4" }).pinned).toBe(false);
    writeAttentionPreference(storage, "repo", item, "read", false);
    expect(readAttentionPreference(storage, "repo", item)).toEqual({ read: false, pinned: true, snoozedUntil: 123 });
    expect(isAttentionPreferenceEvent(attentionPreferenceKey("repo", item, "read"))).toBe(true);
  });
  it("does not claim persistence when local storage fails", () => {
    const storage = { setItem() { throw new Error("quota"); }, removeItem() { throw new Error("quota"); } };
    expect(writeAttentionPreference(storage, "repo", item, "read", true)).toBe(false);
  });
});
