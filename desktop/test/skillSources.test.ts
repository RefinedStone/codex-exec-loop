// 스킬 출처 추적 (#2383). 네트워크는 쓰지 않는다: 가짜 GitHub 이 답한다. 파일은 임시 폴더에만 쓴다.
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, renameSync, rmSync, symlinkSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import { classifyInstall, installRoot, scanClaudeSkills, skillKey } from "../src/main/skillInstall";
import { assertBundle, manifestHash, readSkillFiles, relativePathProblem, SKILL_LIMITS, type SkillFile } from "../src/main/skillManifest";
import { SkillService } from "../src/main/skillService";
import { fetchBundle, normalizeSource, resolveCommit, type SkillFetch, SkillSourceError } from "../src/main/skillSource";
import { SkillStore } from "../src/main/skillStore";
import { classifySkill, type SkillIdentity, type SkillLedgerEntry, type SkillRow, type SkillSourceInput } from "../src/shared/skills";

const sha = (algorithm: string, data: string | Buffer) => createHash(algorithm).update(data).digest("hex");
const file = (path: string, text: string, executable = false): SkillFile => ({ path, bytes: Buffer.from(text), executable });

type RepoFiles = Record<string, string | { text: string; mode: string; type?: string }>;

/** 공개 GitHub 의 읽기 API 를 흉내 낸다: 릴리스, 커밋 해소, 트리, raw 파일. */
function fakeGitHub() {
  const commits = new Map<string, RepoFiles>();
  const refs = new Map<string, string>();
  const state = { release: null as string | null, repositoryStatus: 200, failure: null as null | "offline" | "rateLimited", truncated: false, corruptRaw: false };
  const calls: string[] = [];
  const respond = (status: number, body: string | Buffer = "") => ({ status, headers: { get: () => null }, arrayBuffer: async () => new Uint8Array(Buffer.from(body)).buffer as ArrayBuffer });
  const blobSha = (bytes: Buffer) => createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
  const fetcher: SkillFetch = async (url) => {
    calls.push(url);
    if (state.failure === "offline") throw new Error("net::ERR_INTERNET_DISCONNECTED");
    if (state.failure === "rateLimited") return respond(403, "{}");
    const api = /^https:\/\/api\.github\.com\/repos\/sample\/skills(\/.*)?$/.exec(url);
    if (api) {
      if (state.repositoryStatus !== 200) return respond(state.repositoryStatus);
      const path = api[1] ?? "";
      if (path === "") return respond(200, "{}");
      if (path === "/releases/latest") return state.release ? respond(200, JSON.stringify({ tag_name: state.release })) : respond(404);
      if (path.startsWith("/commits/")) {
        const ref = decodeURIComponent(path.slice("/commits/".length));
        const commit = refs.get(ref) ?? (commits.has(ref) ? ref : undefined);
        return commit ? respond(200, commit) : respond(422);
      }
      const tree = /^\/git\/trees\/([0-9a-f]{40})\?recursive=1$/.exec(path);
      const files = tree ? commits.get(tree[1]!) : undefined;
      if (files) {
        const entries = Object.entries(files).map(([entryPath, value]) => {
          const text = typeof value === "string" ? value : value.text;
          const bytes = Buffer.from(text);
          return { path: entryPath, mode: typeof value === "string" ? "100644" : value.mode, type: typeof value === "string" ? "blob" : (value.type ?? "blob"), sha: blobSha(bytes), size: bytes.length };
        });
        return respond(200, JSON.stringify({ tree: [{ path: "skills", mode: "040000", type: "tree", sha: "0".repeat(40) }, ...entries], truncated: state.truncated }));
      }
      return respond(404);
    }
    const raw = /^https:\/\/raw\.githubusercontent\.com\/sample\/skills\/([0-9a-f]{40})\/(.*)$/.exec(url);
    const value = raw ? commits.get(raw[1]!)?.[decodeURIComponent(raw[2]!)] : undefined;
    if (value === undefined) return respond(404);
    return respond(200, state.corruptRaw ? "tampered" : typeof value === "string" ? value : value.text);
  };
  /** 커밋을 만들고 ref 를 그 커밋으로 옮긴다. */
  const commit = (files: RepoFiles, ...names: string[]): string => {
    const id = sha("sha1", `${commits.size}:${JSON.stringify(files)}`);
    commits.set(id, files);
    for (const name of names) refs.set(name, id);
    return id;
  };
  return { fetcher, commit, state, calls };
}

type FakeGitHub = ReturnType<typeof fakeGitHub>;

const CAVEMAN_V1: RepoFiles = {
  "README.md": "root readme v1\n",
  "skills/caveman/SKILL.md": "---\nname: caveman\n---\ncaveman v1\n",
  "skills/caveman/notes.md": "notes v1\n",
  "skills/caveman/old.md": "old\n",
  "skills/caveman-extra/SKILL.md": "---\nname: caveman-extra\n---\nextra\n",
  "skills/ultracave/SKILL.md": "---\nname: ultracave\n---\nultra v1\n",
};
const CAVEMAN_V2: RepoFiles = {
  ...Object.fromEntries(Object.entries(CAVEMAN_V1).filter(([path]) => path !== "skills/caveman/old.md")),
  "skills/caveman/SKILL.md": "---\nname: caveman\nallowed-tools: Bash\n---\ncaveman v2\n",
  "skills/caveman/scripts/run.sh": { text: "#!/bin/sh\necho hi\n", mode: "100755" },
  "skills/megacave/SKILL.md": "---\nname: megacave\n---\nmega\n",
};
const SOURCE: SkillSourceInput = { repository: "sample/skills", subPath: "skills/caveman", policy: { kind: "branch", branch: "main" } };

