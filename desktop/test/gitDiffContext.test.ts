import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { readGitFileDiff } from "../src/main/gitFileDiff";
import { readGitDiffContext } from "../src/main/gitDiffContext";
import * as contentReader from "../src/main/gitSnapshotContent";
import type { GitDiffSnapshot, GitDiffSource } from "../src/shared/bridge";
import type { GitDiffSide } from "../src/shared/gitDiffContext";

const roots: string[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true, maxRetries: 5 });
});

function repository(initial: string | null = "base\nunchanged\nlast\n") {
  const root = mkdtempSync(join(tmpdir(), "akra-snapshot-"));
  roots.push(root);
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GIT_")));
  const git = (...args: string[]) => execFileSync("git", ["-c", "user.name=Tester", "-c",
    "user.email=fixture@localhost.invalid", "-c", "commit.gpgsign=false", ...args],
  { cwd: root, encoding: "utf8", env: { ...env, GIT_MASTER: "1" } });
  const write = (text: string, path = "한글 file.txt") => writeFileSync(join(root, path), text);
  git("init", "-q", "-b", "main");
  git("config", "core.autocrlf", "false");
  if (initial !== null) {
    write(initial);
    git("add", ".");
    git("commit", "-qm", "fixture");
  }
  return { root, git, write };
}

async function snapshot(root: string, source: GitDiffSource, path = "한글 file.txt") {
  const result = await readGitFileDiff(root, path, source === "untracked", source);
  if (!result.snapshot) throw new Error("Missing snapshot.");
  return result.snapshot;
}

const range = (view: GitDiffSnapshot, side: GitDiffSide, startLine = 1, endLine = 3) =>
  ({ snapshotId: view.snapshotId, source: view.source, side, startLine, endLine });

