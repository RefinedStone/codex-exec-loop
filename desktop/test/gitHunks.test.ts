import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import { executeGitHunkPlan, planGitHunkAction, readGitHunks } from "../src/main/gitHunks";
import type { GitHunkAction } from "../src/shared/gitHunks";

describe("Git hunk actions", () => {
  const roots: string[] = [];
  afterEach(() => {
    for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
  });

  function repository(content = Array.from({ length: 40 }, (_, index) => `line ${index + 1}\n`).join("")) {
    const root = mkdtempSync(join(tmpdir(), "akra-git-hunks-"));
    roots.push(root);
    const git = (...args: string[]) => execFileSync("git", [
      "-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", ...args,
    ], { cwd: root, encoding: "utf8" });
    const write = (text: string) => writeFileSync(join(root, "a.txt"), text);
    const text = () => readFileSync(join(root, "a.txt"), "utf8");
    git("init", "-q", "-b", "main");
    git("config", "core.autocrlf", "false");
    write(content);
    git("add", ".");
    git("commit", "-q", "-m", "initial");
    return { root, git, write, text, content };
  }

  async function act(root: string, action: GitHunkAction, position = 0) {
    const diff = await readGitHunks(root, "a.txt");
    expect(diff.message).toBeNull();
    const hunk = diff.hunks[position];
    if (!hunk) throw new Error("Missing test hunk");
    const result = await planGitHunkAction(root, action, "a.txt", hunk.id);
    if (!result.ok) throw new Error(result.message);
    return executeGitHunkPlan(result.plan);
  }

  it.each(["stage", "discard"] as const)("preserves other hunks and pre-staged changes when %s runs", async (action) => {
    // Given a staged first-line change and two distant unstaged replacements.
    const repo = repository();
    const staged = repo.content.replace("line 1\n", "staged\n");
    repo.write(staged);
    repo.git("add", "a.txt");
    const first = staged.replace("line 12\n", "first\n");
    const both = first.replace("line 32\n", "second\n");
    repo.write(both);
    expect((await readGitHunks(repo.root, "a.txt")).hunks).toHaveLength(2);

    // When the second unstaged hunk is selected.
    const result = await act(repo.root, action, 1);

    // Then the index retains staged work and the other hunk stays unstaged.
    expect(result).toEqual({ ok: true });
    expect(repo.git("show", ":a.txt")).toBe(action === "stage" ? staged.replace("line 32\n", "second\n") : staged);
    expect(repo.text()).toBe(action === "stage" ? both : first);
  });

  it.each(["stage", "discard"] as const)("handles shifted line numbers when %s selects a later hunk", async (action) => {
    const repo = repository();
    const first = repo.content.replace("line 4\n", "extra\nline 4\n");
    const both = first.replace("line 32\n", "second\n");
    repo.write(both);

    expect(await act(repo.root, action, 1)).toEqual({ ok: true });

    expect(repo.git("show", ":a.txt")).toBe(action === "stage" ? repo.content.replace("line 32\n", "second\n") : repo.content);
    expect(repo.text()).toBe(action === "stage" ? both : first);
  });

  it.each(["lf", "crlf"] as const)("preserves %s working-tree endings when discard overrides autocrlf", async (eol) => {
    const repo = repository();
    repo.git("config", "core.autocrlf", "true");
    const first = repo.content.replace("line 12\n", "first\n");
    const both = first.replace("line 32\n", "second\n");
    const format = (text: string) => eol === "crlf" ? text.replaceAll("\n", "\r\n") : text;
    repo.write(format(both));

    expect(await act(repo.root, "discard", 1)).toEqual({ ok: true });

    expect(repo.text()).toBe(format(first));
    expect(repo.git("show", ":a.txt")).toBe(repo.content);
  });

  it("rejects discard when explicit eol attributes would change unrelated lines", async () => {
    const repo = repository();
    writeFileSync(join(repo.root, ".gitattributes"), "a.txt text eol=crlf\n");
    const content = repo.content.replace("line 12\n", "changed\n");
    repo.write(content);
    const diff = await readGitHunks(repo.root, "a.txt");

    const planned = await planGitHunkAction(repo.root, "discard", "a.txt", diff.hunks[0]?.id);

    expect(planned.ok).toBe(false);
    expect(repo.text()).toBe(content);
    expect(repo.git("show", ":a.txt")).toBe(repo.content);
  });

  it("rejects a plan after only working-tree line endings change", async () => {
    const repo = repository();
    repo.git("config", "core.autocrlf", "true");
    const content = repo.content.replace("line 12\n", "changed\n");
    repo.write(content);
    const diff = await readGitHunks(repo.root, "a.txt");
    const planned = await planGitHunkAction(repo.root, "discard", "a.txt", diff.hunks[0]?.id);
    if (!planned.ok) throw new Error(planned.message);
    const newer = content.replaceAll("\n", "\r\n");
    repo.write(newer);

    expect((await executeGitHunkPlan(planned.plan)).ok).toBe(false);

    expect(repo.text()).toBe(newer);
  });

  it.each(["stage", "discard"] as const)("rejects stale worktree plans without writes when %s runs", async (action) => {
    const repo = repository();
    repo.write("changed\n");
    const diff = await readGitHunks(repo.root, "a.txt");
    const planned = await planGitHunkAction(repo.root, action, "a.txt", diff.hunks[0]?.id);
    if (!planned.ok) throw new Error(planned.message);
    repo.write("newer work\n");

    expect((await executeGitHunkPlan(planned.plan)).ok).toBe(false);

    expect(repo.text()).toBe("newer work\n");
    expect(repo.git("show", ":a.txt")).toBe(repo.content);
  });

  it("rejects stale index plans without changing index or worktree", async () => {
    const repo = repository();
    repo.write("changed\n");
    const diff = await readGitHunks(repo.root, "a.txt");
    const planned = await planGitHunkAction(repo.root, "discard", "a.txt", diff.hunks[0]?.id);
    if (!planned.ok) throw new Error(planned.message);
    repo.git("add", "a.txt");

    expect((await executeGitHunkPlan(planned.plan)).ok).toBe(false);

    expect(repo.text()).toBe("changed\n");
    expect(repo.git("show", ":a.txt")).toBe("changed\n");
  });

  it("refuses malicious paths, unknown IDs, invalid actions, and forged plans", async () => {
    const repo = repository();
    repo.write("changed\n");
    const before = repo.git("diff");
    const diff = await readGitHunks(repo.root, "a.txt");

    for (const path of ["../a.txt", join(repo.root, "a.txt"), "a.txt\0", ":(glob)*", ".git/config"]) {
      expect((await planGitHunkAction(repo.root, "stage", path, diff.hunks[0]?.id)).ok).toBe(false);
    }
    for (const id of ["--- patch", "0".repeat(64), undefined, 42]) {
      expect((await planGitHunkAction(repo.root, "stage", "a.txt", id)).ok).toBe(false);
    }
    expect((await planGitHunkAction(repo.root, "unstage", "a.txt", diff.hunks[0]?.id)).ok).toBe(false);
    expect((await executeGitHunkPlan({ path: "a.txt", action: "stage" })).ok).toBe(false);

    expect(repo.git("diff")).toBe(before);
    expect(repo.git("show", ":a.txt")).toBe(repo.content);
  });

  it.each(["stage", "discard"] as const)("preserves a missing final newline when %s runs", async (action) => {
    const repo = repository("before");
    repo.write("after");

    expect(await act(repo.root, action)).toEqual({ ok: true });

    expect(repo.text()).toBe(action === "stage" ? "after" : "before");
    expect(repo.git("show", ":a.txt")).toBe(action === "stage" ? "after" : "before");
  });

  it.each(["stage", "discard"] as const)("handles a tracked deletion when %s runs", async (action) => {
    const repo = repository("before\n");
    rmSync(join(repo.root, "a.txt"));

    expect(await act(repo.root, action)).toEqual({ ok: true });

    expect(existsSync(join(repo.root, "a.txt"))).toBe(action === "discard");
    expect(repo.git("ls-files", "a.txt").trim()).toBe(action === "stage" ? "" : "a.txt");
    if (action === "discard") expect(repo.text()).toBe("before\n");
  });

  it("handles unstaged changes to a staged addition", async () => {
    const repo = repository();
    writeFileSync(join(repo.root, "new.txt"), "staged addition\n");
    repo.git("add", "new.txt");
    writeFileSync(join(repo.root, "new.txt"), "unstaged edit\n");
    const diff = await readGitHunks(repo.root, "new.txt");
    const plan = await planGitHunkAction(repo.root, "discard", "new.txt", diff.hunks[0]?.id);
    if (!plan.ok) throw new Error(plan.message);

    expect(await executeGitHunkPlan(plan.plan)).toEqual({ ok: true });

    expect(readFileSync(join(repo.root, "new.txt"), "utf8")).toBe("staged addition\n");
    expect(repo.git("show", ":new.txt")).toBe("staged addition\n");
  });

  it("refuses untracked, binary, filtered, renamed, and truncated files", async () => {
    const repo = repository();
    writeFileSync(join(repo.root, "new.txt"), "new\n");
    expect((await readGitHunks(repo.root, "new.txt")).message).not.toBeNull();
    repo.write("binary\0data");
    expect((await readGitHunks(repo.root, "a.txt")).hunks).toEqual([]);
    writeFileSync(join(repo.root, ".gitattributes"), "a.txt filter=lfs\n");
    expect((await readGitHunks(repo.root, "a.txt")).message).not.toBeNull();
    rmSync(join(repo.root, ".gitattributes"));
    repo.write(repo.content);
    repo.git("mv", "a.txt", "renamed.txt");
    expect((await readGitHunks(repo.root, "renamed.txt")).message).not.toBeNull();
    repo.git("mv", "renamed.txt", "a.txt");
    repo.write("x".repeat((1 << 20) + 100));
    expect((await readGitHunks(repo.root, "a.txt")).hunks).toEqual([]);
  });

  it("refuses merge conflicts without touching the conflict", async () => {
    const repo = repository("base\n");
    repo.git("checkout", "-q", "-b", "other");
    repo.write("other\n");
    repo.git("commit", "-qam", "other");
    repo.git("checkout", "-q", "main");
    repo.write("main\n");
    repo.git("commit", "-qam", "main");
    expect(() => repo.git("merge", "-q", "other")).toThrow();
    const before = repo.text();

    const result = await readGitHunks(repo.root, "a.txt");

    expect(result.hunks).toEqual([]);
    expect(result.message).not.toBeNull();
    expect(repo.text()).toBe(before);
  });

  it.each(["120000", "160000"])("refuses index mode %s without changing either file state", async (mode) => {
    const repo = repository("base\n");
    const object = repo.git("rev-parse", mode === "160000" ? "HEAD" : "HEAD:a.txt").trim();
    repo.git("update-index", "--cacheinfo", `${mode},${object},a.txt`);
    repo.write("changed\n");
    const index = repo.git("ls-files", "--stage");

    const result = await readGitHunks(repo.root, "a.txt");

    expect(result.hunks).toEqual([]);
    expect(result.message).not.toBeNull();
    expect(repo.git("ls-files", "--stage")).toBe(index);
    expect(repo.text()).toBe("changed\n");
  });

  it("refuses a file that is not UTF-8 instead of corrupting its bytes", async () => {
    // Given a tracked Latin-1 file ("caf\xe9") that is deleted, and the same file with an edit.
    const repo = repository();
    const latin1 = (text: string) => Buffer.from(text, "latin1");
    const path = join(repo.root, "a.txt");
    writeFileSync(path, latin1("caf\xe9\nsecond\n"));
    repo.git("add", "a.txt");
    repo.git("commit", "-q", "-m", "latin1");

    // When the file is deleted, restoring it through a hunk would re-encode the bytes.
    rmSync(path);
    const deleted = await readGitHunks(repo.root, "a.txt");
    expect(deleted).toEqual({ hunks: [], message: "UTF-8 이 아닌 내용이 있습니다. 파일 전체 동작을 사용하십시오." });
    expect((await planGitHunkAction(repo.root, "discard", "a.txt", "0".repeat(64))).ok).toBe(false);
    expect(existsSync(path)).toBe(false);

    // And when a non-UTF-8 line is only context of an edited hunk.
    writeFileSync(path, latin1("caf\xe9\nchanged\n"));
    expect((await readGitHunks(repo.root, "a.txt")).hunks).toEqual([]);
    expect(readFileSync(path).equals(latin1("caf\xe9\nchanged\n"))).toBe(true);
    expect(repo.git("status", "--porcelain").trim()).toBe("M a.txt");
  });

  it("rejects an ID belonging to another file", async () => {
    const repo = repository("base\n");
    writeFileSync(join(repo.root, "b.txt"), "base\n");
    repo.git("add", "b.txt");
    repo.git("commit", "-qm", "second file");
    repo.write("changed\n");
    writeFileSync(join(repo.root, "b.txt"), "changed\n");
    const diff = await readGitHunks(repo.root, "a.txt");

    const result = await planGitHunkAction(repo.root, "stage", "b.txt", diff.hunks[0]?.id);

    expect(result.ok).toBe(false);
    expect(repo.git("diff", "--cached")).toBe("");
  });
});
