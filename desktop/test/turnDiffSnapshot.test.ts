import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import type { FileUpdateChange } from "../src/shared/protocol/generated/v2/FileUpdateChange";
import type { ThreadItem } from "../src/shared/protocol/generated/v2/ThreadItem";
import { turnDiffSources, type TurnDiffFileSource } from "../src/renderer/conversation/diff";
import { applyNotification, type Transcript } from "../src/renderer/conversation/transcript";
import { captureTurnDiff, readTurnDiffContext, type TurnDiffScope, type TurnDiffSnapshot } from "../src/renderer/conversation/turnDiffSnapshot";

const patch = "@@ -10,2 +20,2 @@\n-OLD_TURN\n+NEW_TURN\n SHARED_TURN\n";
const scope: TurnDiffScope = { hostId: "fixture-host", threadId: "thread", turnId: "turn", cwd: "/fixture" };
const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true, maxRetries: 5 });
});
function source(change: FileUpdateChange): TurnDiffFileSource {
  const item: ThreadItem = { type: "fileChange", id: "change", status: "completed", changes: [change] };
  const entry = turnDiffSources(null, [item])[0];
  if (!entry) throw new Error("Missing fixture source.");
  return entry;
}
const update = (text = patch) => source({ path: "file.txt", kind: { type: "update", move_path: null }, diff: text });
async function captured(entry: TurnDiffFileSource, origin = scope): Promise<TurnDiffSnapshot> {
  const result = await captureTurnDiff(entry, origin);
  if (result.status !== "captured") throw new Error(result.reason);
  return result.snapshot;
}
const request = (snapshot: TurnDiffSnapshot, side: "old" | "new", first: number) =>
  ({ snapshotId: snapshot.snapshotId, side, startLine: first, endLine: first + 1 });

