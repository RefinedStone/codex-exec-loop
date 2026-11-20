// 로컬 복원본 창의 세션과 실행 프로필 규칙을 본다 (#2427). 가짜 복원 경로만 쓴다. 실제 키 저장소와 GitHub 는 없다.
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { afterEach, describe, expect, it, vi } from "vitest";

import { VaultRestoreSession, type VaultRestoreChannel, type VaultRestoreSessionDeps } from "../src/main/vaultRestoreSession";
import { IPC_CHANNELS } from "../src/shared/bridge";
import {
  isVaultRestoreIssueUrl,
  type VaultReconnectReview,
  type VaultRestoreHold,
  type VaultRestoreViewState,
  VAULT_RESTORE_CHANNELS,
  vaultRestoreProfileHold,
  vaultRestoreView,
} from "../src/shared/vaultRestore";

const ISSUE = "https://github.com/fixture-owner/fixture-repo/issues/7";
const RESTORED = { kind: "restored" as const, title: "fixture title", body: "token fixture-secret-value" };
const REVIEW: VaultReconnectReview = {
  reviewedHash: "hash-2",
  publishedTitle: "fixture title",
  publishedBody: "edited [[akra:mask:1]] and [[akra:mask:1]]",
  markers: [
    {
      marker: "[[akra:mask:1]]",
      previous: { field: "body", position: 6 },
      candidates: [
        { field: "body", position: 7, context: "edited [[akra:mask:1]]" },
        { field: "body", position: 27, context: "and [[akra:mask:1]]" },
      ],
    },
  ],
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => (resolve = done));
  return { promise, resolve };
}

function fakeChannel(overrides: Partial<VaultRestoreChannel> = {}) {
  return {
    restore: vi.fn(() => Promise.resolve<Awaited<ReturnType<VaultRestoreChannel["restore"]>>>(RESTORED)),
    review: vi.fn(() => Promise.resolve<Awaited<ReturnType<VaultRestoreChannel["review"]>>>({ kind: "review", review: REVIEW })),
    reconnect: vi.fn(() => Promise.resolve<Awaited<ReturnType<VaultRestoreChannel["reconnect"]>>>({ kind: "reconnected" })),
    close: vi.fn(),
    ...overrides,
  };
}

function setup(overrides: Partial<VaultRestoreSessionDeps> = {}, channel = fakeChannel()) {
  const states: VaultRestoreViewState[] = [];
  const source = vi.fn((): VaultRestoreChannel | null => channel);
  const confirm = vi.fn(() => Promise.resolve(true));
  const session = new VaultRestoreSession({
    issueUrl: ISSUE,
    source,
    confirm,
    profileHold: () => null,
    emit: (state) => states.push(state),
    sessionMs: 1_000,
    now: () => 0,
    ...overrides,
  });
  return { session, states, source, confirm, channel, last: () => states.at(-1) };
}

afterEach(() => vi.useRealTimers());

