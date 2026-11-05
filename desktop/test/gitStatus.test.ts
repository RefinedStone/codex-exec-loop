import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterAll, describe, expect, it } from "vitest";

import { parseNumstat, parsePorcelainStatus, readGitFileDiff, readGitStatus, resolveInside } from "../src/main/gitStatus";

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
    git("-c", "filter.lfs.process=", "-c", "filter.lfs.clean=", "add", ".");
    git("-c", "filter.lfs.process=", "-c", "filter.lfs.clean=", "commit", "-q", "-m", "lfs");
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
