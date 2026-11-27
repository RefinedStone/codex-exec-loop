import { afterEach, expect, it, vi } from "vitest";
import { captureSelection, checkBundle } from "./fileAttachment";
import { saveDraft, storedDraftRecords } from "../session/draftStorage";
import type { GitFileDiff } from "../../shared/bridge";

const target = { hostId: "local", worktree: "/work/project", threadId: "thread" };
const source = (text: string): GitFileDiff => ({ kind: "content", truncated: false, text,
  snapshot: { source: "worktree_file", snapshotId: "snapshot", headOid: "commit", originalPath: null,
    contentStatus: "complete", path: "file.txt", worktreeId: target.worktree, repositoryId: "repo" } });
const capture = (text: string, endLine = 1) => captureSelection(source(text), { startLine: 1, endLine },
  { target, turnId: null, manifestId: "manifest", draftRevision: 1 });
afterEach(() => vi.unstubAllGlobals());

it("rejects a multibyte selection or cumulative content over the byte budget without shortening it", async () => {
  await expect(capture("가".repeat(5600))).rejects.toThrow();
  const item = await capture("가".repeat(5200));
  expect(() => checkBundle([item, item, item, item])).toThrow();
  expect(() => checkBundle([item, item, item, { ...item, included: false }])).not.toThrow();
});

it("rejects an oversized physical range even when every line is only a newline", async () => {
  await expect(capture("\n".repeat(101), 101)).rejects.toThrow();
});

it("preserves the last saved version on oversize edits or quota refusal after legacy migration", async () => {
  const data = new Map([["akra.drafts", JSON.stringify({ thread: "original" })]]);
  let full = false;
  vi.stubGlobal("window", { localStorage: {
    get length() { return data.size; }, key: (i: number) => [...data.keys()][i] ?? null,
    getItem: (k: string) => data.get(k) ?? null, removeItem: (k: string) => data.delete(k),
    setItem(k: string, v: string) { if (full) throw new DOMException("full", "QuotaExceededError"); data.set(k, v); },
  } });
  expect(storedDraftRecords()[0]?.text).toBe("original");
  const item = await capture("saved\n");
  expect(saveDraft("thread", "updated", [item])).toBe(true);
  expect(saveDraft("thread", "x".repeat(20_001))).toBe(false);
  full = true;
  expect(saveDraft("thread", "not saved")).toBe(false);
  expect(storedDraftRecords()).toMatchObject([{ key: "thread", text: "updated", attachments: [item] }]);
});
