// Electron 수명 이벤트만 합성한다. native 승인·키 저장소·모델은 실행하지 않는다.
import { EventEmitter } from "node:events";
import type { BrowserWindow } from "electron";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { VAULT_RESTORE_CHANNELS, type VaultRestoreTarget, type VaultRestoreHold } from "../src/shared/vaultRestore";
import { vaultRestorePowerHold } from "../src/main/vaultRestorePower";
import type { VaultRestoreChannel } from "../src/main/vaultRestoreSession";

let nextId = 1;
class FakeContents extends EventEmitter {
  readonly id = nextId++;
  readonly mainFrame = {};
  destroyed = false;
  send = vi.fn();
  setWindowOpenHandler = vi.fn();
  isDestroyed(): boolean { return this.destroyed; }
}
class FakeWindow extends EventEmitter {
  readonly webContents = new FakeContents();
  destroyed = false;
  focus = vi.fn();
  restore = vi.fn();
  show = vi.fn();
  setContentProtection = vi.fn();
  constructor(_options?: unknown) { super(); allWindows.push(this); }
  isDestroyed(): boolean { return this.destroyed; }
  isMinimized(): boolean { return false; }
  close(): void {
    if (this.destroyed) return;
    this.emit("close");
    this.destroyed = true;
    this.webContents.destroyed = true;
    this.webContents.emit("destroyed");
    this.emit("closed");
  }
}
const allWindows: FakeWindow[] = [];
const listeners = new Map<string, (event: unknown, ...args: unknown[]) => void>();
vi.doMock("electron", () => ({
  BrowserWindow: FakeWindow,
  ipcMain: { on: (channel: string, listener: (event: unknown, ...args: unknown[]) => void) => listeners.set(channel, listener) },
}));
const { VaultRestoreWindows } = await import("../src/main/vaultRestoreWindow");