describe("skill sources", () => {
  const roots: string[] = [];
  afterEach(() => {
    for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
  });

  /** 임시 홈, 작업 폴더, 앱 데이터 폴더이다. 사용자의 실제 `~/.claude` 와 `~/.codex` 는 건드리지 않는다. */
  function sandbox() {
    // macOS 의 임시 폴더는 링크(`/var`)이다. 실제 경로로 쓴다.
    const root = realpathSync(mkdtempSync(join(tmpdir(), "akra-skills-")));
    roots.push(root);
    const home = join(root, "home");
    const project = join(root, "project");
    const dataDir = join(root, "data");
    const install = (base: string, runtimeDir: string, name: string, files: Record<string, string>): string => {
      const directory = join(base, runtimeDir, "skills", name);
      for (const [path, text] of Object.entries(files)) {
        mkdirSync(dirname(join(directory, path)), { recursive: true });
        writeFileSync(join(directory, path), text);
      }
      return directory;
    };
    return { root, home, project, dataDir, install };
  }

  const LOCAL_V1 = { "SKILL.md": "---\nname: caveman\n---\ncaveman v1\n", "notes.md": "notes v1\n", "old.md": "old\n" };
  const tree = (directory: string): Record<string, string> => Object.fromEntries(readSkillFiles(directory).map((item) => [item.path, item.bytes.toString("utf8")]));
  const yes = async () => true;

  /** 홈에 caveman v1 복사본이 있고, 가짜 저장소의 main 이 v1 인 상태에서 출처를 연결한다. */
  async function linkedCaveman(options: { rename?: (from: string, to: string) => void } = {}) {
    const box = sandbox();
    const github = fakeGitHub();
    const v1 = github.commit(CAVEMAN_V1, "heads/main");
    const installPath = box.install(box.home, ".claude", "caveman", LOCAL_V1);
    box.install(box.home, ".claude", "ultracave", { "SKILL.md": "local ultracave\n" });
    const service = new SkillService({ dataDir: box.dataDir, home: box.home, fetcher: github.fetcher, now: () => new Date("2026-11-20T00:00:00Z"), rename: options.rename });
    const identity: SkillIdentity = { runtime: "claude", host: "local", scope: "user", installPath };
    const row = (): SkillRow => service.list({ cwd: null, codex: [] }).find((item) => item.identity.installPath === installPath)!;
    expect(await service.link(identity, SOURCE)).toEqual({ ok: true });
    return { ...box, github, v1, installPath, service, identity, row };
  }

  describe("manifest hash rule v1", () => {
    it("hashes every file of the bundle with a fixed, versioned rule", () => {
      const files = [file("b/notes.md", "notes\n"), file("SKILL.md", "skill\n")];
      const expected = `v1:${sha("sha256", `SKILL.md\0${sha("sha256", "skill\n")}\nb/notes.md\0${sha("sha256", "notes\n")}\n`)}`;
      expect(manifestHash(files)).toBe(expected);
      expect(manifestHash([...files].reverse())).toBe(expected);
    });

    it("changes when a file other than SKILL.md changes, is added, or is renamed", () => {
      const base = manifestHash([file("SKILL.md", "skill\n"), file("notes.md", "notes\n")]);
      expect(manifestHash([file("SKILL.md", "skill\n"), file("notes.md", "notes!\n")])).not.toBe(base);
      expect(manifestHash([file("SKILL.md", "skill\n"), file("notes.md", "notes\n"), file("more.md", "")])).not.toBe(base);
      expect(manifestHash([file("SKILL.md", "skill\n"), file("Notes.md", "notes\n")])).not.toBe(base);
    });

    it("hashes bytes as they are: CRLF is a different bundle, the executable bit is not part of the hash", () => {
      expect(manifestHash([file("SKILL.md", "a\r\nb\r\n")])).not.toBe(manifestHash([file("SKILL.md", "a\nb\n")]));
      expect(manifestHash([file("SKILL.md", "a\n", true)])).toBe(manifestHash([file("SKILL.md", "a\n", false)]));
    });

    it("reads an installed folder with `/` paths and skips .DS_Store", () => {
      const box = sandbox();
      const directory = box.install(box.home, ".claude", "caveman", { "SKILL.md": "skill\n", "docs/deep/notes.md": "notes\n", ".DS_Store": "finder", "docs/.DS_Store": "finder" });
      expect(readSkillFiles(directory).map((item) => item.path)).toEqual(["SKILL.md", "docs/deep/notes.md"]);
      expect(manifestHash(readSkillFiles(directory))).toBe(manifestHash([file("SKILL.md", "skill\n"), file("docs/deep/notes.md", "notes\n")]));
    });
  });

  describe("path, link, and file checks", () => {
    it.each(["", "/etc/passwd", "../outside.md", "a/../../b", "a//b", "./a", "a\\b", "C:/x.md", "C:\\x.md", "a/con.txt", "NUL", "a/trailing.", "a/space ", ".git/config", "a/\u0000b"])(
      "rejects the path %j",
      (path) => {
        expect(relativePathProblem(path)).not.toBeNull();
      },
    );

    it("accepts ordinary nested paths", () => {
      for (const path of ["SKILL.md", "scripts/run.sh", "docs/한글 문서.md", ".hidden/x"]) expect(relativePathProblem(path)).toBeNull();
    });

    it("rejects a bundle with case-only duplicates, no SKILL.md, or over the size limits", () => {
      expect(() => assertBundle([file("SKILL.md", ""), file("a.md", ""), file("A.md", "")])).toThrow(/대소문자/);
      expect(() => assertBundle([file("notes.md", "")])).toThrow(/SKILL\.md/);
      expect(() => assertBundle([file("SKILL.md", ""), { path: "big.bin", bytes: Buffer.alloc(SKILL_LIMITS.fileBytes + 1), executable: false }])).toThrow(/너무 큽니다/);
    });

    it.skipIf(process.platform === "win32")("refuses an installed folder that holds a symbolic link, without following it", () => {
      const box = sandbox();
      const directory = box.install(box.home, ".claude", "caveman", { "SKILL.md": "skill\n" });
      writeFileSync(join(box.root, "secret.txt"), "secret");
      symlinkSync(join(box.root, "secret.txt"), join(directory, "link.md"));
      expect(() => readSkillFiles(directory)).toThrow(/심볼릭 링크/);
      rmSync(join(directory, "link.md"));
      symlinkSync(box.root, join(directory, "outside"));
      expect(() => readSkillFiles(directory)).toThrow(/심볼릭 링크/);
    });

    it("knows only skill folders of the matching runtime", () => {
      const box = sandbox();
      const claude = box.install(box.home, ".claude", "caveman", { "SKILL.md": "x" });
      const agents = box.install(box.project, ".agents", "caveman", { "SKILL.md": "x" });
      const system = box.install(box.home, ".codex", join(".system", "builtin"), { "SKILL.md": "x" });
      mkdirSync(join(box.root, "elsewhere", "caveman"), { recursive: true });
      writeFileSync(join(box.root, "elsewhere", "caveman", "SKILL.md"), "x");
      expect(classifyInstall("claude", claude, box.home)).toEqual({ management: "copy", scope: "user" });
      expect(classifyInstall("codex", agents, box.home)).toEqual({ management: "copy", scope: "project" });
      expect(classifyInstall("codex", claude, box.home)).toEqual({ management: "external" });
      expect(classifyInstall("codex", system, box.home)).toEqual({ management: "external" });
      expect(classifyInstall("claude", join(box.root, "elsewhere", "caveman"), box.home)).toEqual({ management: "external" });
      expect(installRoot("claude", `${claude}/../caveman`)).toBeNull();
      expect(installRoot("claude", "relative/.claude/skills/caveman")).toBeNull();
    });

    it.skipIf(process.platform === "win32")("treats a linked skill folder or a linked skills folder as not writable", () => {
      const box = sandbox();
      const real = box.install(box.root, "dotfiles", "caveman", { "SKILL.md": "x" });
      mkdirSync(join(box.home, ".claude", "skills"), { recursive: true });
      symlinkSync(real, join(box.home, ".claude", "skills", "caveman"));
      expect(classifyInstall("claude", join(box.home, ".claude", "skills", "caveman"), box.home)).toEqual({ management: "symlink" });
      mkdirSync(join(box.project, ".claude"), { recursive: true });
      symlinkSync(dirname(real), join(box.project, ".claude", "skills"));
      expect(classifyInstall("claude", join(box.project, ".claude", "skills", "caveman"), box.home)).toEqual({ management: "symlink" });
      expect(scanClaudeSkills(box.home, box.project).map((item) => `${item.scope}:${item.name}`)).toEqual(["user:caveman", "project:caveman"]);
    });
  });

  describe("source path and shared ledger regressions", () => {
    it.each([[".claude", "claude"], [".codex", "codex"], [".agents", "codex"]] as const)("does not follow a linked %s parent while classifying a writable install", (parent, runtime) => {
      const box = sandbox();
      const real = box.install(box.root, "dotfiles", "caveman", { "SKILL.md": "linked source" });
      mkdirSync(box.home, { recursive: true });
      symlinkSync(dirname(dirname(real)), join(box.home, parent), process.platform === "win32" ? "junction" : "dir");
      expect(classifyInstall(runtime, join(box.home, parent, "skills", "caveman"), box.home)).toEqual({ management: "symlink" });
      expect(tree(real)).toEqual({ "SKILL.md": "linked source" });
    });

    it("does not update any ledger key while a different process owns the shared ledger lock", async () => {
      const box = await linkedCaveman();
      const store = new SkillStore(box.dataDir);
      const existing = store.readLedger();
      const entry = existing[skillKey(box.identity)]!;
      const lock = join(box.dataDir, "locks", "ledger.lock");
      mkdirSync(dirname(lock), { recursive: true });
      const child = spawn(process.execPath, ["-e", 'const fs=require("node:fs");fs.writeFileSync(process.argv[1],String(process.pid),{flag:"wx"});console.log("locked");process.stdin.once("data",()=>{fs.unlinkSync(process.argv[1]);process.exit(0)});', lock], { stdio: ["pipe", "pipe", "pipe"] });
      const exited = once(child, "exit");
      try {
        await Promise.race([once(child.stdout, "data"), exited.then(([code]) => { throw new Error("lock owner exited before ready: " + code); })]);
        expect(() => store.writeEntry("another-skill", entry)).toThrow(/다른 갱신/);
        expect(store.readLedger()).toEqual(existing);
      } finally {
        if (child.exitCode === null) child.stdin.end("release");
        await exited;
      }
      store.writeEntry("another-skill", entry);
      expect(store.readLedger()).toEqual({ ...existing, "another-skill": entry });
      expect(readdirSync(box.dataDir).filter((name) => name.includes(".tmp"))).toEqual([]);
    });

    it("does not reclaim a lock whose owner is still being initialized", async () => {
      const box = await linkedCaveman();
      const store = new SkillStore(box.dataDir);
      const existing = store.readLedger();
      const lock = join(box.dataDir, "locks", "ledger.lock");
      mkdirSync(dirname(lock), { recursive: true });
      writeFileSync(lock, "");
      expect(() => store.writeEntry("another-skill", existing[skillKey(box.identity)]!)).toThrow(/다른 갱신/);
      expect(store.readLedger()).toEqual(existing);
    });
  });

  describe("state classification", () => {
    const entry = (patch: Partial<SkillLedgerEntry>): SkillLedgerEntry => ({
      identity: { runtime: "claude", host: "local", scope: "user", installPath: "/x" },
      source: { repository: "sample/skills", subPath: "skills/caveman", policy: { kind: "branch", branch: "main" } },
      manifestVersion: 1,
      installedCommit: "a".repeat(40),
      installedManifestHash: "v1:installed",
      lastSuccess: { at: "t", ref: "main", commit: "a".repeat(40), manifestHash: "v1:installed" },
      lastFailure: null,
      ...patch,
    });
    const newer = { at: "t", ref: "main", commit: "b".repeat(40), manifestHash: "v1:newer" };

    it("keeps every state distinct", () => {
      expect(classifySkill(null, "v1:installed")).toEqual({ status: "sourceUnknown", localModified: false, canReplace: false });
      expect(classifySkill(entry({ lastSuccess: null }), "v1:installed").status).toBe("notChecked");
      expect(classifySkill(entry({}), "v1:installed")).toEqual({ status: "latest", localModified: false, canReplace: false });
      expect(classifySkill(entry({ lastSuccess: newer }), "v1:installed")).toEqual({ status: "updateAvailable", localModified: false, canReplace: true });
      expect(classifySkill(entry({ installedCommit: null, installedManifestHash: null, lastSuccess: newer }), "v1:local")).toEqual({ status: "baselineUnknown", localModified: false, canReplace: true });
    });

    it("shows a local modification together with a remote update", () => {
      expect(classifySkill(entry({ lastSuccess: newer }), "v1:edited")).toEqual({ status: "updateAvailable", localModified: true, canReplace: true });
      expect(classifySkill(entry({}), "v1:edited")).toEqual({ status: "latest", localModified: true, canReplace: true });
    });

    it("never calls a failed check latest, even when the last success was latest", () => {
      const failed = entry({ lastFailure: { at: "t2", kind: "offline", message: "offline" } });
      expect(classifySkill(failed, "v1:installed")).toEqual({ status: "checkFailed", localModified: false, canReplace: false });
    });

    it("keeps a pinned install pinned, also when the pinned ref now points to another commit", () => {
      const pinned = entry({ source: { repository: "sample/skills", subPath: "skills/caveman", policy: { kind: "pinned", ref: "v1" } } });
      expect(classifySkill(pinned, "v1:installed")).toEqual({ status: "pinned", localModified: false, canReplace: false });
      expect(classifySkill({ ...pinned, lastSuccess: newer }, "v1:installed")).toEqual({ status: "pinned", localModified: false, canReplace: false });
      expect(classifySkill(pinned, "v1:edited")).toEqual({ status: "pinned", localModified: true, canReplace: true });
    });
  });

  describe("public GitHub source", () => {
    it("normalizes what the user typed and rejects what is not a public GitHub repository path", () => {
      expect(normalizeSource({ repository: " https://github.com/Sample/Skills.git ", subPath: "/skills/caveman/", policy: { kind: "release" } })).toEqual({
        repository: "sample/skills",
        subPath: "skills/caveman",
        policy: { kind: "release" },
      });
      for (const repository of ["sample", "https://gitlab.com/sample/skills", "sample/skills/extra", "sample/..", "../sample/skills", "sample/skills?x=1"]) {
        expect(() => normalizeSource({ repository, subPath: "", policy: { kind: "release" } })).toThrow(SkillSourceError);
      }
      expect(() => normalizeSource({ repository: "sample/skills", subPath: "../other", policy: { kind: "release" } })).toThrow(SkillSourceError);
      expect(() => normalizeSource({ repository: "sample/skills", subPath: "", policy: { kind: "branch", branch: "a..b" } })).toThrow(SkillSourceError);
      expect(() => normalizeSource({ repository: "sample/skills", subPath: "", policy: { kind: "pinned", ref: "" } })).toThrow(SkillSourceError);
      expect(() => normalizeSource({ repository: "sample/skills", subPath: "", policy: { kind: "auto" } })).toThrow(SkillSourceError);
    });

    it("resolves every policy to a commit", async () => {
      const github = fakeGitHub();
      const tagged = github.commit(CAVEMAN_V1, "tags/v1.0.0");
      const head = github.commit(CAVEMAN_V2, "heads/feat/next");
      github.state.release = "v1.0.0";
      const source = (policy: SkillSourceInput["policy"]) => normalizeSource({ ...SOURCE, policy });
      expect(await resolveCommit(github.fetcher, source({ kind: "release" }))).toEqual({ ref: "v1.0.0", commit: tagged });
      expect(await resolveCommit(github.fetcher, source({ kind: "branch", branch: "feat/next" }))).toEqual({ ref: "feat/next", commit: head });
      expect(await resolveCommit(github.fetcher, source({ kind: "pinned", ref: tagged }))).toEqual({ ref: tagged, commit: tagged });
    });

    it("does not switch to a branch when there is no stable release", async () => {
      const github = fakeGitHub();
      github.commit(CAVEMAN_V1, "heads/main");
      await expect(resolveCommit(github.fetcher, normalizeSource({ ...SOURCE, policy: { kind: "release" } }))).rejects.toMatchObject({ kind: "noRelease" });
      expect(github.calls.some((url) => url.includes("/commits/"))).toBe(false);
    });

    it("takes only the files under the skill sub-path: sibling skills and the root README stay out", async () => {
      const github = fakeGitHub();
      const commit = github.commit(CAVEMAN_V2);
      const files = await fetchBundle(github.fetcher, "sample/skills", "skills/caveman", commit);
      expect(files.map((item) => item.path).sort()).toEqual(["SKILL.md", "notes.md", "scripts/run.sh"]);
      expect(files.find((item) => item.path === "scripts/run.sh")?.executable).toBe(true);
      expect(github.calls.filter((url) => url.includes("raw.githubusercontent.com")).every((url) => url.includes(`/${commit}/skills/caveman/`))).toBe(true);
    });

    it.each([
      ["a symbolic link", { "skills/caveman/link.md": { text: "/etc/passwd", mode: "120000" } }],
      ["a submodule", { "skills/caveman/vendor": { text: "", mode: "160000", type: "commit" } }],
      ["a backslash path", { "skills/caveman/a\\b.md": "x" }],
      ["a drive path", { "skills/caveman/C:/x.md": "x" }],
      ["a file over the size limit", { "skills/caveman/big.bin": "x".repeat(SKILL_LIMITS.fileBytes + 1) }],
    ])("rejects the whole bundle when the source holds %s", async (_name, extra) => {
      const github = fakeGitHub();
      const commit = github.commit({ ...CAVEMAN_V1, ...extra });
      await expect(fetchBundle(github.fetcher, "sample/skills", "skills/caveman", commit)).rejects.toMatchObject({ kind: "invalid" });
      expect(github.calls.some((url) => url.includes("raw.githubusercontent.com"))).toBe(false);
    });

    it("rejects a download that is not the content of the commit, and a truncated tree", async () => {
      const github = fakeGitHub();
      const commit = github.commit(CAVEMAN_V1);
      github.state.corruptRaw = true;
      await expect(fetchBundle(github.fetcher, "sample/skills", "skills/caveman", commit)).rejects.toThrow(/커밋의 내용과 다릅니다/);
      github.state.corruptRaw = false;
      github.state.truncated = true;
      await expect(fetchBundle(github.fetcher, "sample/skills", "skills/caveman", commit)).rejects.toMatchObject({ kind: "invalid" });
    });

    it("reports a sub-path without SKILL.md as missing", async () => {
      const github = fakeGitHub();
      const commit = github.commit(CAVEMAN_V1);
      await expect(fetchBundle(github.fetcher, "sample/skills", "skills/gone", commit)).rejects.toMatchObject({ kind: "missing" });
    });
  });

  describe("installed list", () => {
    it("keeps same-name installs of different scopes and runtimes as separate rows and marks managed installs read-only", () => {
      const box = sandbox();
      const userClaude = box.install(box.home, ".claude", "caveman", LOCAL_V1);
      const projectClaude = box.install(box.project, ".claude", "caveman", { "SKILL.md": "project copy\n" });
      const userCodex = box.install(box.home, ".codex", "caveman", LOCAL_V1);
      const plugin = join(box.home, ".codex", "plugins", "market", "skills", "caveman");
      const system = box.install(box.home, ".codex", join(".system", "builtin"), { "SKILL.md": "x" });
      const service = new SkillService({ dataDir: box.dataDir, home: box.home, fetcher: fakeGitHub().fetcher });
      const rows = service.list({
        cwd: box.project,
        codex: [
          { name: "caveman", path: join(userCodex, "SKILL.md"), scope: "user", enabled: true, pluginId: null },
          { name: "caveman", path: join(plugin, "SKILL.md"), scope: "user", enabled: true, pluginId: "caveman@market" },
          { name: "builtin", path: join(system, "SKILL.md"), scope: "system", enabled: false, pluginId: null },
          { name: "bad", path: "relative/SKILL.md", scope: "user", enabled: true, pluginId: null },
        ],
      });
      const summary = rows.map((row) => `${row.identity.runtime}:${row.identity.scope}:${row.name}:${row.management}:${row.state.status}`).sort();
      expect(summary).toEqual([
        "claude:project:caveman:copy:sourceUnknown",
        "claude:user:caveman:copy:sourceUnknown",
        "codex:system:builtin:system:sourceUnknown",
        "codex:user:caveman:copy:sourceUnknown",
        "codex:user:caveman:plugin:sourceUnknown",
      ]);
      expect(new Set(rows.map((row) => row.key)).size).toBe(5);
      expect(rows.map((row) => row.identity.installPath).sort()).toEqual([projectClaude, userClaude, system, userCodex, plugin].sort());
      // 같은 내용의 복사본도 이름이나 내용으로 묶지 않는다: 출처는 행마다 따로 연결한다.
      expect(rows.every((row) => row.entry === null && !row.state.canReplace && !row.hasSnapshot)).toBe(true);
      expect(rows.find((row) => row.management === "plugin")?.localHash).toBeNull();
      expect(rows.find((row) => row.identity.runtime === "claude")?.enabled).toBeNull();
    });

    it("does not change files or link anything before the user links a source", async () => {
      const box = sandbox();
      const installPath = box.install(box.home, ".claude", "caveman", LOCAL_V1);
      const github = fakeGitHub();
      github.commit(CAVEMAN_V2, "heads/main");
      const service = new SkillService({ dataDir: box.dataDir, home: box.home, fetcher: github.fetcher });
      const identity: SkillIdentity = { runtime: "claude", host: "local", scope: "user", installPath };
      const local = service.list({ cwd: null, codex: [] })[0]!.localHash!;
      for (const result of [await service.check(identity), await service.preview(identity), await service.apply(identity, { commit: "a".repeat(40), targetHash: "v1:x", localHash: local }, yes), await service.rollback(identity, local, yes)]) {
        expect(result.ok).toBe(false);
      }
      expect(tree(installPath)).toEqual(LOCAL_V1);
      expect(github.calls).toEqual([]);
      expect(existsSync(join(box.dataDir, "ledger.json"))).toBe(false);
    });

    it("refuses every operation on an install that Akra does not own", async () => {
      const box = sandbox();
      const plugin = join(box.home, ".codex", "plugins", "market", "skills", "caveman");
      mkdirSync(plugin, { recursive: true });
      writeFileSync(join(plugin, "SKILL.md"), "plugin\n");
      const service = new SkillService({ dataDir: box.dataDir, home: box.home, fetcher: fakeGitHub().fetcher });
      for (const identity of [
        { runtime: "codex", host: "local", scope: "user", installPath: plugin },
        { runtime: "claude", host: "local", scope: "user", installPath: join(box.home, ".claude", "skills", "..", "..", ".ssh") },
        { runtime: "claude", host: "cloud", scope: "user", installPath: box.install(box.home, ".claude", "caveman", LOCAL_V1) },
        // 범위가 경로와 다르다.
        { runtime: "claude", host: "local", scope: "project", installPath: join(box.home, ".claude", "skills", "caveman") },
      ]) {
        const result = await service.link(identity, SOURCE);
        expect(result.ok).toBe(false);
      }
      expect(existsSync(join(box.dataDir, "ledger.json"))).toBe(false);
    });
  });

  describe("link and check", () => {
    it("records the commit as installed only when the bundle of that commit matches the local files", async () => {
      const matched = await linkedCaveman();
      expect(matched.row().entry).toMatchObject({ installedCommit: matched.v1, installedManifestHash: matched.row().localHash, source: { repository: "sample/skills", subPath: "skills/caveman" } });
      expect(matched.row().state).toEqual({ status: "latest", localModified: false, canReplace: false });

      const box = sandbox();
      const github = fakeGitHub();
      github.commit(CAVEMAN_V2, "heads/main");
      const installPath = box.install(box.home, ".claude", "caveman", LOCAL_V1);
      const service = new SkillService({ dataDir: box.dataDir, home: box.home, fetcher: github.fetcher });
      expect(await service.link({ runtime: "claude", host: "local", scope: "user", installPath }, SOURCE)).toEqual({ ok: true });
      const row = service.list({ cwd: null, codex: [] })[0]!;
      expect(row.entry).toMatchObject({ installedCommit: null, installedManifestHash: null });
      expect(row.state).toEqual({ status: "baselineUnknown", localModified: false, canReplace: true });
      expect(tree(installPath)).toEqual(LOCAL_V1);
    });

    it("shows no update when only another skill or the root README changed", async () => {
      const linked = await linkedCaveman();
      const next = linked.github.commit({ ...CAVEMAN_V1, "README.md": "root readme v2\n", "skills/ultracave/SKILL.md": "ultra v2\n", "skills/megacave/SKILL.md": "new sibling\n" }, "heads/main");
      expect(await linked.service.check(linked.identity)).toEqual({ ok: true });
      expect(linked.row().entry?.lastSuccess?.commit).toBe(next);
      expect(linked.row().state).toEqual({ status: "latest", localModified: false, canReplace: false });
      expect((await linked.service.preview(linked.identity)).ok).toBe(false);
    });

    it("shows the new commit and the per-file changes when the tracked skill changed", async () => {
      const linked = await linkedCaveman();
      const v2 = linked.github.commit(CAVEMAN_V2, "heads/main");
      await linked.service.check(linked.identity);
      expect(linked.row().state).toEqual({ status: "updateAvailable", localModified: false, canReplace: true });
      const result = await linked.service.preview(linked.identity);
      if (!result.ok) throw new Error(result.message);
      expect(result.preview).toMatchObject({ currentCommit: linked.v1, targetCommit: v2, localHash: linked.row().localHash, targetHash: linked.row().entry?.lastSuccess?.manifestHash });
      expect(result.preview.files.map((item) => `${item.change}:${item.path}:${item.behavior ? "behavior" : "text"}`)).toEqual([
        "modify:SKILL.md:behavior",
        "delete:old.md:text",
        "add:scripts/run.sh:behavior",
      ]);
      expect(result.preview.files[0]).toMatchObject({ oldText: LOCAL_V1["SKILL.md"], newText: "---\nname: caveman\nallowed-tools: Bash\n---\ncaveman v2\n" });
      // 미리보기는 아무것도 쓰지 않는다.
      expect(tree(linked.installPath)).toEqual(LOCAL_V1);
    });

    it.each([
      ["offline", (github: FakeGitHub): void => {
        github.state.failure = "offline";
      }, "offline"],
      ["rate limited", (github: FakeGitHub): void => {
        github.state.failure = "rateLimited";
      }, "rateLimited"],
      ["moved", (github: FakeGitHub): void => {
        github.state.repositoryStatus = 301;
      }, "moved"],
      ["deleted", (github: FakeGitHub): void => {
        github.state.repositoryStatus = 404;
      }, "missing"],
    ] as const)("keeps the last success and never claims latest when the source is %s", async (_name, breakSource, kind) => {
      const linked = await linkedCaveman();
      const success = linked.row().entry?.lastSuccess;
      breakSource(linked.github);
      const result = await linked.service.check(linked.identity);
      expect(result.ok).toBe(false);
      const row = linked.row();
      expect(row.state.status).toBe("checkFailed");
      expect(row.entry?.lastFailure?.kind).toBe(kind);
      expect(row.entry?.lastSuccess).toEqual(success);
      expect(row.entry?.installedCommit).toBe(linked.v1);
      expect((await linked.service.preview(linked.identity)).ok).toBe(false);
    });

    it("reports a deleted skill folder of the source as missing", async () => {
      const linked = await linkedCaveman();
      linked.github.commit({ "README.md": "moved away\n" }, "heads/main");
      await linked.service.check(linked.identity);
      expect(linked.row().entry?.lastFailure?.kind).toBe("missing");
    });

    it("does not raise a pinned install when the branch moves", async () => {
      const linked = await linkedCaveman();
      expect(await linked.service.link(linked.identity, { ...SOURCE, policy: { kind: "pinned", ref: linked.v1 } })).toEqual({ ok: true });
      linked.github.commit(CAVEMAN_V2, "heads/main");
      await linked.service.check(linked.identity);
      expect(linked.row().state).toEqual({ status: "pinned", localModified: false, canReplace: false });
      expect(linked.row().entry?.installedCommit).toBe(linked.v1);
      expect((await linked.service.preview(linked.identity)).ok).toBe(false);
    });

    it("drops the installed commit when the user links another repository path", async () => {
      const linked = await linkedCaveman();
      await linked.service.link(linked.identity, { ...SOURCE, subPath: "skills/ultracave" });
      expect(linked.row().entry).toMatchObject({ installedCommit: null, installedManifestHash: null });
      expect(linked.row().state.status).toBe("baselineUnknown");
    });
  });

  describe("apply and rollback", () => {
    async function withUpdate(options: { rename?: (from: string, to: string) => void } = {}) {
      const linked = await linkedCaveman(options);
      const v2 = linked.github.commit(CAVEMAN_V2, "heads/main");
      await linked.service.check(linked.identity);
      const approve = async () => {
        const result = await linked.service.preview(linked.identity);
        if (!result.ok) throw new Error(result.message);
        return { commit: result.preview.targetCommit, targetHash: result.preview.targetHash, localHash: result.preview.localHash };
      };
      const leftovers = () => readdirSync(dirname(linked.installPath)).filter((name) => name.startsWith(".akra-skill-"));
      return { ...linked, v2, approve, leftovers };
    }
    const TARGET_V2 = { "SKILL.md": "---\nname: caveman\nallowed-tools: Bash\n---\ncaveman v2\n", "notes.md": "notes v1\n", "scripts/run.sh": "#!/bin/sh\necho hi\n" };

    it("replaces the install with exactly the approved commit and records it", async () => {
      const update = await withUpdate();
      const before = update.row();
      expect(await update.service.apply(update.identity, await update.approve(), yes)).toEqual({ ok: true });
      expect(tree(update.installPath)).toEqual(TARGET_V2);
      const row = update.row();
      expect(row.entry).toMatchObject({ installedCommit: update.v2, installedManifestHash: row.localHash });
      expect(row.state).toEqual({ status: "latest", localModified: false, canReplace: false });
      expect(row.hasSnapshot).toBe(true);
      expect(row.key).toBe(before.key);
      expect(update.leftovers()).toEqual([]);
      expect(readdirSync(join(update.dataDir, "journal"))).toEqual([]);
    });

    it("never touches sibling skills: ultracave and megacave do not follow a caveman update", async () => {
      const update = await withUpdate();
      await update.service.apply(update.identity, await update.approve(), yes);
      const skills = join(update.home, ".claude", "skills");
      expect(readdirSync(skills).sort()).toEqual(["caveman", "ultracave"]);
      expect(tree(join(skills, "ultracave"))).toEqual({ "SKILL.md": "local ultracave\n" });
    });

    it("changes nothing when the user cancels", async () => {
      const update = await withUpdate();
      expect(await update.service.apply(update.identity, await update.approve(), async () => false)).toMatchObject({ ok: false, cancelled: true });
      expect(tree(update.installPath)).toEqual(LOCAL_V1);
      expect(update.row().entry?.installedCommit).toBe(update.v1);
      expect(update.row().hasSnapshot).toBe(false);
    });

    it("stops when a local file changed after the preview", async () => {
      const update = await withUpdate();
      const approval = await update.approve();
      writeFileSync(join(update.installPath, "notes.md"), "my local edit\n");
      expect(await update.service.apply(update.identity, approval, yes)).toMatchObject({ ok: false, message: expect.stringContaining("미리보기 뒤에") });
      expect(tree(update.installPath)).toEqual({ ...LOCAL_V1, "notes.md": "my local edit\n" });
    });

    it("stops when a local file changes while the confirmation is open", async () => {
      const update = await withUpdate();
      const approval = await update.approve();
      const editDuringConfirm = async () => {
        writeFileSync(join(update.installPath, "added.md"), "added while confirming\n");
        return true;
      };
      expect(await update.service.apply(update.identity, approval, editDuringConfirm)).toMatchObject({ ok: false });
      expect(tree(update.installPath)).toEqual({ ...LOCAL_V1, "added.md": "added while confirming\n" });
      expect(update.row().entry?.installedCommit).toBe(update.v1);
      expect(update.leftovers()).toEqual([]);
    });

    it("applies only the commit and hash of the last check", async () => {
      const update = await withUpdate();
      const approval = await update.approve();
      expect((await update.service.apply(update.identity, { ...approval, commit: update.v1 }, yes)).ok).toBe(false);
      expect((await update.service.apply(update.identity, { ...approval, targetHash: "v1:other" }, yes)).ok).toBe(false);
      // 승인한 뒤에 원본이 또 바뀌었다: 다시 확인하면 예전 승인으로는 적용하지 않는다.
      update.github.commit({ ...CAVEMAN_V2, "skills/caveman/notes.md": "notes v3\n" }, "heads/main");
      await update.service.check(update.identity);
      expect((await update.service.apply(update.identity, approval, yes)).ok).toBe(false);
      expect(tree(update.installPath)).toEqual(LOCAL_V1);
    });

    it("overwrites a local modification without merging and restores it, with the ledger, on rollback", async () => {
      const update = await withUpdate();
      writeFileSync(join(update.installPath, "notes.md"), "my local edit\n");
      const modified = { ...LOCAL_V1, "notes.md": "my local edit\n" };
      expect(update.row().state).toEqual({ status: "updateAvailable", localModified: true, canReplace: true });
      const ledgerBefore = update.row().entry;
      expect(await update.service.apply(update.identity, await update.approve(), yes)).toEqual({ ok: true });
      expect(tree(update.installPath)).toEqual(TARGET_V2);

      expect(await update.service.rollback(update.identity, update.row().localHash!, yes)).toEqual({ ok: true });
      expect(tree(update.installPath)).toEqual(modified);
      expect(update.row().entry).toEqual(ledgerBefore);
      expect(update.row().state).toEqual({ status: "updateAvailable", localModified: true, canReplace: true });
      // 되돌린 것을 다시 되돌릴 수 있다.
      expect(await update.service.rollback(update.identity, update.row().localHash!, yes)).toEqual({ ok: true });
      expect(tree(update.installPath)).toEqual(TARGET_V2);
      expect(update.row().entry?.installedCommit).toBe(update.v2);
    });

    it("stops a rollback when the files changed after the list was read, and on cancel", async () => {
      const update = await withUpdate();
      await update.service.apply(update.identity, await update.approve(), yes);
      const seen = update.row().localHash!;
      expect(await update.service.rollback(update.identity, seen, async () => false)).toMatchObject({ ok: false, cancelled: true });
      writeFileSync(join(update.installPath, "notes.md"), "edited after apply\n");
      expect((await update.service.rollback(update.identity, seen, yes)).ok).toBe(false);
      expect(tree(update.installPath)).toEqual({ ...TARGET_V2, "notes.md": "edited after apply\n" });
      expect(update.row().entry?.installedCommit).toBe(update.v2);
    });

    it.each([1, 2])("leaves the old install active when moving folder %i fails (a locked file)", async (failing) => {
      let moves = 0;
      const rename = (from: string, to: string) => {
        moves += 1;
        if (moves === failing) throw Object.assign(new Error("EBUSY: resource busy or locked"), { code: "EBUSY" });
        renameSync(from, to);
      };
      const update = await withUpdate({ rename });
      const result = await update.service.apply(update.identity, await update.approve(), yes);
      expect(result).toMatchObject({ ok: false, message: expect.stringContaining("이전 상태로 되돌렸습니다") });
      expect(tree(update.installPath)).toEqual(LOCAL_V1);
      expect(update.row().entry?.installedCommit).toBe(update.v1);
      expect(update.row().hasSnapshot).toBe(false);
      expect(update.leftovers()).toEqual([]);
      expect(readdirSync(join(update.dataDir, "journal"))).toEqual([]);
    });

    it("stops while another update holds the lock of the install", async () => {
      const update = await withUpdate();
      const approval = await update.approve();
      mkdirSync(join(update.dataDir, "locks"), { recursive: true });
      const lock = join(update.dataDir, "locks", `${update.row().key}.lock`);
      writeFileSync(lock, String(process.pid));
      expect(await update.service.apply(update.identity, approval, yes)).toMatchObject({ ok: false, message: expect.stringContaining("다른 갱신") });
      expect(tree(update.installPath)).toEqual(LOCAL_V1);
      // 죽은 프로세스의 잠금은 치우고 진행한다.
      writeFileSync(lock, "2147483646");
      expect(await update.service.apply(update.identity, approval, yes)).toEqual({ ok: true });
      expect(existsSync(lock)).toBe(false);
    });

    it.skipIf(process.platform === "win32")("refuses to replace an install that holds a link or was swapped for a link", async () => {
      const update = await withUpdate();
      const approval = await update.approve();
      symlinkSync(update.root, join(update.installPath, "outside"));
      expect(update.row().scanError).toMatch(/심볼릭 링크/);
      expect((await update.service.apply(update.identity, approval, yes)).ok).toBe(false);
      unlinkSync(join(update.installPath, "outside"));

      const moved = join(update.root, "moved-caveman");
      const swapDuringConfirm = async () => {
        renameSync(update.installPath, moved);
        symlinkSync(moved, update.installPath);
        return true;
      };
      expect((await update.service.apply(update.identity, approval, swapDuringConfirm)).ok).toBe(false);
      expect(tree(moved)).toEqual(LOCAL_V1);
    });

    describe("interrupted replace", () => {
      /** 교체를 준비한 뒤에 `steps` 만큼만 진행하고 앱이 꺼진 상태를 만든다. */
      async function interrupted(steps: "prepared" | "oldMoved" | "newMoved" | "ledgerWritten" | "committed") {
        const update = await withUpdate();
        const approval = await update.approve();
        const key = skillKey(update.identity);
        const store = new SkillStore(update.dataDir);
        const previous = store.readLedger()[key]!;
        const files = await fetchBundle(update.github.fetcher, "sample/skills", "skills/caveman", approval.commit);
        const journal = store.prepareReplace({ key, installPath: update.installPath, files, expectedLocalHash: approval.localHash, at: "2026-11-20T00:00:01.000Z" }, previous);
        const order = ["prepared", "oldMoved", "newMoved", "ledgerWritten", "committed"];
        const reached = (step: string) => order.indexOf(steps) >= order.indexOf(step);
        if (reached("oldMoved")) renameSync(journal.installPath, journal.backupPath);
        if (reached("newMoved")) renameSync(journal.stagePath, journal.installPath);
        if (reached("ledgerWritten")) store.writeEntry(key, { ...previous, installedCommit: approval.commit, installedManifestHash: approval.targetHash });
        if (reached("committed")) writeFileSync(join(update.dataDir, "journal", `${key}.json`), JSON.stringify({ ...journal, phase: "committed" }));
        // 다시 켠 앱이다.
        const restarted = new SkillService({ dataDir: update.dataDir, home: update.home, fetcher: update.github.fetcher });
        return { ...update, restarted, journal };
      }

      it.each(["prepared", "oldMoved", "newMoved", "ledgerWritten"] as const)("restores the previous files and ledger after a stop at %s", async (step) => {
        const state = await interrupted(step);
        if (step === "oldMoved") expect(existsSync(state.installPath)).toBe(false);
        expect(state.restarted.recover()).toEqual([]);
        expect(tree(state.installPath)).toEqual(LOCAL_V1);
        const row = state.restarted.list({ cwd: null, codex: [] }).find((item) => item.identity.installPath === state.installPath)!;
        expect(row.entry?.installedCommit).toBe(state.v1);
        expect(row.state.status).toBe("updateAvailable");
        expect(row.hasSnapshot).toBe(false);
        expect(state.leftovers()).toEqual([]);
        expect(readdirSync(join(state.dataDir, "journal"))).toEqual([]);
      });

      it("finishes the cleanup after a stop behind the commit mark, and rollback then restores the previous state", async () => {
        const state = await interrupted("committed");
        expect(state.restarted.recover()).toEqual([]);
        expect(tree(state.installPath)).toEqual(TARGET_V2);
        expect(state.leftovers()).toEqual([]);
        const row = () => state.restarted.list({ cwd: null, codex: [] }).find((item) => item.identity.installPath === state.installPath)!;
        expect(row().entry?.installedCommit).toBe(state.v2);
        expect(await state.restarted.rollback(state.identity, row().localHash!, yes)).toEqual({ ok: true });
        expect(tree(state.installPath)).toEqual(LOCAL_V1);
        expect(row().entry?.installedCommit).toBe(state.v1);
      });

      it("does not delete an install folder that is not the bundle of the journal", async () => {
        const state = await interrupted("newMoved");
        writeFileSync(join(state.installPath, "notes.md"), "someone else wrote here\n");
        expect(state.restarted.recover()).toHaveLength(1);
        expect(readFileSync(join(state.installPath, "notes.md"), "utf8")).toBe("someone else wrote here\n");
        expect(tree(state.journal.backupPath)).toEqual(LOCAL_V1);
      });

      it("does not act on a journal whose paths leave the skill folder", async () => {
        const state = await interrupted("oldMoved");
        const victim = join(state.root, "victim");
        mkdirSync(victim);
        writeFileSync(join(victim, "keep.txt"), "keep");
        const path = join(state.dataDir, "journal", `${state.journal.key}.json`);
        writeFileSync(path, JSON.stringify({ ...state.journal, stagePath: victim }));
        expect(state.restarted.recover()).toHaveLength(1);
        expect(readFileSync(join(victim, "keep.txt"), "utf8")).toBe("keep");
        expect(existsSync(state.journal.backupPath)).toBe(true);
      });
    });
  });
});