describe("immutable diff context", () => {
  it("preserves worktree file reads and the repository/worktree identities", async () => {
    const { root, git } = repository();
    const view = await snapshot(root, "worktree_file");
    expect(view).toMatchObject({ worktreeId: root, repositoryId: git("rev-parse", "--path-format=absolute", "--git-common-dir").trim() });
    expect(await readGitDiffContext(root, range(view, "new"))).toMatchObject({ text: "base\nunchanged\nlast\n", source: "worktree_file" });
  });

  it.each([
    ["head_index", "base", "staged"],
    ["index_worktree", "INDEX_ONLY", "WORKTREE_ONLY"],
    ["head_worktree", "base", "working"],
  ] as const)("reads exact old/new lines for %s", async (source, old, next) => {
    const { root, git, write } = repository();
    write(`${source === "index_worktree" ? "INDEX_ONLY" : "staged"}\nunchanged\nlast\n`); git("add", ".");
    write(`${source === "index_worktree" ? "WORKTREE_ONLY" : "working"}\nunchanged\nlast\n`);
    const view = await snapshot(root, source);
    expect(await readGitDiffContext(root, range(view, "old"))).toMatchObject({ status: "available", text: `${old}\nunchanged\nlast\n`, current: true });
    expect(await readGitDiffContext(root, range(view, "new"))).toMatchObject({ status: "available", text: `${next}\nunchanged\nlast\n`, current: true });
  });

  it("retains the captured bytes and reports stale after file, index and HEAD edits", async () => {
    const { root, git, write } = repository();
    write("selected\nunchanged\nlast\n");
    const view = await snapshot(root, "head_worktree");
    write("later\nunchanged\nlast\n"); git("add", "."); git("commit", "-qm", "later");
    expect(await readGitDiffContext(root, range(view, "new"))).toMatchObject({ text: "selected\nunchanged\nlast\n", current: false });
    expect(await readGitDiffContext(root, range(view, "old"))).toMatchObject({ text: "base\nunchanged\nlast\n", current: false });
  });

  it("keeps rename coordinates and CRLF bytes on both sides", async () => {
    const { root, git, write } = repository("base\r\nunchanged\r\nlast");
    git("mv", "한글 file.txt", "renamed file.txt");
    write("changed\r\nunchanged\r\nlast", "renamed file.txt"); git("add", ".");
    const view = await snapshot(root, "head_index", "renamed file.txt");
    expect(view).toMatchObject({ originalPath: "한글 file.txt", contents: { old: { lineEnding: "crlf" }, new: { lineEnding: "crlf" } } });
    expect(await readGitDiffContext(root, range(view, "old", 2, 3))).toMatchObject({ text: "unchanged\r\nlast" });
    expect(await readGitDiffContext(root, range(view, "new", 1, 1))).toMatchObject({ text: "changed\r\n" });
  });

  it("keeps removed lines on the old side and exposes an empty new side", async () => {
    const { root, git } = repository();
    git("rm", "-q", "한글 file.txt");
    const view = await snapshot(root, "head_index");
    expect(view.contents?.new).toMatchObject({ status: "complete", lineCount: 0 });
    expect(await readGitDiffContext(root, range(view, "old"))).toMatchObject({ text: "base\nunchanged\nlast\n" });
    expect(await readGitDiffContext(root, range(view, "new", 1, 1))).toEqual({ status: "unavailable", reason: "range_limit" });
  });

  it("captures unborn staged and untracked files without inventing an old side", async () => {
    const { root, git, write } = repository(null);
    write("new\n");
    const untracked = await snapshot(root, "untracked");
    git("add", ".");
    const staged = await snapshot(root, "head_index");
    const combined = await snapshot(root, "head_worktree");
    for (const view of [untracked, staged, combined]) {
      expect(view.contents?.old).toMatchObject({ status: "complete", lineCount: 0 });
      expect(await readGitDiffContext(root, range(view, "new", 1, 1))).toMatchObject({ text: "new\n" });
    }
  }, 20_000);

  it("reports gitlinks as unsupported content instead of text lines", async () => {
    const { root, git } = repository();
    const head = git("rev-parse", "HEAD").trim();
    git("update-index", "--add", "--cacheinfo", `160000,${head},module`);
    const view = await snapshot(root, "head_index", "module");
    expect(view.contents?.new).toEqual({ status: "unavailable", reason: "unsupported_entry" });
  });

  it.each([
    ["x".repeat((1 << 20) + 1), "size_limit"],
    ["binary\0bytes", "binary"],
  ] as const)("reports unavailable content for bounded or binary files", async (text, reason) => {
    const { root, write } = repository();
    write(text);
    const view = await snapshot(root, "head_worktree");
    expect(view.contents?.new).toEqual({ status: "unavailable", reason });
    expect(await readGitDiffContext(root, range(view, "new", 1, 1))).toEqual({ status: "unavailable", reason });
  });

  it("rejects cross-repository/source ranges and expires old cache entries", async () => {
    const { root, write } = repository();
    const other = repository();
    write("selected\n");
    const view = await snapshot(root, "head_worktree");
    expect(await readGitDiffContext(other.root, range(view, "new", 1, 1))).toEqual({ status: "unavailable", reason: "scope_mismatch" });
    expect(await readGitDiffContext(root, { ...range(view, "new"), source: "head_index" })).toEqual({ status: "unavailable", reason: "scope_mismatch" });
    expect(await readGitDiffContext(root, range(view, "new", 1, 201))).toEqual({ status: "unavailable", reason: "range_limit" });
    for (let index = 0; index < 17; index += 1) {
      write(`revision ${index}\n`);
      await snapshot(root, "head_worktree");
    }
    expect(await readGitDiffContext(root, range(view, "new", 1, 1))).toEqual({ status: "unavailable", reason: "snapshot_expired" });
  }, 60_000);

  it("rejects a working copy that changes between the two content reads", async () => {
    const { root, write } = repository();
    write("first read\n");
    const capture = contentReader.captureWorkingFile;
    vi.spyOn(contentReader, "captureWorkingFile").mockImplementationOnce(async (file) => {
      const result = await capture(file);
      write("second read\n");
      return result;
    });
    await expect(snapshot(root, "head_worktree")).rejects.toThrow("변경");
  });
});
