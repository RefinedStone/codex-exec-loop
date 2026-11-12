import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterAll, describe, expect, it } from "vitest";

import { discardCommands, NO_HELPER_PROGRAMS, parseNumstat, parsePorcelainStatus, readGitFileDiff, readGitStatus, resolveGitFile, resolveInside, runGitFileAction } from "../src/main/gitStatus";

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
  const root = mkdtempSync(join(tmpdir(), "akra-git-actions-"));
  afterAll(() => rmSync(root, { recursive: true, force: true }));
  const git = (...args: string[]) =>
    execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", ...args], { cwd: root, encoding: "utf8" });
  const statusOf = async () => {
    const status = await readGitStatus(root);
    return new Map(status.ok ? status.files.map((file) => [file.path, file]) : []);
  };
  const fileOf = async (path: string) => {
    const file = (await statusOf()).get(path);
    if (!file) throw new Error(`${path} is not in git status`);
    return file;
  };

  it("picks the discard commands from the status", () => {
    expect(discardCommands("??", "new", null)).toEqual([["clean", "-fd", "--", "new"]]);
    expect(discardCommands("A ", "added", null)).toEqual([["rm", "-f", "-q", "--", "added"]]);
    expect(discardCommands("R ", "new", "old").map((command) => command[0])).toEqual(["rm", "restore"]);
    expect(discardCommands(" M", "changed", null)).toEqual([["restore", "--staged", "--worktree", "--source=HEAD", "--", "changed"]]);
  });

  it("stages, unstages and discards one file and leaves the other files alone", async () => {
    git("init", "-q", "-b", "main");
    writeFileSync(join(root, "a.txt"), "a\n");
    writeFileSync(join(root, "b.txt"), "b\n");
    writeFileSync(join(root, "old.txt"), "rename me\n");
    git("add", ".");
    git("commit", "-q", "-m", "init");
    writeFileSync(join(root, "a.txt"), "A\n");
    writeFileSync(join(root, "b.txt"), "B\n");
    writeFileSync(join(root, "new file.txt"), "new\n");

    expect(await runGitFileAction(root, "stage", await fileOf("a.txt"))).toEqual({ ok: true });
    expect((await fileOf("a.txt")).status).toBe("M ");
    expect((await fileOf("b.txt")).status).toBe(" M");

    expect(await runGitFileAction(root, "unstage", await fileOf("a.txt"))).toEqual({ ok: true });
    expect((await fileOf("a.txt")).status).toBe(" M");

    expect(await runGitFileAction(root, "discard", await fileOf("a.txt"))).toEqual({ ok: true });
    expect(await runGitFileAction(root, "discard", await fileOf("new file.txt"))).toEqual({ ok: true });
    const after = await statusOf();
    expect([...after.keys()]).toEqual(["b.txt"]);
    expect(existsSync(join(root, "new file.txt"))).toBe(false);
    expect(readFileSync(join(root, "a.txt"), "utf8").replace(/\r/g, "")).toBe("a\n");
    expect(readFileSync(join(root, "b.txt"), "utf8").replace(/\r/g, "")).toBe("B\n");
  });

  it("discards a staged new file and a staged rename", async () => {
    writeFileSync(join(root, "added.txt"), "x\n");
    git("add", "added.txt");
    git("mv", "old.txt", "renamed.txt");
    expect((await fileOf("renamed.txt")).status).toBe("R ");

    expect(await runGitFileAction(root, "discard", await fileOf("added.txt"))).toEqual({ ok: true });
    expect(await runGitFileAction(root, "discard", await fileOf("renamed.txt"))).toEqual({ ok: true });
    expect([...(await statusOf()).keys()]).toEqual(["b.txt"]);
    expect(existsSync(join(root, "old.txt"))).toBe(true);
    expect(existsSync(join(root, "renamed.txt"))).toBe(false);
  });

  it("refuses a file with a content filter, a path outside the repository and an unknown action", async () => {
    writeFileSync(join(root, ".gitattributes"), "*.bin filter=lfs\n");
    writeFileSync(join(root, "big.bin"), "data\n");
    const filtered = await runGitFileAction(root, "stage", await fileOf("big.bin"));
    expect(filtered.ok).toBe(false);
    expect((await fileOf("big.bin")).status).toBe("??");

    const outside = { path: "../escape.txt", originalPath: null, status: " M", untracked: false, directory: false, added: null, removed: null };
    await expect(runGitFileAction(root, "discard", outside)).rejects.toThrow();
    await expect(runGitFileAction(root, "push", await fileOf("b.txt"))).rejects.toThrow();
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
