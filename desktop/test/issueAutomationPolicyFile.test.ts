import { lstat, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, posix, win32 } from "node:path";
import { describe, expect, it, vi } from "vitest";
import {
  openIssueAutomationPolicyFile,
  type IssueAutomationPolicyFileAccess,
  type IssueAutomationPolicyPathMetadata,
} from "../src/main/issueAutomationPolicyFile";
import type { IssueAutomationPolicyFileRequest } from "../src/shared/bridge";

type EntryKind = "directory" | "file" | "symlink" | "junction" | "other" | "denied";

function metadata(kind: EntryKind): IssueAutomationPolicyPathMetadata {
  return {
    isDirectory: () => kind === "directory",
    isFile: () => kind === "file",
    isSymbolicLink: () => kind === "symlink" || kind === "junction",
  };
}

function fixture(options: {
  platform?: NodeJS.Platform;
  environment?: Record<string, string | undefined>;
  editor?: string;
  editorFails?: boolean;
} = {}) {
  const platform = options.platform ?? "linux";
  const paths = platform === "win32" ? win32 : posix;
  const workspace = platform === "win32" ? "C:\\work\\repo" : "/work/repo";
  const parent = paths.join(workspace, ".akra");
  const target = paths.join(parent, "issue-automation.toml");
  const entries = new Map<string, EntryKind>([[workspace, "directory"], [parent, "directory"], [target, "file"]]);
  const access = {
    platform,
    environment: options.environment ?? { HOME: "/users/operator" },
    lstat: vi.fn(async (path: string) => {
      const kind = entries.get(path);
      if (kind === "denied") throw Object.assign(new Error("PRIVATE_OS_ERROR"), { code: "EACCES" });
      if (kind === undefined) throw Object.assign(new Error("missing fixture entry"), { code: "ENOENT" });
      return metadata(kind);
    }),
    getApplicationNameForProtocol: vi.fn((_protocol: string) => options.editor ?? "Visual Studio Code"),
    openExternal: vi.fn(async (_url: string) => {
      if (options.editorFails) throw new Error("editor failed");
    }),
    showItemInFolder: vi.fn((_path: string) => undefined),
  } satisfies IssueAutomationPolicyFileAccess;
  const request: IssueAutomationPolicyFileRequest = {
    workspace, scope: "project", expectedPath: target, host: { kind: "local" },
  };
  return { access, entries, paths, workspace, parent, target, request };
}

