import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterAll, describe, expect, it } from "vitest";

import { discardCommands, editorUrl, NO_HELPER_PROGRAMS, parseNumstat, parsePorcelainStatus, readGitFileDiff, readGitStatus, resolveGitFile, resolveInside, runGitFileAction } from "../src/main/gitStatus";
import { isConflictStatus, stageState } from "../src/shared/gitChange";

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
    for (const bad of ["../outside.txt", "src/../../outside.txt", join(root, "a.ts"), "", ".", 42]) {
      expect(() => resolveInside(root, bad)).toThrow();
    }
  });
});

describe("git file actions", () => {
  const roots: string[] = [];
  afterAll(() => roots.forEach((root) => rmSync(root, { recursive: true, force: true })));
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
  afterAll(() => rmSync(root, { recursive: true, force: true }));
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
      rmSync(plain, { recursive: true, force: true });
    }
  });
});
