// 로컬 복원본 창의 세션과 실행 프로필 규칙을 본다 (#2427). 가짜 복원 경로만 쓴다. 실제 키 저장소와 GitHub 는 없다.
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { afterEach, describe, expect, it, vi } from "vitest";

import { VaultRestoreSession, type VaultRestoreChannel, type VaultRestoreSessionDeps } from "../src/main/vaultRestoreSession";
import { IPC_CHANNELS } from "../src/shared/bridge";
import {
  isVaultRestoreIssueUrl,
  isVaultRestoreTarget,
  copyVaultRestoreTarget,
  VAULT_RESTORE_CHANNELS,
  type VaultReconnectReview,
  type VaultRestoreHold,
  type VaultRestoreViewState,
  type VaultRestoreTarget,
  vaultRestoreProfileHold,
  vaultRestoreView,
} from "../src/shared/vaultRestore";

const ISSUE = "https://github.com/fixture-owner/fixture-repo/issues/7";
const TARGET: VaultRestoreTarget = {
  workspace: "C:/fixture/workspace", issueUrl: ISSUE,
  issue: { provider: "github", server_id: "github.com", repository_id: "R_fixture", issue_id: "I_fixture", project_scope: { workspace_dir: "C:/fixture/workspace" } },
};
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
    capabilities: { reconnect: true },
    prepare: vi.fn(() => Promise.resolve<Awaited<ReturnType<VaultRestoreChannel["prepare"]>>>({ kind: "prepared" })),
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
  const session = new VaultRestoreSession({
    target: TARGET,
    source,
    profileHold: () => null,
    emit: (state) => states.push(state),
    sessionMs: 1_000,
    now: () => 0,
    ...overrides,
  });
  return { session, states, source, channel, last: () => states.at(-1) };
}

afterEach(() => vi.useRealTimers());