const TARGET: VaultRestoreTarget = {
  workspace: "C:/fixture/workspace", issueUrl: "https://github.com/fixture/repository/issues/7",
  issue: { provider: "github", server_id: "github.com", repository_id: "R_fixture", issue_id: "I_fixture",
    project_scope: { workspace_dir: "C:/fixture/workspace" } },
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
function fixture(profileHold: () => VaultRestoreHold | null = () => null) {
  const created: ReturnType<typeof newChannel>[] = [];
  function newChannel() {
    const prepared = deferred<{ kind: "prepared" }>();
    const channel = {
      capabilities: { reconnect: true },
      prepare: vi.fn(() => prepared.promise),
      restore: vi.fn<VaultRestoreChannel["restore"]>(async () => ({ kind: "restored", title: "private-title", body: "private-body" })),
      review: vi.fn<VaultRestoreChannel["review"]>(async () => ({ kind: "held", reason: "source_unavailable" })),
      reconnect: vi.fn<VaultRestoreChannel["reconnect"]>(async () => ({ kind: "held", reason: "source_unavailable" })),
      close: vi.fn(),
    };
    return { channel, prepared };
  }
  const source = vi.fn(() => {
    const entry = newChannel();
    created.push(entry);
    return entry.channel;
  });
  const windows = new VaultRestoreWindows({
    source, profileHold, preloadPath: "fixture", hidden: true, loadPage: vi.fn(),
  });
  const opener = new FakeWindow();
  windows.open(opener as unknown as BrowserWindow, TARGET);
  const window = allWindows.at(-1)!;
  return { windows, source, created, opener, window };
}
function send(window: FakeWindow, channel: string, ...args: unknown[]) {
  listeners.get(channel)?.({ sender: window.webContents, senderFrame: window.webContents.mainFrame }, ...args);
}
async function microtasks(): Promise<void> { await Promise.resolve(); await Promise.resolve(); }

beforeEach(() => { allWindows.length = 0; listeners.clear(); });
afterEach(() => { for (const window of allWindows) window.close(); });

describe("dedicated Vault window lifetime", () => {
  it("does not prepare when the initial OS state is unknown, and does not auto-open when it becomes known", async () => {
    let state = "unknown";
    const f = fixture(() => vaultRestorePowerHold("win32", () => state));
    send(f.window, VAULT_RESTORE_CHANNELS.ready);
    expect(f.source).not.toHaveBeenCalled();
    expect(f.window.webContents.send).toHaveBeenLastCalledWith(
      VAULT_RESTORE_CHANNELS.state, { phase: "held", reason: "os_lock_unavailable" });
    state = "active";
    await microtasks();
    expect(f.source).not.toHaveBeenCalled();
    send(f.window, VAULT_RESTORE_CHANNELS.unlock);
    expect(f.source).toHaveBeenCalledTimes(1);
    state = "unknown";
    f.created[0]!.prepared.resolve({ kind: "prepared" });
    await microtasks();
    expect(f.created[0]!.channel.close).toHaveBeenCalledTimes(1);
    expect(f.created[0]!.channel.restore).not.toHaveBeenCalled();
    expect(f.window.webContents.send).toHaveBeenLastCalledWith(
      VAULT_RESTORE_CHANNELS.state, { phase: "held", reason: "os_lock_unavailable" });
  });

  it("does not start a helper on Linux even if its idle state looks active", async () => {
    const f = fixture(() => vaultRestorePowerHold("linux", () => "active"));
    send(f.window, VAULT_RESTORE_CHANNELS.ready);
    send(f.window, VAULT_RESTORE_CHANNELS.unlock);
    await microtasks();
    expect(f.source).not.toHaveBeenCalled();
    expect(f.window.webContents.send).toHaveBeenLastCalledWith(
      VAULT_RESTORE_CHANNELS.state, { phase: "held", reason: "os_lock_unsupported" });
  });

  it("accepts the first page load but ignores subframe and in-place navigation", async () => {
    const f = fixture();
    f.window.webContents.emit("did-start-navigation", {}, "file:///vault-restore.html", false, true);
    send(f.window, VAULT_RESTORE_CHANNELS.ready);
    f.window.webContents.emit("did-start-navigation", {}, "file:///vault-restore.html#part", true, true);
    f.window.webContents.emit("did-start-navigation", {}, "about:blank", false, false);
    expect(f.source).toHaveBeenCalledTimes(1);
    expect(f.created[0]!.channel.close).not.toHaveBeenCalled();
    f.created[0]!.prepared.resolve({ kind: "prepared" });
    await microtasks();
    expect(f.created[0]!.channel.restore).toHaveBeenCalledTimes(1);
  });

  it.each(["reload", "crash", "close", "destroyed"] as const)(
    "cancels pending confirmation immediately on %s without waiting for ready or closed", async (cause) => {
      const f = fixture();
      send(f.window, VAULT_RESTORE_CHANNELS.ready);
      const entry = f.created[0]!;
      if (cause === "reload") f.window.webContents.emit("did-start-navigation", {}, "file:///vault-restore.html", false, true);
      else if (cause === "crash") f.window.webContents.emit("render-process-gone", {}, { reason: "crashed" });
      else if (cause === "close") f.window.emit("close");
      else { f.window.webContents.destroyed = true; f.window.webContents.emit("destroyed"); }
      expect(entry.channel.close).toHaveBeenCalledTimes(1);
      entry.prepared.resolve({ kind: "prepared" });
      await microtasks();
      expect(entry.channel.restore).not.toHaveBeenCalled();
      expect(JSON.stringify(f.window.webContents.send.mock.calls)).not.toContain("private-body");
      send(f.window, VAULT_RESTORE_CHANNELS.unlock);
      await microtasks();
      expect(f.source).toHaveBeenCalledTimes(1);
    },
  );

  it("requires a new current-target open after reload and replaces the invalid renderer", async () => {
    const f = fixture();
    send(f.window, VAULT_RESTORE_CHANNELS.ready);
    f.window.webContents.emit("did-start-navigation", {}, "file:///vault-restore.html", false, true);
    send(f.window, VAULT_RESTORE_CHANNELS.ready);
    send(f.window, VAULT_RESTORE_CHANNELS.unlock);
    await microtasks();
    expect(f.source).toHaveBeenCalledTimes(1);
    f.windows.open(f.opener as unknown as BrowserWindow, TARGET);
    const next = allWindows.at(-1)!;
    expect(next).not.toBe(f.window);
    expect(f.window.isDestroyed()).toBe(true);
    send(next, VAULT_RESTORE_CHANNELS.ready);
    expect(f.source).toHaveBeenCalledTimes(2);
  });

  it("deactivates an opener's target on context replacement until a new open", async () => {
    const f = fixture();
    send(f.window, VAULT_RESTORE_CHANNELS.ready);
    f.windows.lockOpenedBy(f.opener.webContents.id, "renderer_changed");
    expect(f.created[0]!.channel.close).toHaveBeenCalledTimes(1);
    send(f.window, VAULT_RESTORE_CHANNELS.unlock);
    await microtasks();
    expect(f.source).toHaveBeenCalledTimes(1);
    f.windows.open(f.opener as unknown as BrowserWindow, TARGET);
    expect(allWindows.at(-1)).toBe(f.window);
    send(f.window, VAULT_RESTORE_CHANNELS.unlock);
    expect(f.source).toHaveBeenCalledTimes(2);
  });

  it("cancels as soon as the opener starts closing", () => {
    const f = fixture();
    send(f.window, VAULT_RESTORE_CHANNELS.ready);
    f.opener.emit("close");
    expect(f.created[0]!.channel.close).toHaveBeenCalledTimes(1);
    expect(f.window.isDestroyed()).toBe(false);
    send(f.window, VAULT_RESTORE_CHANNELS.unlock);
    expect(f.source).toHaveBeenCalledTimes(1);
  });

  it("fences all windows and new opens until both OS power holds are released", async () => {
    const f = fixture();
    send(f.window, VAULT_RESTORE_CHANNELS.ready);
    const first = f.created[0]!;
    const otherOpener = new FakeWindow();
    f.windows.open(otherOpener as unknown as BrowserWindow, TARGET);
    const other = allWindows.at(-1)!;
    send(other, VAULT_RESTORE_CHANNELS.ready);
    const second = f.created[1]!;
    const restoring = deferred<{ kind: "restored"; title: string; body: string }>();
    second.channel.restore.mockImplementation(() => restoring.promise);
    second.prepared.resolve({ kind: "prepared" });
    await microtasks();
    expect(second.channel.restore).toHaveBeenCalledTimes(1);

    f.windows.lockAll("os_lock");
    expect(first.channel.close).toHaveBeenCalledTimes(1);
    expect(second.channel.close).toHaveBeenCalledTimes(1);
    first.prepared.resolve({ kind: "prepared" });
    restoring.resolve({ kind: "restored", title: "late title", body: "late-private-canary" });
    await microtasks();
    expect(first.channel.restore).not.toHaveBeenCalled();
    expect(JSON.stringify(other.webContents.send.mock.calls)).not.toContain("late-private-canary");

    // Queued unlocks and new windows cannot start another helper while locked.
    send(f.window, VAULT_RESTORE_CHANNELS.unlock);
    f.windows.open(f.opener as unknown as BrowserWindow, {
      ...TARGET, issue: { ...TARGET.issue, issue_id: "I_next" },
    });
    const next = allWindows.at(-1)!;
    send(next, VAULT_RESTORE_CHANNELS.ready);
    expect(f.source).toHaveBeenCalledTimes(2);
    expect(next.webContents.send).toHaveBeenLastCalledWith(
      VAULT_RESTORE_CHANNELS.state, { phase: "held", reason: "os_locked" },
    );

    f.windows.lockAll("suspend");
    f.windows.releasePowerHold("os_lock");
    send(next, VAULT_RESTORE_CHANNELS.unlock);
    expect(next.webContents.send).toHaveBeenLastCalledWith(
      VAULT_RESTORE_CHANNELS.state, { phase: "held", reason: "system_suspended" },
    );
    expect(f.source).toHaveBeenCalledTimes(2);
    f.windows.releasePowerHold("suspend");
    expect(f.source).toHaveBeenCalledTimes(2); // no automatic confirmation
    send(next, VAULT_RESTORE_CHANNELS.unlock);
    expect(f.source).toHaveBeenCalledTimes(3);
    expect(f.created[2]!.channel.prepare).toHaveBeenCalledTimes(1);
  });

  it("ignores the general window and nested frames on the private channels", () => {
    const f = fixture();
    send(f.opener, VAULT_RESTORE_CHANNELS.ready);
    listeners.get(VAULT_RESTORE_CHANNELS.ready)?.({ sender: f.window.webContents, senderFrame: {} });
    expect(f.source).not.toHaveBeenCalled();
    send(f.window, VAULT_RESTORE_CHANNELS.ready);
    expect(f.source).toHaveBeenCalledTimes(1);
  });
});