describe("issue automation policy file opening", () => {
  it("opens only the independently calculated project file without reading its contents", async () => {
    const f = fixture();
    await openIssueAutomationPolicyFile(f.request, f.access);
    expect(f.access.lstat.mock.calls.map(([path]) => path)).toEqual([f.workspace, f.parent, f.target]);
    expect(f.access.getApplicationNameForProtocol).toHaveBeenCalledWith("vscode://");
    expect(f.access.openExternal).toHaveBeenCalledWith("vscode://file//work/repo/.akra/issue-automation.toml");
    expect(f.access.showItemInFolder).not.toHaveBeenCalled();
  });

  it.each([
    { platform: "linux" as const, environment: { AKRA_HOME: "/private/akra", HOME: "/ignored" }, target: "/private/akra/issue-automation.toml" },
    { platform: "linux" as const, environment: { USERPROFILE: "/ignored", HOME: "/users/operator" }, target: "/users/operator/.akra/issue-automation.toml" },
    { platform: "win32" as const, environment: { AKRA_HOME: "E:\\private-akra", USERPROFILE: "C:\\ignored", HOME: "D:\\ignored" }, target: "E:\\private-akra\\issue-automation.toml" },
    { platform: "win32" as const, environment: { USERPROFILE: "C:\\Users\\operator", HOME: "D:\\ignored" }, target: "C:\\Users\\operator\\.akra\\issue-automation.toml" },
  ])("matches Rust user-home selection: $target", async ({ platform, environment, target }) => {
    const f = fixture({ platform, environment });
    f.entries.set(f.paths.dirname(target), "directory");
    f.entries.set(target, "file");
    await openIssueAutomationPolicyFile({ ...f.request, scope: "user", expectedPath: target }, f.access);
    expect(f.access.lstat).toHaveBeenCalledWith(target);
    expect(f.access.openExternal).toHaveBeenCalledTimes(1);
    expect(f.access.showItemInFolder).not.toHaveBeenCalled();
  });

  it.each([
    { platform: "linux" as const, environment: {} },
    { platform: "linux" as const, environment: { AKRA_HOME: "", HOME: "/users/operator" } },
    { platform: "win32" as const, environment: { AKRA_HOME: "", USERPROFILE: "", HOME: "D:\\home" } },
    { platform: "win32" as const, environment: { HOME: "D:\\home" } },
    { platform: "linux" as const, environment: { AKRA_HOME: "relative", HOME: "/valid-home" } },
    { platform: "linux" as const, environment: { AKRA_HOME: "file:///private/akra", HOME: "/valid-home" } },
    { platform: "linux" as const, environment: { HOME: "relative" } },
    { platform: "linux" as const, environment: { HOME: "/users/./operator" } },
    { platform: "win32" as const, environment: { USERPROFILE: "C:/Users/./operator" } },
    { platform: "win32" as const, environment: { USERPROFILE: "relative", HOME: "D:\\valid-home" } },
    { platform: "win32" as const, environment: { AKRA_HOME: "C:relative", USERPROFILE: "C:\\Users\\operator" } },
  ])("does not replace an invalid or missing user-home choice with another home: %#", async ({ platform, environment }) => {
    const f = fixture({ platform, environment });
    await expect(openIssueAutomationPolicyFile({ ...f.request, scope: "user" }, f.access)).rejects.toThrow();
    expect(f.access.lstat).not.toHaveBeenCalled();
    expect(f.access.openExternal).not.toHaveBeenCalled();
    expect(f.access.showItemInFolder).not.toHaveBeenCalled();
  });

  it("does not need a user-home fallback to open a project policy", async () => {
    const f = fixture({ environment: {} });
    await openIssueAutomationPolicyFile(f.request, f.access);
    expect(f.access.openExternal).toHaveBeenCalledTimes(1);
  });

  it("rejects a remote host before inspecting any local path", async () => {
    const f = fixture();
    await expect(openIssueAutomationPolicyFile({ ...f.request, host: { kind: "remote", id: "remote-example" } }, f.access)).rejects.toThrow("원격 호스트");
    expect(f.access.lstat).not.toHaveBeenCalled();
    expect(f.access.openExternal).not.toHaveBeenCalled();
    expect(f.access.showItemInFolder).not.toHaveBeenCalled();
  });

  it.each([
    null,
    [],
    "file:///work/repo/.akra/issue-automation.toml",
    {},
    { scope: "user" },
    { workspace: "/work/repo", scope: "global", expectedPath: "/work/repo/.akra/issue-automation.toml", host: { kind: "local" } },
    { workspace: "/work/repo", scope: "project", expectedPath: "/work/repo/.akra/issue-automation.toml", host: { kind: "local", id: "remote" } },
    { workspace: "/work/repo", scope: "project", expectedPath: "/work/repo/.akra/issue-automation.toml", host: { kind: "local" }, command: "run" },
  ])("rejects malformed IPC input without invoking OS operations: %#", async (input) => {
    const f = fixture();
    await expect(openIssueAutomationPolicyFile(input, f.access)).rejects.toThrow();
    expect(f.access.lstat).not.toHaveBeenCalled();
    expect(f.access.openExternal).not.toHaveBeenCalled();
    expect(f.access.showItemInFolder).not.toHaveBeenCalled();
  });

  it.each([
    { workspace: "relative" },
    { workspace: "/work/repo\nother" },
    { expectedPath: "issue-automation.toml" },
    { expectedPath: "vscode://file/etc/passwd" },
    { expectedPath: "file:///etc/passwd" },
    { expectedPath: "javascript:alert(1)" },
    { expectedPath: "/work/repo/.akra/issue-automation.toml\u0000ignored" },
    { expectedPath: "/work/repo/.akra/issue-automation.toml\u0085" },
    { expectedPath: "/foreign/issue-automation.toml" },
    { expectedPath: "/work/repo/.akra/other.toml" },
  ])("rejects relative, controlled, URI and foreign paths before metadata access: %#", async (changed) => {
    const f = fixture();
    await expect(openIssueAutomationPolicyFile({ ...f.request, ...changed }, f.access)).rejects.toThrow();
    expect(f.access.lstat).not.toHaveBeenCalled();
    expect(f.access.openExternal).not.toHaveBeenCalled();
  });

  it.each(["C:relative", "\\rooted-without-drive", "\\\\.\\pipe\\policy", "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume1"] as const)(
    "rejects Windows drive-relative and device paths: %s",
    async (workspace) => {
      const f = fixture({ platform: "win32" });
      await expect(openIssueAutomationPolicyFile({ ...f.request, workspace }, f.access)).rejects.toThrow();
      expect(f.access.lstat).not.toHaveBeenCalled();
      expect(f.access.openExternal).not.toHaveBeenCalled();
    },
  );

  it("accepts standard Rust Windows extended paths without emitting a device URL", async () => {
    const f = fixture({ platform: "win32" });
    await openIssueAutomationPolicyFile({
      ...f.request,
      workspace: "\\\\?\\C:\\work\\repo",
      expectedPath: "\\\\?\\C:\\work\\repo\\.akra\\issue-automation.toml",
    }, f.access);
    expect(f.access.openExternal).toHaveBeenCalledWith("vscode://file/C:/work/repo/.akra/issue-automation.toml");
  });

  it("accepts a fully qualified Windows share with a dotted server name", async () => {
    const f = fixture({ platform: "win32" });
    const workspace = "\\\\server.example\\workspace\\repo";
    const parent = win32.join(workspace, ".akra");
    const target = win32.join(parent, "issue-automation.toml");
    f.entries.set(workspace, "directory");
    f.entries.set(parent, "directory");
    f.entries.set(target, "file");
    await openIssueAutomationPolicyFile({ ...f.request, workspace, expectedPath: target }, f.access);
    expect(f.access.openExternal).toHaveBeenCalledWith("vscode://file///server.example/workspace/repo/.akra/issue-automation.toml");
  });

  it.each([
    { part: "parent" as const, kind: "symlink" as const },
    { part: "parent" as const, kind: "junction" as const },
    { part: "parent" as const, kind: "file" as const },
    { part: "parent" as const, kind: "other" as const },
    { part: "leaf" as const, kind: "symlink" as const },
    { part: "leaf" as const, kind: "junction" as const },
    { part: "leaf" as const, kind: "directory" as const },
    { part: "leaf" as const, kind: "other" as const },
  ])("rejects $part $kind before launching an editor or reveal", async ({ part, kind }) => {
    const f = fixture();
    f.entries.set(part === "parent" ? f.parent : f.target, kind);
    await expect(openIssueAutomationPolicyFile(f.request, f.access)).rejects.toThrow();
    if (part === "parent") expect(f.access.lstat).not.toHaveBeenCalledWith(f.target);
    expect(f.access.openExternal).not.toHaveBeenCalled();
    expect(f.access.showItemInFolder).not.toHaveBeenCalled();
  });

  it.each(["missing", "symlink", "junction", "file", "other"] as const)("requires an actual workspace directory, not %s", async (kind) => {
    const f = fixture();
    if (kind === "missing") f.entries.delete(f.workspace);
    else f.entries.set(f.workspace, kind);
    await expect(openIssueAutomationPolicyFile(f.request, f.access)).rejects.toThrow("작업 폴더");
    expect(f.access.openExternal).not.toHaveBeenCalled();
    expect(f.access.showItemInFolder).not.toHaveBeenCalled();
  });

  it("does not mistake inaccessible metadata for a missing file or expose the OS error", async () => {
    const f = fixture();
    f.entries.set(f.parent, "denied");
    await expect(openIssueAutomationPolicyFile(f.request, f.access)).rejects.toThrow("정책 파일 위치를 확인할 수 없습니다.");
    expect(f.access.openExternal).not.toHaveBeenCalled();
    expect(f.access.showItemInFolder).not.toHaveBeenCalled();
  });

  it.each([{ editor: "" }, { editor: "Visual Studio Code", editorFails: true }])(
    "reveals an existing policy when the editor is absent or fails: %#",
    async (options) => {
      const f = fixture(options);
      await openIssueAutomationPolicyFile(f.request, f.access);
      expect(f.access.showItemInFolder).toHaveBeenCalledWith(f.target);
      expect(f.access.openExternal).toHaveBeenCalledTimes(options.editor ? 1 : 0);
    },
  );

  it("opens a missing policy in the editor without creating its missing parent or leaf", async () => {
    const f = fixture();
    f.entries.delete(f.target);
    f.entries.delete(f.parent);
    const before = [...f.entries.entries()];
    await openIssueAutomationPolicyFile(f.request, f.access);
    expect(f.access.openExternal).toHaveBeenCalledWith("vscode://file//work/repo/.akra/issue-automation.toml");
    expect(f.access.showItemInFolder).not.toHaveBeenCalled();
    expect([...f.entries.entries()]).toEqual(before);
  });

  it("reveals the nearest existing parent for a deeply missing user policy", async () => {
    const f = fixture({ environment: { AKRA_HOME: "/users/new/deep/akra" }, editor: "" });
    f.entries.set("/users", "directory");
    const before = [...f.entries.entries()];
    await openIssueAutomationPolicyFile({
      ...f.request, scope: "user", expectedPath: "/users/new/deep/akra/issue-automation.toml",
    }, f.access);
    expect(f.access.showItemInFolder).toHaveBeenCalledWith("/users");
    expect(f.access.openExternal).not.toHaveBeenCalled();
    expect([...f.entries.entries()]).toEqual(before);
  });

  it("encodes spaces, query and fragment characters and Unicode in the existing editor URL format", async () => {
    const f = fixture();
    const workspace = "/work/A ?# % 한글";
    const parent = posix.join(workspace, ".akra");
    const target = posix.join(parent, "issue-automation.toml");
    f.entries.set(workspace, "directory");
    f.entries.set(parent, "directory");
    f.entries.set(target, "file");
    await openIssueAutomationPolicyFile({ ...f.request, workspace, expectedPath: target }, f.access);
    expect(f.access.openExternal).toHaveBeenCalledWith("vscode://file//work/A%20%3F%23%20%25%20%ED%95%9C%EA%B8%80/.akra/issue-automation.toml");
  });

  it("keeps a missing policy absent with real filesystem metadata", async () => {
    const workspace = await mkdtemp(join(tmpdir(), "akra-policy-open-missing-"));
    const f = fixture({ platform: process.platform });
    const parent = join(workspace, ".akra");
    const target = join(parent, "issue-automation.toml");
    try {
      await openIssueAutomationPolicyFile({ ...f.request, workspace, expectedPath: target }, { ...f.access, lstat });
      await expect(lstat(parent)).rejects.toMatchObject({ code: "ENOENT" });
      await expect(lstat(target)).rejects.toMatchObject({ code: "ENOENT" });
      expect(f.access.openExternal).toHaveBeenCalledTimes(1);
      expect(f.access.showItemInFolder).not.toHaveBeenCalled();
    } finally {
      await rm(workspace, { recursive: true, force: true });
    }
  });

  it("rejects a real symlink or Windows junction at the policy parent", async () => {
    const root = await mkdtemp(join(tmpdir(), "akra-policy-open-link-"));
    const workspace = join(root, "repo");
    const outside = join(root, "outside");
    const f = fixture({ platform: process.platform });
    try {
      await mkdir(workspace);
      await mkdir(outside);
      try {
        await symlink(outside, join(workspace, ".akra"), process.platform === "win32" ? "junction" : "dir");
      } catch (error) {
        if (process.platform === "win32" && ["EPERM", "EACCES", "ENOTSUP"].includes((error as NodeJS.ErrnoException).code ?? "")) return;
        throw error;
      }
      await expect(openIssueAutomationPolicyFile({
        ...f.request, workspace, expectedPath: join(workspace, ".akra", "issue-automation.toml"),
      }, { ...f.access, lstat })).rejects.toThrow("부모");
      expect(f.access.openExternal).not.toHaveBeenCalled();
      expect(f.access.showItemInFolder).not.toHaveBeenCalled();
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
  it.each([
    { platform: "linux" as const, home: "/users/operator/link/../settings" },
    { platform: "linux" as const, home: "/users/operator/./settings" },
    { platform: "linux" as const, home: "/users/operator/back\\slash" },
    { platform: "win32" as const, home: "C:\\Users\\operator\\link\\..\\settings" },
    { platform: "win32" as const, home: "\\\\?\\C:\\Users\\operator\\settings." },
    { platform: "win32" as const, home: "\\\\?\\C:\\Users\\operator\\settings " },
    { platform: "win32" as const, home: "\\\\?\\UNC\\server\\share\\settings.\\nested" },
  ])("rejects policy paths whose normalization can change the filesystem identity: $home", async ({ platform, home }) => {
    const f = fixture({ platform, environment: { AKRA_HOME: home } });
    const expectedPath = home + (platform === "win32" ? "\\" : "/") + "issue-automation.toml";
    await expect(openIssueAutomationPolicyFile({ ...f.request, scope: "user", expectedPath }, f.access)).rejects.toThrow();
    // Even a normalized matching expectedPath must not make a different configured home acceptable.
    await expect(openIssueAutomationPolicyFile({
      ...f.request, scope: "user", expectedPath: f.paths.normalize(expectedPath),
    }, f.access)).rejects.toThrow();
    expect(f.access.lstat).not.toHaveBeenCalled();
    expect(f.access.openExternal).not.toHaveBeenCalled();
    expect(f.access.showItemInFolder).not.toHaveBeenCalled();
  });

  it("refuses dot traversal through a real link instead of opening a different policy", async () => {
    const root = await mkdtemp(join(tmpdir(), "akra-policy-open-identity-"));
    const workspace = join(root, "repo");
    const outside = join(root, "outside");
    const actual = join(outside, "settings", "issue-automation.toml");
    const normalized = join(workspace, "settings", "issue-automation.toml");
    try {
      await mkdir(join(workspace, "settings"), { recursive: true });
      await mkdir(join(outside, "home"), { recursive: true });
      await mkdir(join(outside, "settings"));
      await writeFile(actual, "observed-policy");
      await writeFile(normalized, "different-policy");
      try {
        await symlink(join(outside, "home"), join(workspace, "link"), process.platform === "win32" ? "junction" : "dir");
      } catch (error) {
        if (process.platform === "win32" && ["EPERM", "EACCES", "ENOTSUP"].includes((error as NodeJS.ErrnoException).code ?? "")) return;
        throw error;
      }
      const separator = process.platform === "win32" ? "\\" : "/";
      const home = workspace + separator + "link" + separator + ".." + separator + "settings";
      const expectedPath = home + separator + "issue-automation.toml";
      if (process.platform !== "win32") expect(await readFile(expectedPath, "utf8")).toBe("observed-policy");
      const f = fixture({ platform: process.platform, environment: { AKRA_HOME: home } });
      const inspect = vi.fn(lstat);
      await expect(openIssueAutomationPolicyFile({
        ...f.request, workspace, scope: "user", expectedPath,
      }, { ...f.access, lstat: inspect })).rejects.toThrow("경로");
      expect(inspect).not.toHaveBeenCalled();
      expect(f.access.openExternal).not.toHaveBeenCalled();
      expect(f.access.showItemInFolder).not.toHaveBeenCalled();
      expect(await readFile(actual, "utf8")).toBe("observed-policy");
      expect(await readFile(normalized, "utf8")).toBe("different-policy");
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

});