describe("VaultRestoreSession", () => {
  it("shows the restored copy only after the profile check and the user confirmation", async () => {
    const { session, states, source, channel } = setup();
    await session.unlock();
    expect(states).toEqual([{ phase: "opening" }, { phase: "restored", title: RESTORED.title, body: RESTORED.body, expiresAt: 1_000 }]);
    expect(channel.prepare).toHaveBeenCalledTimes(1);
    expect(source).toHaveBeenCalledWith(TARGET);
    expect(vi.mocked(channel.prepare).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(channel.restore).mock.invocationCallOrder[0]!);
  });

  it("holds without a confirmation or a source call when the execution profile cannot enforce the boundary", async () => {
    const { session, states, source, channel } = setup({ profileHold: () => "profile_automation" });
    await session.unlock();
    expect(states).toEqual([{ phase: "held", reason: "profile_automation" }]);
    expect(channel.prepare).not.toHaveBeenCalled();
    expect(source).not.toHaveBeenCalled();
  });

  it("holds when the profile changes while the confirmation is open", async () => {
    let hold: VaultRestoreHold | null = null;
    const channel = fakeChannel({ prepare: vi.fn<VaultRestoreChannel["prepare"]>(async () => {
      hold = "agent_working";
      return { kind: "prepared" };
    }) });
    const { session, last, source } = setup({ profileHold: () => hold }, channel);
    await session.unlock();
    expect(last()).toEqual({ phase: "held", reason: "agent_working" });
    expect(source).toHaveBeenCalledTimes(1);
    expect(channel.restore).not.toHaveBeenCalled();
    expect(channel.close).toHaveBeenCalledTimes(1);
  });

  it("does not restore when the user cancels native confirmation", async () => {
    const channel = fakeChannel({ prepare: vi.fn<VaultRestoreChannel["prepare"]>(async () => ({ kind: "held", reason: "confirmation_cancelled" })) });
    const { session, last } = setup({}, channel);
    await session.unlock();
    expect(last()).toEqual({ phase: "held", reason: "confirmation_cancelled" });
    expect(channel.restore).not.toHaveBeenCalled();
    expect(channel.close).toHaveBeenCalledTimes(1);
  });

  it("holds when no local restore source is connected", async () => {
    const { session, last } = setup({ source: () => null });
    await session.unlock();
    expect(last()).toEqual({ phase: "held", reason: "source_unavailable" });
  });

  it.each(["os_lock", "window_closed", "project_switch"] as const)("closes a pending native confirmation and drops late prepared after %s", async (cause) => {
    const answer = deferred<{ kind: "prepared" }>();
    const channel = fakeChannel({ prepare: vi.fn(() => answer.promise) });
    const { session, states } = setup({}, channel);
    const pending = session.unlock();
    session.lock(cause);
    expect(channel.close).toHaveBeenCalledTimes(1);
    answer.resolve({ kind: "prepared" });
    await pending;
    expect(states).toEqual([{ phase: "opening" }, { phase: "locked", cause }]);
    expect(channel.restore).not.toHaveBeenCalled();
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
    const { session, channel, source } = setup();
    await session.unlock();
    session.lock("project_switch");
    await session.unlock();
    expect(channel.prepare).toHaveBeenCalledTimes(2);
    expect(source).toHaveBeenCalledTimes(2);
  });

  it("keeps the session for a reconnect review when the published copy changed", async () => {
    const restore = vi
      .fn<VaultRestoreChannel["restore"]>()
      .mockResolvedValueOnce({ kind: "held", reason: "marker_duplicated", canReconnect: true })
      .mockResolvedValueOnce(RESTORED);
    const channel = fakeChannel({ restore });
    const { session, last } = setup({}, channel);
    await session.unlock();
    expect(last()).toEqual({ phase: "held", reason: "marker_duplicated", canReconnect: true });
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
    const channel = fakeChannel({ restore: vi.fn(() => Promise.resolve({ kind: "held" as const, reason: "published_copy_changed" as const, canReconnect: true })) });
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
    const restore = vi.fn(() => Promise.resolve({ kind: "held" as const, reason: "marker_moved" as const, canReconnect: true }));
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
    expect(vaultRestoreView({ phase: "held", reason: "marker_missing", canReconnect: true }).actions).toEqual(["review", "lock"]);
    expect(vaultRestoreView({ phase: "held", reason: "marker_missing" }).actions).toEqual(["unlock"]);
    expect(vaultRestoreView({ phase: "held", reason: "platform_unsupported" }).detail).toContain("현재 환경");
    expect(vaultRestoreView({ phase: "held", reason: "latest_read_failed" }).actions).toEqual(["unlock"]);
    expect(vaultRestoreView({ phase: "locked", cause: "os_lock" }).actions).toEqual(["unlock"]);
  });

  it("accepts only a github.com issue address", () => {
    expect(isVaultRestoreIssueUrl(ISSUE)).toBe(true);
    expect(isVaultRestoreIssueUrl("https://github.com/fixture-owner/fixture-repo/pull/7")).toBe(false);
    expect(isVaultRestoreIssueUrl("https://example.test/fixture-owner/fixture-repo/issues/7")).toBe(false);
    expect(isVaultRestoreIssueUrl(`${ISSUE}?x=1`)).toBe(false);
    for (const url of [
      "https://github.com/owner/repo/issues/0",
      "https://github.com/owner/repo/issues/01",
      "https://github.com/owner/repo/issues/9223372036854775808",
      "https://github.com/../repo/issues/1",
      "https://github.com/owner/./issues/1",
      "https://github.com/" + "x".repeat(101) + "/repo/issues/1",
    ]) expect(isVaultRestoreIssueUrl(url)).toBe(false);

  });

  it("keeps restore capabilities separate within the approved bridge contract", () => {
    const general = new Set<string>(Object.values(IPC_CHANNELS));
    for (const channel of Object.values(VAULT_RESTORE_CHANNELS)) expect(general.has(channel)).toBe(false);
    const source = (path: string) => readFileSync(join(import.meta.dirname, "..", "src", path), "utf8");
    expect(source("preload/index.ts")).not.toContain("VAULT_RESTORE_CHANNELS");
    expect(source("preload/vaultRestore.ts")).not.toContain("IPC_CHANNELS.");
    // 복원본 창은 브라우저 패널(자동화와 캡처의 대상)과 Gateway 접속을 쓰지 않는다.
    for (const file of ["main/vaultRestoreWindow.ts", "main/vaultRestoreSession.ts", "main/vaultRestoreSource.ts"]) {
      expect(source(file)).not.toMatch(/from ".*(browserPanel|browserAgent|gatewayConnection)/);
    }
    expect(source("renderer/vaultRestore/main.ts")).not.toMatch(/innerHTML/);
    expect(source("preload/vaultRestore.ts")).toContain('exposeInMainWorld("akra", surface)');
    expect(source("preload/vaultRestore.ts")).not.toContain("akraVaultRestore");
  });
});

describe("native restore session regressions", () => {
  it("does not offer reconnect when the source cannot reconnect", async () => {
    const channel = fakeChannel({
      capabilities: { reconnect: false },
      restore: vi.fn<VaultRestoreChannel["restore"]>(async () => ({ kind: "held", reason: "marker_moved" })),
    });
    const { session, last } = setup({}, channel);
    await session.unlock();
    expect(last()).toEqual({ phase: "held", reason: "marker_moved" });
    expect(vaultRestoreView(last()!).actions).toEqual(["unlock"]);
    expect(vaultRestoreView(last()!).detail).toContain("재연결을 지원하지 않습니다");
    expect(channel.close).toHaveBeenCalledTimes(1);
    await session.requestReview();
    expect(channel.review).not.toHaveBeenCalled();
  });

  it("drops plaintext when the profile changes during restore without an explicit lock", async () => {
    let hold: VaultRestoreHold | null = null;
    const channel = fakeChannel({ restore: vi.fn<VaultRestoreChannel["restore"]>(async () => {
      hold = "agent_working";
      return RESTORED;
    }) });
    const { session, states, last } = setup({ profileHold: () => hold }, channel);
    await session.unlock();
    expect(last()).toEqual({ phase: "held", reason: "agent_working" });
    expect(JSON.stringify(states)).not.toContain(RESTORED.body);
    expect(channel.close).toHaveBeenCalledTimes(1);
  });

  it("does not extend the viewing deadline while a restore reply is in flight", async () => {
    let now = 0;
    const channel = fakeChannel({ restore: vi.fn<VaultRestoreChannel["restore"]>(async () => {
      now = 900;
      return RESTORED;
    }) });
    const { session, last } = setup({ now: () => now }, channel);
    await session.unlock();
    expect(last()).toEqual({ phase: "restored", title: RESTORED.title, body: RESTORED.body, expiresAt: 1_000 });
    session.lock("user");
  });

  it("drops a restored reply that arrives after expiry even before the timer callback", async () => {
    let now = 0;
    const channel = fakeChannel({ restore: vi.fn<VaultRestoreChannel["restore"]>(async () => {
      now = 1_001;
      return RESTORED;
    }) });
    const { session, states, last } = setup({ now: () => now }, channel);
    await session.unlock();
    expect(last()).toEqual({ phase: "locked", cause: "session_expired" });
    expect(JSON.stringify(states)).not.toContain(RESTORED.body);
  });

  it("normalizes errors from starting or reading a helper", async () => {
    const secret = "fixture-private-error";
    for (const channel of [
      fakeChannel({ prepare: vi.fn<VaultRestoreChannel["prepare"]>(async () => { throw new Error(secret); }) }),
      fakeChannel({ restore: vi.fn<VaultRestoreChannel["restore"]>(async () => { throw new Error(secret); }) }),
    ]) {
      const { session, last, states } = setup({}, channel);
      await session.unlock();
      expect(last()).toEqual({ phase: "held", reason: "source_unavailable" });
      expect(JSON.stringify(states)).not.toContain(secret);
      expect(channel.close).toHaveBeenCalledTimes(1);
    }
    const { session, last } = setup({ source: () => { throw new Error(secret); } });
    await session.unlock();
    expect(last()).toEqual({ phase: "held", reason: "source_unavailable" });
  });
});

describe("bound restore target", () => {
  it("requires a complete supported identity in the same absolute workspace", () => {
    expect(isVaultRestoreTarget(TARGET)).toBe(true);
    for (const workspace of ["/work/project", "C:\\work\\project", "\\\\server\\share\\project"]) {
      const target = copyVaultRestoreTarget(TARGET);
      target.workspace = workspace;
      Object.assign(target.issue.project_scope, { workspace_dir: workspace });
      expect(isVaultRestoreTarget(target)).toBe(true);
    }
    for (const value of [
      ISSUE,
      { ...TARGET, approved: true },
      { ...TARGET, workspace: "relative/path" },
      { ...TARGET, workspace: "C:/another" },
      { ...TARGET, issueUrl: ISSUE + "?preview=true" },
      { ...TARGET, issue: { ...TARGET.issue, issue_id: "" } },
      { ...TARGET, issue: { ...TARGET.issue, provider: "jira" } },
      { ...TARGET, issue: { ...TARGET.issue, server_id: "example.test" } },
      { ...TARGET, issue: { ...TARGET.issue, repository_id: "R fixture" } },
      { ...TARGET, issue: { ...TARGET.issue, repository_id: "R_한글" } },
      { ...TARGET, issue: { ...TARGET.issue, issue_id: "I".repeat(201) } },
      { ...TARGET, issue: { ...TARGET.issue, approved: true } },
      { ...TARGET, issue: { ...TARGET.issue, project_scope: { ...TARGET.issue.project_scope, approved: true } } },
    ]) expect(isVaultRestoreTarget(value)).toBe(false);
  });

  it("takes an independent full-coordinate snapshot", () => {
    const value = copyVaultRestoreTarget(TARGET);
    const copy = copyVaultRestoreTarget(value);
    Object.assign(value.issue.project_scope, { workspace_dir: "C:/other" });
    Object.assign(value.issue, { issue_id: "I_other" });
    expect(copy).toEqual(TARGET);
  });
});


describe("single native reconnect session", () => {
  function changedChannel(overrides: Partial<VaultRestoreChannel> = {}) {
    return fakeChannel({
      restore: vi.fn<VaultRestoreChannel["restore"]>()
        .mockResolvedValueOnce({ kind: "held", reason: "marker_moved", canReconnect: true })
        .mockResolvedValue(RESTORED),
      ...overrides,
    });
  }
  it("requires explicit per-reply reconnect permission even from a capable channel", async () => {
    const channel = changedChannel({ restore: vi.fn<VaultRestoreChannel["restore"]>(async () => ({ kind: "held", reason: "marker_moved" })) });
    const { session, last } = setup({}, channel);
    await session.unlock();
    expect(last()).toEqual({ phase: "held", reason: "marker_moved" });
    await session.requestReview();
    expect(channel.review).not.toHaveBeenCalled();
    expect(channel.close).toHaveBeenCalledTimes(1);
  });

  it("runs only one review and one reconnect despite duplicate and out-of-order requests", async () => {
    const reviewing = deferred<{ kind: "review"; review: VaultReconnectReview }>();
    const reconnecting = deferred<{ kind: "reconnected" }>();
    const channel = changedChannel({
      review: vi.fn(() => reviewing.promise), reconnect: vi.fn(() => reconnecting.promise),
    });
    const { session, last } = setup({}, channel);
    await session.unlock();
    await session.reconnect([]);
    expect(channel.reconnect).not.toHaveBeenCalled();
    const reviewPending = session.requestReview();
    await session.requestReview();
    await session.reconnect([]);
    expect(channel.review).toHaveBeenCalledTimes(1);
    expect(last()).toEqual({ phase: "reviewing" });
    reviewing.resolve({ kind: "review", review: REVIEW });
    await reviewPending;
    const writePending = session.reconnect([]);
    await session.reconnect([]);
    await session.requestReview();
    expect(channel.reconnect).toHaveBeenCalledTimes(1);
    expect(channel.reconnect).toHaveBeenCalledWith(REVIEW.reviewedHash, []);
    expect(channel.review).toHaveBeenCalledTimes(1);
    expect(last()).toEqual({ phase: "reconnecting" });
    reconnecting.resolve({ kind: "reconnected" });
    await writePending;
    expect(last()?.phase).toBe("restored");
    await session.requestReview();
    await session.reconnect([]);
    expect(channel.reconnect).toHaveBeenCalledTimes(1);
  });

  it("does not reuse a final changed-copy hold even if a faulty channel offers it", async () => {
    const channel = changedChannel({ restore: vi.fn<VaultRestoreChannel["restore"]>(async () => ({ kind: "held", reason: "marker_missing", canReconnect: true })) });
    const { session, last } = setup({}, channel);
    await session.unlock();
    await session.requestReview();
    await session.reconnect([]);
    expect(last()).toEqual({ phase: "held", reason: "marker_missing" });
    await session.requestReview();
    expect(channel.review).toHaveBeenCalledTimes(1);
    expect(channel.close).toHaveBeenCalledTimes(1);
  });

  it.each(["os_lock", "renderer_changed", "project_switch", "session_expired"] as const)(
    "drops late reconnect success and does not read plaintext after %s", async (cause) => {
      const reconnecting = deferred<{ kind: "reconnected" }>();
      const channel = changedChannel({ reconnect: vi.fn(() => reconnecting.promise) });
      const { session, last } = setup({}, channel);
      await session.unlock();
      await session.requestReview();
      const pending = session.reconnect([]);
      session.lock(cause);
      reconnecting.resolve({ kind: "reconnected" });
      await pending;
      expect(last()).toEqual({ phase: "locked", cause });
      expect(channel.restore).toHaveBeenCalledTimes(1);
      expect(channel.close).toHaveBeenCalledTimes(1);
    },
  );

  it("does not extend the original deadline while waiting for rewrite confirmation", async () => {
    let now = 0;
    const reconnecting = deferred<{ kind: "reconnected" }>();
    const channel = changedChannel({ reconnect: vi.fn(() => reconnecting.promise) });
    const { session, last } = setup({ now: () => now }, channel);
    await session.unlock();
    await session.requestReview();
    const pending = session.reconnect([]);
    now = 1_001;
    reconnecting.resolve({ kind: "reconnected" });
    await pending;
    expect(last()).toEqual({ phase: "locked", cause: "session_expired" });
    expect(channel.restore).toHaveBeenCalledTimes(1);
  });

  it("keeps an uncertain write terminal and asks the user to reopen without an automatic retry", async () => {
    const channel = changedChannel({ reconnect: vi.fn<VaultRestoreChannel["reconnect"]>(async () => ({ kind: "held", reason: "write_outcome_unknown" })) });
    const { session, last } = setup({}, channel);
    await session.unlock();
    await session.requestReview();
    await session.reconnect([]);
    expect(last()).toEqual({ phase: "held", reason: "write_outcome_unknown" });
    expect(vaultRestoreView(last()!).actions).toEqual(["unlock"]);
    expect(vaultRestoreView(last()!).detail).toContain("완료됐을 수");
    await session.reconnect([]);
    await session.requestReview();
    expect(channel.reconnect).toHaveBeenCalledTimes(1);
    expect(channel.restore).toHaveBeenCalledTimes(1);
  });

  it("clears the session if the execution profile changes during rewrite confirmation", async () => {
    let hold: VaultRestoreHold | null = null;
    const channel = changedChannel({ reconnect: vi.fn<VaultRestoreChannel["reconnect"]>(async () => {
      hold = "agent_working";
      return { kind: "reconnected" };
    }) });
    const { session, last } = setup({ profileHold: () => hold }, channel);
    await session.unlock();
    await session.requestReview();
    await session.reconnect([]);
    expect(last()).toEqual({ phase: "held", reason: "agent_working" });
    expect(channel.restore).toHaveBeenCalledTimes(1);
  });
});
