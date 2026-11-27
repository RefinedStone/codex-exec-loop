import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterAll, describe, expect, it } from "vitest";

import { discardCommands, editorUrl, executeGitFilePlan, NO_HELPER_PROGRAMS, parseNumstat, parsePorcelainStatus, planGitFileAction, readGitFileDiff, readGitStatus, resolveGitFile, resolveInside, runGitFileAction } from "../src/main/gitStatus";
import { isConflictStatus, stageState } from "../src/shared/gitChange";
import { parseGitDiff } from "../src/renderer/conversation/diff";

describe("git status parsing", () => {
  it("reads the branch, renames, untracked folders and plain changes", () => {
    const output = ["## main...origin/main [ahead 1]", " M src/a.ts", "R  new name.ts", "old name.ts", "?? build/", "?? 한글.txt", ""].join("\0");
    const { branch, files } = parsePorcelainStatus(output);
    expect(branch).toBe("main...origin/main [ahead 1]");
    expect(files.map((file) => [file.status, file.path, file.originalPath, file.untracked, file.directory])).toEqual([
      [" M", "src/a.ts", null, false, false],
      ["R ", "new name.ts", "old name.ts", false, false],
      ["??", "build/", null, true, true],
      ["??", "한글.txt", null, true, false],
    ]);
  });

  it("reads numstat counts, binary files and renames", () => {
    const output = ["3\t1\tsrc/a.ts", "-\t-\timage.png", "2\t0\t", "old.ts", "new.ts", ""].join("\0");
    const counts = parseNumstat(output);
    expect(counts.get("src/a.ts")).toEqual({ added: 3, removed: 1 });
    expect(counts.get("image.png")).toEqual({ added: null, removed: null });
    expect(counts.get("new.ts")).toEqual({ added: 2, removed: 0 });
  });

  it("only accepts relative paths inside the folder", () => {
    const root = join(tmpdir(), "akra-root");
    expect(resolveInside(root, "src/a.ts")).toBe(join(root, "src", "a.ts"));
    expect(resolveInside(root, "..notes")).toBe(join(root, "..notes"));
    expect(resolveInside(root, "src/..notes")).toBe(join(root, "src", "..notes"));
    for (const bad of ["../outside.txt", "src/../../outside.txt", join(root, "a.ts"), "", ".", 42]) {
      expect(() => resolveInside(root, bad)).toThrow();
    }
  });
});