describe("VaultRestoreSession", () => {
  it("shows the restored copy only after the profile check and the user confirmation", async () => {
    const { session, states, source, confirm } = setup();
    await session.unlock();
    expect(states).toEqual([{ phase: "opening" }, { phase: "restored", title: RESTORED.title, body: RESTORED.body, expiresAt: 1_000 }]);
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(source).toHaveBeenCalledWith(ISSUE);
  });

  it("holds without a confirmation or a source call when the execution profile cannot enforce the boundary", async () => {
    const { session, states, source, confirm } = setup({ profileHold: () => "profile_automation" });
    await session.unlock();
    expect(states).toEqual([{ phase: "held", reason: "profile_automation" }]);
    expect(confirm).not.toHaveBeenCalled();
    expect(source).not.toHaveBeenCalled();
  });

  it("holds when the profile changes while the confirmation is open", async () => {
    let hold: VaultRestoreHold | null = null;
    const { session, last, source } = setup({
      profileHold: () => hold,
      confirm: () => {
        hold = "agent_working";
        return Promise.resolve(true);
      },
    });
    await session.unlock();
    expect(last()).toEqual({ phase: "held", reason: "agent_working" });
    expect(source).not.toHaveBeenCalled();
  });

  it("does not open the source when the user cancels", async () => {
    const { session, last, source } = setup({ confirm: () => Promise.resolve(false) });
    await session.unlock();
    expect(last()).toEqual({ phase: "held", reason: "confirmation_cancelled" });
    expect(source).not.toHaveBeenCalled();
  });

  it("holds when no local restore source is connected", async () => {
    const { session, last } = setup({ source: () => null });
    await session.unlock();
    expect(last()).toEqual({ phase: "held", reason: "source_unavailable" });
  });

  it("drops a late confirmation after a lock", async () => {
    const answer = deferred<boolean>();
    const { session, states, source } = setup({ confirm: () => answer.promise });
    const pending = session.unlock();
    session.lock("os_lock");
    answer.resolve(true);
    await pending;
    expect(states).toEqual([{ phase: "opening" }, { phase: "locked", cause: "os_lock" }]);
    expect(source).not.toHaveBeenCalled();
  });

  it("drops a late restore reply after a lock and closes the channel", async () => {
    const reply = deferred<typeof RESTORED>();
    const channel = fakeChannel({ restore: vi.fn(() => reply.promise) });
    const { session, states } = setup({}, channel);
    const pending = session.unlock();
    await vi.waitFor(() => expect(channel.restore).toHaveBeenCalled());
    session.lock("window_closed");
    reply.resolve(RESTORED);
    await pending;
    expect(states).toEqual([{ phase: "opening" }, { phase: "locked", cause: "window_closed" }]);
    expect(channel.close).toHaveBeenCalledTimes(1);
    expect(JSON.stringify(states)).not.toContain("fixture-secret-value");
  });

  it("locks again when the viewing session expires", async () => {
    vi.useFakeTimers();
    const { session, last, channel } = setup();
    await session.unlock();
    expect(last()?.phase).toBe("restored");
    vi.advanceTimersByTime(1_000);
    expect(last()).toEqual({ phase: "locked", cause: "session_expired" });
    expect(channel.close).toHaveBeenCalledTimes(1);
  });

  it("asks for a new confirmation after every lock", async () => {
    const { session, confirm, source } = setup();
    await session.unlock();
    session.lock("project_switch");
    await session.unlock();
    expect(confirm).toHaveBeenCalledTimes(2);
    expect(source).toHaveBeenCalledTimes(2);
  });

  it("keeps the session for a reconnect review when the published copy changed", async () => {
    const restore = vi
      .fn<VaultRestoreChannel["restore"]>()
      .mockResolvedValueOnce({ kind: "held", reason: "marker_duplicated" })
      .mockResolvedValueOnce(RESTORED);
    const channel = fakeChannel({ restore });
    const { session, last } = setup({}, channel);
    await session.unlock();
    expect(last()).toEqual({ phase: "held", reason: "marker_duplicated" });
    expect(channel.close).not.toHaveBeenCalled();
    await session.requestReview();
    expect(last()).toEqual({ phase: "review", review: REVIEW });
    await session.reconnect([{ marker: "[[akra:mask:1]]", field: "body", position: 27 }]);
    expect(channel.reconnect).toHaveBeenCalledWith("hash-2", [{ marker: "[[akra:mask:1]]", field: "body", position: 27 }]);
    expect(last()?.phase).toBe("restored");
  });

  it("ends the session on a hold that a reconnect cannot fix", async () => {
    const channel = fakeChannel({ restore: vi.fn(() => Promise.resolve({ kind: "held" as const, reason: "not_bound" as const })) });
    const { session, last } = setup({}, channel);
    await session.unlock();
    expect(last()).toEqual({ phase: "held", reason: "not_bound" });
    expect(channel.close).toHaveBeenCalledTimes(1);
    await session.requestReview();
    expect(channel.review).not.toHaveBeenCalled();
  });

  it.each([
    ["an unknown marker", [{ marker: "[[akra:mask:9]]", field: "body", position: 7 }]],
    ["a place that the review did not offer", [{ marker: "[[akra:mask:1]]", field: "title", position: 0 }]],
    ["one marker twice", [{ marker: "[[akra:mask:1]]", field: "body", position: 7 }, { marker: "[[akra:mask:1]]", field: "body", position: 27 }]],
    ["a value that is not a list", "all"],
  ])("rejects an approval with %s", async (_name, mappings) => {
    const channel = fakeChannel({ restore: vi.fn(() => Promise.resolve({ kind: "held" as const, reason: "published_copy_changed" as const })) });
    const { session, last } = setup({}, channel);
    await session.unlock();
    await session.requestReview();
    await session.reconnect(mappings);
    expect(last()).toEqual({ phase: "held", reason: "mapping_rejected" });
    expect(channel.reconnect).not.toHaveBeenCalled();
    expect(channel.close).toHaveBeenCalledTimes(1);
  });

  it("drops a late review reply after a lock", async () => {
    const reply = deferred<{ kind: "review"; review: VaultReconnectReview }>();
    const restore = vi.fn(() => Promise.resolve({ kind: "held" as const, reason: "marker_moved" as const }));
    const { session, last } = setup({}, fakeChannel({ restore, review: vi.fn(() => reply.promise) }));
    await session.unlock();
    const pending = session.requestReview();
    session.lock("suspend");
    reply.resolve({ kind: "review", review: REVIEW });
    await pending;
    expect(last()).toEqual({ phase: "locked", cause: "suspend" });
  });
});