describe("last-turn source snapshots", () => {
  it("retains actual turn excerpts after later HEAD, index and working file edits", async () => {
    const root = mkdtempSync(join(tmpdir(), "akra-turn-source-")); roots.push(root);
    const env = { ...Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GIT_"))), GIT_MASTER: "1" };
    const git = (...args: string[]) => execFileSync("git", ["-c", "user.name=Tester", "-c",
      "user.email=fixture@localhost.invalid", "-c", "commit.gpgsign=false", ...args], { cwd: root, env });
    git("init", "-q");
    writeFileSync(join(root, "file.txt"), "HEAD_BEFORE\n");
    git("add", "."); git("commit", "-qm", "before");
    const snapshot = await captured(update(), { ...scope, cwd: root });

    writeFileSync(join(root, "file.txt"), "HEAD_AFTER\n"); git("add", "."); git("commit", "-qm", "after");
    writeFileSync(join(root, "file.txt"), "INDEX_AFTER\n"); git("add", ".");
    writeFileSync(join(root, "file.txt"), "DISK_AFTER\n");

    expect(readTurnDiffContext(snapshot, request(snapshot, "old", 10))).toMatchObject({ status: "available", source: "last_turn", text: "OLD_TURN\nSHARED_TURN\n" });
    expect(readTurnDiffContext(snapshot, request(snapshot, "new", 20))).toMatchObject({ status: "available", text: "NEW_TURN\nSHARED_TURN\n" });
    expect(snapshot.old.status).toBe("partial");
  });

  it.each([["old", 9], ["old", 12], ["new", 19], ["new", 22]] as const)(
    "returns unavailable for unprovided %s lines at %i", async (side, first) => {
      const snapshot = await captured(update());
      expect(readTurnDiffContext(snapshot, request(snapshot, side, first))).toEqual({ status: "unavailable", reason: "context_not_provided" });
    });

  it.each(["add", "delete"] as const)("preserves complete %s content including CRLF and final newline absence", async type => {
    const snapshot = await captured(source({ path: "file.txt", kind: { type }, diff: "first\r\nlast" }));
    const side = type === "add" ? "new" : "old";
    expect(snapshot[side]).toMatchObject({ status: "complete", lineCount: 2 });
    expect(readTurnDiffContext(snapshot, request(snapshot, side, 1))).toMatchObject({ text: "first\r\nlast" });
    expect(readTurnDiffContext(snapshot, request(snapshot, type === "add" ? "old" : "new", 1)))
      .toEqual({ status: "unavailable", reason: "context_not_provided" });
  });

  it("preserves each side's explicit no-newline marker", async () => {
    const snapshot = await captured(update("@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n"));
    expect(readTurnDiffContext(snapshot, { ...request(snapshot, "old", 1), endLine: 1 })).toMatchObject({ text: "old" });
    expect(readTurnDiffContext(snapshot, { ...request(snapshot, "new", 1), endLine: 1 })).toMatchObject({ text: "new" });
  });

  it("excludes an incomplete hunk while retaining an earlier verified hunk", async () => {
    const snapshot = await captured(update(patch + "@@ -30 +40,2 @@\n-incomplete old\n+incomplete new\n"));
    expect(readTurnDiffContext(snapshot, request(snapshot, "old", 10))).toMatchObject({ text: "OLD_TURN\nSHARED_TURN\n" });
    expect(readTurnDiffContext(snapshot, request(snapshot, "new", 40))).toEqual({ status: "unavailable", reason: "context_not_provided" });
  });

  it("does not invent coordinates for an unnumbered update", async () => {
    const snapshot = await captured(update("-old\n+new\n"));
    expect(snapshot.old.status).toBe("unavailable");
    expect(readTurnDiffContext(snapshot, request(snapshot, "new", 1))).toEqual({ status: "unavailable", reason: "context_not_provided" });
  });

  it("pins aggregate notifications independently from a later update of the same turn", async () => {
    const transcript: Transcript = { threadId: "thread", cwd: "/fixture", title: "", status: null, tokenUsage: null, olderTurnsCursor: null,
      turns: [{ id: "turn", status: "inProgress", errorMessage: null, items: [], diff: null, plan: null, startedAt: null, durationMs: null }] };
    const diff = "diff --git a/old.txt b/new.txt\nrename from old.txt\nrename to new.txt\n" + patch;
    const first = applyNotification(transcript, { method: "turn/diff/updated", params: { threadId: "thread", turnId: "turn", diff } });
    const entry = turnDiffSources(first.turns[0]?.diff ?? null, [])[0];
    if (!entry) throw new Error("Missing aggregate source.");
    const original = await captured(entry);
    const next = applyNotification(first, { method: "turn/diff/updated", params: { threadId: "thread", turnId: "turn", diff: diff.replace("NEW_TURN", "LATER_TURN") } });
    const newer = turnDiffSources(next.turns[0]?.diff ?? null, [])[0];
    if (!newer) throw new Error("Missing later aggregate source.");
    expect((await captured(newer)).snapshotId).not.toBe(original.snapshotId);
    expect(original).toMatchObject({ path: "new.txt", originalPath: "old.txt" });
    expect(readTurnDiffContext(original, request(original, "new", 20))).toMatchObject({ text: "NEW_TURN\nSHARED_TURN\n" });
  });

  it("binds snapshot identity to the host, thread and turn and rejects a mismatched request", async () => {
    const entry = update(), snapshot = await captured(entry);
    expect((await captured(entry, { cwd: scope.cwd, turnId: scope.turnId, threadId: scope.threadId, hostId: scope.hostId })).snapshotId)
      .toBe(snapshot.snapshotId);
    for (const changed of [{ ...scope, hostId: "other" }, { ...scope, threadId: "other" }, { ...scope, turnId: "other" }]) {
      expect((await captured(entry, changed)).snapshotId).not.toBe(snapshot.snapshotId);
    }
    expect(readTurnDiffContext(snapshot, { ...request(snapshot, "old", 10), snapshotId: "different" }))
      .toEqual({ status: "unavailable", reason: "snapshot_mismatch" });
  });

  it("keeps last-turn identity when a caller supplies extra request metadata", async () => {
    const snapshot = await captured(update());
    const input = { ...request(snapshot, "old", 10), source: "head_worktree", status: "unavailable" };
    expect(readTurnDiffContext(snapshot, input)).toMatchObject({
      status: "available", source: "last_turn", text: "OLD_TURN\nSHARED_TURN\n",
    });
  });

  it("bounds source bytes and context ranges without claiming missing lines", async () => {
    expect(await captureTurnDiff(update("x".repeat((1 << 20) + 1)), scope)).toEqual({ status: "unavailable", reason: "size_limit" });
    expect(await captureTurnDiff(update("binary\0text"), scope)).toEqual({ status: "unavailable", reason: "binary" });
    const snapshot = await captured(source({ path: "file.txt", kind: { type: "add" }, diff: "x".repeat((64 << 10) + 1) }));
    expect(readTurnDiffContext(snapshot, { snapshotId: snapshot.snapshotId, side: "new", startLine: 1, endLine: 1 }))
      .toEqual({ status: "unavailable", reason: "range_limit" });
    expect(readTurnDiffContext(snapshot, { snapshotId: snapshot.snapshotId, side: "new", startLine: 1, endLine: 201 }))
      .toEqual({ status: "unavailable", reason: "range_limit" });
  });
});