describe("git file actions", () => {
  const roots: string[] = [];
  afterAll(() => roots.forEach((root) => rmSync(root, { recursive: true, force: true, maxRetries: 5 })));
  /** 커밋 하나가 있는 새 저장소이다. `files` 는 첫 커밋의 파일이다. */
  const repository = (files: Record<string, string> = { "a.txt": "a\n", "b.txt": "b\n" }) => {
    const root = mkdtempSync(join(tmpdir(), "akra-git-actions-"));
    roots.push(root);
    const git = (...args: string[]) =>
      execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", ...args], { cwd: root, encoding: "utf8" });
    const write = (path: string, text: string) => writeFileSync(join(root, path), text);
    const text = (path: string) => readFileSync(join(root, path), "utf8").replace(/\r/g, "");
    const statuses = async () => {
      const status = await readGitStatus(root);
      return Object.fromEntries(status.ok ? status.files.map((file) => [file.path, file.status]) : []);
    };
    git("init", "-q", "-b", "main");
    for (const [path, content] of Object.entries(files)) write(path, content);
    if (Object.keys(files).length > 0) {
      git("add", ".");
      git("commit", "-q", "-m", "init");
    }
    return { root, git, write, text, statuses };
  };

  it("keeps parser paths stable when Git uses mnemonic prefixes", async () => {
    const { root, git, write } = repository({ "a.txt": "base\n" });
    git("config", "diff.mnemonicPrefix", "true");
    write("a.txt", "staged\n");
    git("add", "a.txt");
    write("a.txt", "working\n");
    for (const source of [undefined, "head_worktree", "head_index", "index_worktree"] as const) {
      const diff = await readGitFileDiff(root, "a.txt", false, source);
      if (diff.kind !== "gitDiff") throw new Error("expected text patch");
      const file = parseGitDiff(diff.text).find((entry) => entry.path === "a.txt" || entry.movePath === "a.txt");
      expect(file?.lines.length).toBeGreaterThan(0);
      expect(diff.text).toContain("diff --git a/a.txt b/a.txt");
    }
  });

  it("overrides disabled and custom Git prefixes for file comparisons", async () => {
    const { root, git, write } = repository({ "a.txt": "base\n" });
    git("config", "diff.noprefix", "true");
    git("config", "diff.srcPrefix", "old/");
    git("config", "diff.dstPrefix", "new/");
    write("a.txt", "working\n");
    const diff = await readGitFileDiff(root, "a.txt", false, "head_worktree");
    if (diff.kind !== "gitDiff") throw new Error("expected text patch");
    expect(parseGitDiff(diff.text)[0]?.path).toBe("a.txt");
    expect(diff.text).toContain("diff --git a/a.txt b/a.txt");
  });

  it("distinguishes staged, unstaged and combined diff snapshots", async () => {
    const { root, git, write } = repository({ "a.txt": "base\n" });
    write("a.txt", "stage\n");
    git("add", "a.txt");
    write("a.txt", "work\n");
    const staged = await readGitFileDiff(root, "a.txt", false, "head_index");
    const unstaged = await readGitFileDiff(root, "a.txt", false, "index_worktree");
    const combined = await readGitFileDiff(root, "a.txt", false, "head_worktree");
    expect(staged).toMatchObject({ kind: "gitDiff", text: expect.stringContaining("+stage"), snapshot: { source: "head_index", contentStatus: "patch_only" } });
    expect(unstaged).toMatchObject({ kind: "gitDiff", text: expect.stringContaining("-stage"), snapshot: { source: "index_worktree" } });
    expect(combined).toMatchObject({ kind: "gitDiff", text: expect.stringContaining("+work"), snapshot: { source: "head_worktree" } });
    expect(new Set([staged.snapshot?.snapshotId, unstaged.snapshot?.snapshotId, combined.snapshot?.snapshotId]).size).toBe(3);
    write("a.txt", "changed again\n");
    expect((await readGitFileDiff(root, "a.txt", false, "index_worktree")).snapshot?.snapshotId).not.toBe(unstaged.snapshot?.snapshotId);
    await expect(readGitFileDiff(root, "a.txt", false, "untracked")).rejects.toThrow();
    await expect(readGitFileDiff(root, "a.txt", false, "invalid")).rejects.toThrow();
  });

  it("preserves unstaged rename coordinates and the complete rename patch", async () => {
    const { root, git, statuses } = repository({ "old.txt": "unchanged\n" });
    git("mv", "old.txt", "new.txt");
    git("reset", "-q");
    git("add", "-N", "new.txt");
    expect(await statuses()).toMatchObject({ "new.txt": " R" });

    const diff = await readGitFileDiff(root, "new.txt", false, "index_worktree");
    expect(diff).toMatchObject({
      kind: "gitDiff",
      text: expect.stringContaining("rename from old.txt\nrename to new.txt"),
      snapshot: { source: "index_worktree", originalPath: "old.txt", path: "new.txt" },
    });
    if (diff.kind !== "gitDiff") throw new Error("expected rename patch");
    expect(diff.text).not.toContain("new file mode");
  });

  it("does not describe a staged rename as a worktree rename", async () => {
    const { root, git, write } = repository({ "old.txt": "unchanged\n" });
    git("mv", "old.txt", "new.txt");
    write("new.txt", "edited after staging\n");
    expect((await readGitFileDiff(root, "new.txt", false, "head_index")).snapshot?.originalPath).toBe("old.txt");
    const diff = await readGitFileDiff(root, "new.txt", false, "index_worktree");
    expect(diff).toMatchObject({
      kind: "gitDiff",
      text: expect.stringContaining("+edited after staging"),
      snapshot: { originalPath: null, path: "new.txt" },
    });
  });

  it("marks tracked binary comparisons across all three sources", async () => {
    const { root, git, write } = repository({ "blob.bin": "base\0binary\n" });
    write("blob.bin", "stage\0binary\n");
    git("add", "blob.bin");
    write("blob.bin", "work\0binary\n");
    for (const source of ["head_worktree", "head_index", "index_worktree"] as const) {
      expect(await readGitFileDiff(root, "blob.bin", false, source)).toMatchObject({
        kind: "gitDiff",
        text: expect.stringMatching(/^Binary files .* differ$/m),
        snapshot: { source, contentStatus: "binary" },
      });
    }
  });

  it("does not mislabel a first-commit staged diff as a combined comparison", async () => {
    const { root, git, write } = repository({});
    write("a.txt", "initial\n"); git("add", "a.txt");
    const staged = await readGitFileDiff(root, "a.txt", false, "head_index");
    expect(staged.snapshot).toMatchObject({ source: "head_index", headOid: null });
    await expect(readGitFileDiff(root, "a.txt", false, "head_worktree")).rejects.toThrow("첫 커밋");
  });

  it("picks the discard commands from the status", () => {
    expect(discardCommands("??", "new", null)).toEqual([["clean", "-f", "--", "new"]]);
    expect(discardCommands("A ", "added", null)).toEqual([["rm", "-f", "-q", "--", "added"]]);
    expect(discardCommands("R ", "new", "old").map((command) => command[0])).toEqual(["rm", "restore"]);
    expect(discardCommands(" M", "changed", null)).toEqual([["restore", "--staged", "--worktree", "--source=HEAD", "--", "changed"]]);
  });

  it("reads the two status letters", () => {
    expect(stageState({ status: "M ", untracked: false })).toEqual({ staged: true, unstaged: false });
    expect(stageState({ status: "MM", untracked: false })).toEqual({ staged: true, unstaged: true });
    expect(stageState({ status: "??", untracked: true })).toEqual({ staged: false, unstaged: true });
    expect(["UU", "AA", "DD", "AU", "DU"].map(isConflictStatus)).toEqual([true, true, true, true, true]);
    expect([" M", "A ", "??", "R "].map(isConflictStatus)).toEqual([false, false, false, false]);
  });

  it("builds the editor address without encoding the drive letter", () => {
    expect(editorUrl("C:\\work\\my app\\한글.ts")).toBe("vscode://file/C:/work/my%20app/%ED%95%9C%EA%B8%80.ts");
    expect(editorUrl("/home/me/a#b.ts")).toBe("vscode://file//home/me/a%23b.ts");
  });

  it("stages, unstages and discards one file and leaves the other files alone", async () => {
    const { root, write, text, statuses } = repository();
    write("a.txt", "A\n");
    write("b.txt", "B\n");
    write("new file.txt", "new\n");

    expect(await runGitFileAction(root, "stage", "a.txt")).toEqual({ ok: true });
    expect(await statuses()).toMatchObject({ "a.txt": "M ", "b.txt": " M" });
    expect(await runGitFileAction(root, "unstage", "a.txt")).toEqual({ ok: true });
    expect(await statuses()).toMatchObject({ "a.txt": " M" });

    expect(await runGitFileAction(root, "discard", "a.txt")).toEqual({ ok: true });
    expect(await runGitFileAction(root, "discard", "new file.txt")).toEqual({ ok: true });
    expect(await statuses()).toEqual({ "b.txt": " M" });
    expect(existsSync(join(root, "new file.txt"))).toBe(false);
    expect([text("a.txt"), text("b.txt")]).toEqual(["a\n", "B\n"]);
  });

  it("discards a staged new file and a staged rename", async () => {
    const { root, git, write, statuses } = repository();
    write("added.txt", "x\n");
    git("add", "added.txt");
    git("mv", "a.txt", "renamed.txt");
    expect(await statuses()).toMatchObject({ "renamed.txt": "R " });

    expect(await runGitFileAction(root, "discard", "added.txt")).toEqual({ ok: true });
    expect(await runGitFileAction(root, "discard", "renamed.txt")).toEqual({ ok: true });
    expect(await statuses()).toEqual({});
    expect([existsSync(join(root, "a.txt")), existsSync(join(root, "renamed.txt"))]).toEqual([true, false]);
  });

  it("discards a detected copy without changing its existing source", async () => {
    const content = Array.from({ length: 20 }, (_, index) => `original line ${index}\n`).join("");
    const { root, git, write, text, statuses } = repository({ "source.txt": content });
    git("config", "status.renames", "copies");
    write("copy.txt", content);
    write("source.txt", content + "staged source edit\n");
    git("add", "source.txt", "copy.txt");
    expect(await statuses()).toMatchObject({ "copy.txt": "C ", "source.txt": "M " });
    const sourceIndexBefore = git("ls-files", "--stage", "--", "source.txt");
    const planned = await planGitFileAction(root, "discard", "copy.txt");
    expect(planned.ok).toBe(true);
    if (!planned.ok) return;

    expect(await executeGitFilePlan(planned.plan)).toEqual({ ok: true });
    expect(existsSync(join(root, "copy.txt"))).toBe(false);
    expect(text("source.txt")).toBe(content + "staged source edit\n");
    expect(git("ls-files", "--stage", "--", "source.txt")).toBe(sourceIndexBefore);
    expect(await statuses()).toEqual({ "source.txt": "M " });
  });

  it("reads the state from git, not from the caller", async () => {
    const { root, write, text, statuses } = repository();
    // 바뀌지 않은 파일과 목록에 없는 경로는 거절한다. 이전에는 호출한 쪽이 보낸 상태로 명령을 골랐다.
    expect((await runGitFileAction(root, "discard", "a.txt")).ok).toBe(false);
    expect((await runGitFileAction(root, "stage", "missing.txt")).ok).toBe(false);
    write("a.txt", "A\n");
    expect((await runGitFileAction(root, "unstage", "a.txt")).ok).toBe(false);
    expect(text("a.txt")).toBe("A\n");
    expect(await statuses()).toEqual({ "a.txt": " M" });
  });

  it("does not overwrite a new file at the old path of a rename", async () => {
    const { root, git, write, text } = repository();
    git("mv", "a.txt", "a2.txt");
    write("a.txt", "new work\n");
    const result = await runGitFileAction(root, "discard", "a2.txt");
    expect(result.ok).toBe(false);
    expect([text("a.txt"), existsSync(join(root, "a2.txt"))]).toEqual(["new work\n", true]);
  });

  it("refuses a stale confirmed rename discard when its old path is newly created", async () => {
    const { root, git, write, text, statuses } = repository();
    git("mv", "a.txt", "renamed.txt");
    const planned = await planGitFileAction(root, "discard", "renamed.txt");
    expect(planned.ok).toBe(true);
    if (!planned.ok) return;

    // The confirmation dialog remains open while the operator creates a new old-path file.
    write("a.txt", "new content during confirmation\\n".replace("\\n", "\n"));
    const result = await executeGitFilePlan(planned.plan);
    expect(result.ok).toBe(false);
    expect(text("a.txt")).toBe("new content during confirmation\n");
    expect(text("renamed.txt")).toBe("a\n");
    expect(await statuses()).toMatchObject({ "renamed.txt": "R ", "a.txt": "??" });
  });

  it("rejects a discard plan if the renamed file is staged differently after confirmation", async () => {
    const { root, git, write, text } = repository();
    git("mv", "a.txt", "renamed.txt");
    const planned = await planGitFileAction(root, "discard", "renamed.txt");
    expect(planned.ok).toBe(true);
    if (!planned.ok) return;

    // The staged blob for the selected path is no longer the one the user confirmed.
    write("renamed.txt", "different staged content\n");
    git("add", "renamed.txt");
    expect((await executeGitFilePlan(planned.plan)).ok).toBe(false);
    expect(text("renamed.txt")).toBe("different staged content\n");
    expect(existsSync(join(root, "a.txt"))).toBe(false);
  });

  it("rejects a confirmed discard if HEAD advances or the destination is edited", async () => {
    const { root, git, write, text } = repository();
    git("mv", "a.txt", "renamed.txt");
    const planned = await planGitFileAction(root, "discard", "renamed.txt");
    expect(planned.ok).toBe(true);
    if (!planned.ok) return;
    write("renamed.txt", "new work while confirming\n");
    expect((await executeGitFilePlan(planned.plan)).ok).toBe(false);
    expect(text("renamed.txt")).toBe("new work while confirming\n");

    git("add", "renamed.txt");
    git("commit", "-q", "-m", "new operator commit");
    expect((await executeGitFilePlan(planned.plan)).ok).toBe(false);
    expect(text("renamed.txt")).toBe("new work while confirming\n");
  });

  it("refuses every action on a file with a merge conflict", async () => {
    const { root, git, write, text, statuses } = repository();
    git("checkout", "-q", "-b", "other");
    write("a.txt", "theirs\n");
    git("commit", "-q", "-am", "theirs");
    git("checkout", "-q", "main");
    write("a.txt", "ours\n");
    git("commit", "-q", "-am", "ours");
    try {
      git("merge", "-q", "other");
    } catch {
      // 충돌로 끝난다.
    }
    expect(await statuses()).toEqual({ "a.txt": "UU" });
    const before = text("a.txt");
    for (const action of ["stage", "unstage", "discard"]) {
      expect((await runGitFileAction(root, action, "a.txt")).ok).toBe(false);
    }
    expect(await statuses()).toEqual({ "a.txt": "UU" });
    expect(text("a.txt")).toBe(before);
  });

  it("unstages before the first commit", async () => {
    const { root, git, write, statuses } = repository({});
    write("first.txt", "1\n");
    git("add", "first.txt");
    expect(await runGitFileAction(root, "unstage", "first.txt")).toEqual({ ok: true });
    expect(await statuses()).toEqual({ "first.txt": "??" });
    expect(existsSync(join(root, "first.txt"))).toBe(true);
  });

  it("reads a staged file diff before the first commit", async () => {
    const { root, git, write } = repository({});
    write("first.txt", "staged content\n");
    git("add", "first.txt");
    // The diff should show staged content, not a later unstaged edit.
    write("first.txt", "unstaged edit\n");
    const diff = await readGitFileDiff(root, "first.txt", false);
    expect(diff.kind).toBe("gitDiff");
    if (diff.kind === "gitDiff") {
      expect(diff.text).toContain("+staged content");
      expect(diff.text).not.toContain("+unstaged edit");
    }
  });

  it("stages a new folder, but not one with a filtered file, and never discards a folder", async () => {
    const { root, write, statuses } = repository();
    mkdirSync(join(root, "docs"));
    write("docs/note.txt", "n\n");
    expect((await runGitFileAction(root, "discard", "docs/")).ok).toBe(false);
    expect(existsSync(join(root, "docs", "note.txt"))).toBe(true);
    expect(await runGitFileAction(root, "stage", "docs/")).toEqual({ ok: true });
    expect(await statuses()).toEqual({ "docs/note.txt": "A " });

    write(".gitattributes", "*.bin filter=lfs\n");
    mkdirSync(join(root, "assets"));
    write("assets/big.bin", "data\n");
    write("top.bin", "data\n");
    expect((await runGitFileAction(root, "stage", "assets/")).ok).toBe(false);
    expect((await runGitFileAction(root, "stage", "top.bin")).ok).toBe(false);
    expect((await runGitFileAction(root, "discard", "top.bin")).ok).toBe(false);
    expect(await statuses()).toMatchObject({ "assets/": "??", "top.bin": "??" });
  });

  it("never reads content behind an untracked symlink outside the repository", async () => {
    const { root, write } = repository();
    const outside = mkdtempSync(join(tmpdir(), "akra-untracked-secret-"));
    try {
      writeFileSync(join(outside, "secret.txt"), "PRIVATE_EXTERNAL_CONTENT");
      try {
        symlinkSync(join(outside, "secret.txt"), join(root, "linked.txt"), "file");
      } catch (error) {
        if (process.platform === "win32" && ["EPERM", "ENOTSUP", "EACCES"].includes((error as NodeJS.ErrnoException).code ?? "")) return;
        throw error;
      }
      await expect(readGitFileDiff(root, "linked.txt", true)).rejects.toThrow(/심볼릭 링크|저장소 외부/);
      write("ordinary.txt", "visible inside repository\n");
      expect(await readGitFileDiff(root, "ordinary.txt", true)).toEqual({
        kind: "content",
        text: "visible inside repository\n",
        truncated: false,
      });
    } finally {
      rmSync(outside, { recursive: true, force: true, maxRetries: 5 });
    }
  });

  it("rejects untracked file reads through symlinked parent directories", async () => {
    const { root } = repository();
    const outside = mkdtempSync(join(tmpdir(), "akra-untracked-parent-"));
    try {
      writeFileSync(join(outside, "secret.txt"), "DO_NOT_DISCLOSE");
      try {
        symlinkSync(outside, join(root, "linked-parent"), process.platform === "win32" ? "junction" : "dir");
      } catch (error) {
        if (process.platform === "win32" && ["EPERM", "ENOTSUP", "EACCES"].includes((error as NodeJS.ErrnoException).code ?? "")) return;
        throw error;
      }
      await expect(readGitFileDiff(root, "linked-parent/secret.txt", true)).rejects.toThrow(/저장소 외부/);
    } finally {
      rmSync(outside, { recursive: true, force: true, maxRetries: 5 });
    }
  });

  it("refuses a path outside the repository and an unknown action", async () => {
    const { root, write } = repository();
    write("b.txt", "B\n");
    await expect(runGitFileAction(root, "discard", "../escape.txt")).rejects.toThrow();
    await expect(runGitFileAction(root, "push", "b.txt")).rejects.toThrow();
    await expect(resolveGitFile(root, "../escape.txt")).rejects.toThrow();
    expect(await resolveGitFile(root, "b.txt")).toBe(join(root, "b.txt"));
  });
});