describe("vault restore contract", () => {
  it("holds for each profile fact that removes the trusted boundary", () => {
    const enforceable = { automation: false, devRenderer: false, hiddenWindows: false, agentWorking: false };
    expect(vaultRestoreProfileHold(enforceable)).toBeNull();
    expect(vaultRestoreProfileHold({ ...enforceable, automation: true })).toBe("profile_automation");
    expect(vaultRestoreProfileHold({ ...enforceable, devRenderer: true })).toBe("profile_dev_renderer");
    expect(vaultRestoreProfileHold({ ...enforceable, hiddenWindows: true })).toBe("profile_hidden_window");
    expect(vaultRestoreProfileHold({ ...enforceable, agentWorking: true })).toBe("agent_working");
  });

  it("offers a reconnect review only when a change of the published copy is confirmed", () => {
    expect(vaultRestoreView({ phase: "held", reason: "marker_missing" }).actions).toEqual(["review", "lock"]);
    expect(vaultRestoreView({ phase: "held", reason: "latest_read_failed" }).actions).toEqual(["unlock"]);
    expect(vaultRestoreView({ phase: "locked", cause: "os_lock" }).actions).toEqual(["unlock"]);
  });

  it("accepts only a github.com issue address", () => {
    expect(isVaultRestoreIssueUrl(ISSUE)).toBe(true);
    expect(isVaultRestoreIssueUrl("https://github.com/fixture-owner/fixture-repo/pull/7")).toBe(false);
    expect(isVaultRestoreIssueUrl("https://example.test/fixture-owner/fixture-repo/issues/7")).toBe(false);
    expect(isVaultRestoreIssueUrl(`${ISSUE}?x=1`)).toBe(false);
  });

  it("keeps the restore channels out of the general message bus and the general preload", () => {
    const general = new Set<string>(Object.values(IPC_CHANNELS));
    for (const channel of Object.values(VAULT_RESTORE_CHANNELS)) expect(general.has(channel)).toBe(false);
    const source = (path: string) => readFileSync(join(import.meta.dirname, "..", "src", path), "utf8");
    expect(source("preload/index.ts")).not.toContain("VAULT_RESTORE_CHANNELS");
    expect(source("preload/vaultRestore.ts")).not.toContain("IPC_CHANNELS.");
    // 복원본 창은 브라우저 패널(자동화와 캡처의 대상)과 Gateway 접속을 쓰지 않는다.
    for (const file of ["main/vaultRestoreWindow.ts", "main/vaultRestoreSession.ts"]) {
      expect(source(file)).not.toMatch(/from ".*(browserPanel|browserAgent|gatewayConnection|shared\/bridge)/);
    }
    expect(source("renderer/vaultRestore/main.ts")).not.toMatch(/window\.akra\b|innerHTML/);
  });
});