describe("git status in a real repository", () => {
  const root = mkdtempSync(join(tmpdir(), "akra-git-"));
  afterAll(() => rmSync(root, { recursive: true, force: true, maxRetries: 5 }));
  const git = (...args: string[]) =>
    execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", ...args], {
      cwd: root,
    });

  it("lists changes with counts and reads each file's diff", async () => {
    git("init", "-q", "-b", "main");
    writeFileSync(join(root, "tracked.txt"), "one\ntwo\n");
    git("add", ".");
    git("commit", "-q", "-m", "init");
    writeFileSync(join(root, "tracked.txt"), "one\nTWO\nthree\n");
    writeFileSync(join(root, "new.txt"), "hello\n");
    writeFileSync(join(root, "blob.bin"), Buffer.from([1, 0, 2]));
    mkdirSync(join(root, "folder"));
    writeFileSync(join(root, "folder", "inside.txt"), "x\n");

    const status = await readGitStatus(root);
    expect(status.ok).toBe(true);
    if (!status.ok) return;
    expect(status.branch).toBe("main");
    const byPath = new Map(status.files.map((file) => [file.path, file]));
    expect(byPath.get("tracked.txt")).toMatchObject({ status: " M", added: 2, removed: 1, untracked: false });
    expect(byPath.get("new.txt")).toMatchObject({ status: "??", untracked: true, added: null });
    expect(byPath.get("folder/")).toMatchObject({ directory: true });

    const tracked = await readGitFileDiff(root, "tracked.txt", false);
    expect(tracked.kind === "gitDiff" && tracked.text).toContain("+TWO");
    expect(await readGitFileDiff(root, "new.txt", true)).toEqual({ kind: "content", text: "hello\n", truncated: false });
    expect(await readGitFileDiff(root, "blob.bin", true)).toEqual({ kind: "binary" });
    await expect(readGitFileDiff(root, "../escape.txt", true)).rejects.toThrow();
  });

  it("reads root-relative status paths when the thread cwd is a subfolder", async () => {
    mkdirSync(join(root, "sub"), { recursive: true });
    writeFileSync(join(root, "sub", "inner.txt"), "a\n");
    git("add", ".");
    git("commit", "-q", "-m", "sub");
    writeFileSync(join(root, "sub", "inner.txt"), "b\n");
    writeFileSync(join(root, "sub", "fresh[1].txt"), "new\n");
    const subfolder = join(root, "sub");

    const status = await readGitStatus(subfolder);
    expect(status.ok && status.files.map((file) => file.path)).toEqual(expect.arrayContaining(["sub/inner.txt", "sub/fresh[1].txt"]));
    const inner = await readGitFileDiff(subfolder, "sub/inner.txt", false);
    expect(inner.kind === "gitDiff" && inner.text).toContain("+b");
    expect(await readGitFileDiff(subfolder, "sub/fresh[1].txt", true)).toEqual({ kind: "content", text: "new\n", truncated: false });
  });

  // Windows 에서는 필터가 실제로 돌면 콘솔 창이 뜬다. 그래서 이 확인은 CI(Linux) 에서만 한다.
  it.skipIf(process.platform === "win32")("does not run LFS filter programs while reading", async () => {
    const marker = join(root, "..", `${root.split(/[\\/]/).pop()}-filter-ran`);
    git("config", "filter.lfs.process", `sh -c 'touch "${marker}"; cat'`);
    git("config", "filter.lfs.clean", `sh -c 'touch "${marker}"; cat'`);
    writeFileSync(join(root, ".gitattributes"), "*.lfs filter=lfs\n");
    writeFileSync(join(root, "model.lfs"), "v1\n");
    // 준비 단계도 같은 설정으로 필터를 끈다. CI 러너의 전역 `filter.lfs.required=true` 도 함께 끈다.
    git(...NO_HELPER_PROGRAMS, "add", ".");
    git(...NO_HELPER_PROGRAMS, "commit", "-q", "-m", "lfs");
    writeFileSync(join(root, "model.lfs"), "v2\n");

    expect((await readGitStatus(root)).ok).toBe(true);
    await readGitFileDiff(root, "model.lfs", false);
    expect(existsSync(marker)).toBe(false);
  });

  it("reports a folder that is not a git repository", async () => {
    const plain = mkdtempSync(join(tmpdir(), "akra-plain-"));
    try {
      const status = await readGitStatus(plain);
      expect(status.ok).toBe(false);
    } finally {
      rmSync(plain, { recursive: true, force: true, maxRetries: 5 });
    }
  });
});
