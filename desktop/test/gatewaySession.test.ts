// main 의 GatewayConnection 과 renderer 세션을 가짜 Gateway 에 실제로 연결해서 시험한다. 턴은 쓰지 않는다.
import { afterEach, describe, expect, it } from "vitest";

import type { AkraBridge, ConnectionState } from "../src/shared/bridge";
import { GatewayConnection } from "../src/main/gatewayConnection";
import { EMPTY_KERNEL_VIEW } from "../src/renderer/kernel/kernelConnection";
import { DesktopSession, groupThreadsByCwd, startDesktopSession, isAgentThreadSource } from "../src/renderer/session/desktopSession";
import { type DesktopState, mergeWorkspaceFolders, remoteHostKey, type SessionStore } from "../src/renderer/session/desktopStore";
import type { Thread } from "../src/shared/protocol/generated/v2/Thread";
import {
  APPROVAL_TURN_RECORDING,
  BASIC_RECORDING,
  type FakeGateway,
  startFakeGateway,
  uniqueTestEndpoint,
} from "./fake-gateway/fakeGateway";

const cleanups: Array<() => Promise<void> | void> = [];

afterEach(async () => {
  while (cleanups.length > 0) {
    await cleanups.pop()?.();
  }
});

async function waitFor(condition: () => boolean, timeoutMs = 5_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!condition()) {
    if (Date.now() > deadline) {
      throw new Error("condition was not met in time");
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

async function gateway(endpoint = uniqueTestEndpoint()): Promise<FakeGateway> {
  const fake = await startFakeGateway(endpoint, BASIC_RECORDING);
  cleanups.push(() => fake.close());
  return fake;
}

/** main 의 접속과 renderer 의 메시지 버스를 프로세스 안에서 잇는다 (preload 대신). */
function connectBridge(endpoint: string, startDaemon?: () => void, host?: string) {
  const messageListeners = new Set<(text: string) => void>();
  const stateListeners = new Set<(state: ConnectionState) => void>();
  const states: ConnectionState[] = [];
  const connection = new GatewayConnection({
    endpoint,
    onMessage: (text) => messageListeners.forEach((listener) => listener(text)),
    onState: (state) => {
      states.push(state);
      stateListeners.forEach((listener) => listener(state));
    },
    ...(startDaemon ? { startDaemon } : {}),
    ...(host ? { host } : {}),
    reconnectInitialMs: 50,
    reconnectMaxMs: 200,
  });
  const bridge: AkraBridge = {
    send: (text) => void connection.send(text),
    onMessage: (listener) => {
      messageListeners.add(listener);
      return () => messageListeners.delete(listener);
    },
    onConnectionState: (listener) => {
      stateListeners.add(listener);
      return () => stateListeners.delete(listener);
    },
    ready: () => undefined,
    chooseFolder: async () => null,
    gitStatus: async () => ({ ok: false, message: "not used" }),
    gitFileDiff: async () => ({ kind: "binary" }),
    gitFileAction: async () => ({ ok: true as const }),
    gitOpenFile: async () => undefined,
    gitBranchSummary: async () => ({ ok: false as const, message: "" }),
    gitCommitDiff: async () => ({ text: "", truncated: false }),
    gitPush: async () => ({ ok: true as const, url: null }),
    gitCreatePullRequest: async () => ({ ok: true as const, url: null }),
    gitHunks: async () => ({ hunks: [], message: null }),
    gitHunkAction: async () => ({ ok: true as const }),
    terminal: {
      open: async () => ({ ok: false, message: "not used" }),
      write: () => undefined,
      resize: () => undefined,
      close: () => undefined,
      onData: () => () => undefined,
      onExit: () => () => undefined,
    },
    browser: {
      show: async () => {
        throw new Error("not used");
      },
      setBounds: () => undefined,
      hide: () => undefined,
      navigate: async () => false,
      command: () => undefined,
      onState: () => () => undefined,
      loadSettings: async () => ({ profiles: [], presets: [] }),
      saveSettings: async (settings) => settings,
    },
    github: {
      summary: async () => ({ login: "", myPullRequests: [], reviewRequests: [] }),
      reviewPrompt: async () => "",
      project: async () => ({ title: "", url: "", items: [], closedCount: 0, truncated: false }),
      issuePrompt: async () => "",
      repository: async () => null,
      openExternal: () => undefined,
    },
    jira: {
      status: async () => ({ connected: false }),
      connect: async () => ({ connected: false }),
      disconnect: async () => ({ connected: false }),
      summary: async () => ({ siteUrl: "", issues: [] }),
      issuePrompt: async () => "",
      openExternal: () => undefined,
    },
    notion: { openExternal: () => undefined },
    dots: { getEnabled: () => Promise.resolve(false), setEnabled: () => Promise.resolve(false), openWindow: () => undefined, takeDraft: () => Promise.resolve(null), reportSeen: () => undefined, onSeen: () => () => undefined, hasWindow: () => Promise.resolve(false), dock: () => undefined, onDock: () => () => undefined, isSendAllowed: () => Promise.resolve(false), allowSend: () => undefined },
    remote: { open: () => undefined, close: () => undefined, send: () => undefined, onMessage: () => () => undefined, onConnectionState: () => () => undefined },
  };
  cleanups.push(() => connection.close());
  return { connection, bridge, states };
}

/** 세션이 쓰는 저장소 인터페이스를 평범한 객체로 구현한다 (zustand 없이). */
function sessionStore() {
  const snapshot: Partial<DesktopState> = {
    activeThreadIds: [],
    openThreadId: null,
    transcript: null,
    pendingApprovals: [],
    folders: [],
  };
  const store: SessionStore & { snapshot: Partial<DesktopState> } = {
    snapshot,
    setConnection: (connection) => Object.assign(snapshot, { connection }),
    setSession: (session) => Object.assign(snapshot, session, { sessionError: null }),
    setSessionError: (sessionError) => Object.assign(snapshot, { sessionError }),
    setActiveThreads: (threadIds) => {
      snapshot.activeThreadIds = threadIds;
    },
    setThreadFailed: () => undefined,
    setThreadActive: (threadId, active) => {
      const others = (snapshot.activeThreadIds ?? []).filter((id) => id !== threadId);
      snapshot.activeThreadIds = active ? [...others, threadId] : others;
    },
    setOpenThread: (openThreadId) => Object.assign(snapshot, { openThreadId, transcript: null, transcriptError: null }),
    setTranscript: (transcript, transcriptError = null) => Object.assign(snapshot, { transcript, transcriptError }),
    updateTranscript: (update) => {
      if (snapshot.transcript) {
        snapshot.transcript = update(snapshot.transcript);
      }
    },
    addApproval: (approval) => {
      snapshot.pendingApprovals = [...(snapshot.pendingApprovals ?? []), approval];
    },
    removeApproval: (requestId) => {
      snapshot.pendingApprovals = (snapshot.pendingApprovals ?? []).filter(
        (approval) => String(approval.requestId) !== String(requestId),
      );
    },
    clearApprovals: () => {
      snapshot.pendingApprovals = [];
    },
    addThreadToFolder: (cwd, thread) => {
      snapshot.folders = [{ cwd, threads: [thread] }, ...(snapshot.folders ?? []).filter((folder) => folder.cwd !== cwd)];
    },
    mergeFolders: (folders) => {
      snapshot.folders = mergeWorkspaceFolders(snapshot.folders ?? [], folders);
    },
    getOpenThreadId: () => snapshot.openThreadId ?? null,
    getTranscript: () => snapshot.transcript ?? null,
    setKernel: (kernel) => Object.assign(snapshot, { kernel }),
    setDots: (dots) => Object.assign(snapshot, { dots }),
    applyDotLive: (live) => Object.assign(snapshot, { lastDotLive: live }),
    getKernel: () => snapshot.kernel ?? EMPTY_KERNEL_VIEW,
  };
  return store;
}

describe("desktop session over the fake gateway", () => {
  it("initializes and shows gateway info and workspace folders", async () => {
    const fake = await gateway();
    const { connection, bridge } = connectBridge(fake.endpoint);
    const store = sessionStore();
    cleanups.push(startDesktopSession(bridge, store, "test"));
    connection.start();

    await waitFor(() => (store.snapshot.folders?.length ?? 0) > 0);
    expect(store.snapshot.gateway?.codexVersion).toBe("codex-cli 0.159.0");
    expect(store.snapshot.versionCheck?.matches).toBe(true);
    expect(store.snapshot.folders?.map((folder) => folder.cwd)).toEqual(["/work/example-app", "/work/other-lib"]);
    expect(fake.receivedMethods.slice(0, 2)).toEqual(["initialize", "initialized"]);
  });

  it("reconnects and initializes again after the gateway restarts", async () => {
    const endpoint = uniqueTestEndpoint();
    const first = await startFakeGateway(endpoint, BASIC_RECORDING);
    const { connection, bridge, states } = connectBridge(endpoint);
    const store = sessionStore();
    cleanups.push(startDesktopSession(bridge, store, "test"));
    connection.start();
    await waitFor(() => first.receivedMethods.includes("thread/list"));

    await first.close();
    await waitFor(() => states.at(-1)?.phase === "disconnected");
    const second = await gateway(endpoint);
    await waitFor(() => second.receivedMethods.includes("thread/list"));
    expect(second.receivedMethods[0]).toBe("initialize");
  });

  it("starts the daemon when nothing is listening and then connects", async () => {
    const endpoint = uniqueTestEndpoint();
    let daemonStarts = 0;
    const { connection, states } = connectBridge(endpoint, () => {
      daemonStarts += 1;
      void gateway(endpoint);
    });
    connection.start();
    await waitFor(() => states.at(-1)?.phase === "connected");
    expect(daemonStarts).toBe(1);
  });
});

describe("agent threads and cloud threads (D-19)", () => {
  it("marks only the threads that a cloud agent made", () => {
    expect(isAgentThreadSource("aeon_child")).toBe(true);
    expect(isAgentThreadSource("aeon")).toBe(true);
    expect(isAgentThreadSource("user")).toBe(false);
    expect(isAgentThreadSource(null)).toBe(false);

    const thread = (id: string, threadSource: string | null) => ({ id, cwd: "/work", name: id, preview: "", updatedAt: 1, status: { type: "idle" }, gitInfo: null, threadSource });
    const folders = groupThreadsByCwd([thread("agent", "aeon_child"), thread("mine", "user"), thread("old", null)] as never);
    const byId = Object.fromEntries(folders.flatMap((folder) => folder.threads).map((item) => [item.id, item.agentThread === true]));
    expect(byId).toEqual({ agent: true, mine: false, old: false });
  });
});

describe("a remote host over the fake gateway (plan 25)", () => {
  it("selects the host with the request path and reads its threads without writing", async () => {
    const fake = await startFakeGateway(uniqueTestEndpoint(), BASIC_RECORDING, { hosts: ["cloud"] });
    cleanups.push(() => fake.close());
    const { connection, bridge } = connectBridge(fake.endpoint, undefined, "cloud");
    const store = sessionStore();
    const session = new DesktopSession(bridge, store, "test", undefined, true);
    session.start();
    cleanups.push(() => session.stop());
    connection.start();

    await waitFor(() => (store.snapshot.folders?.length ?? 0) > 0);
    expect(fake.receivedMethods).toEqual([]);
    expect(remoteHostKey({ gateway: store.snapshot.gateway ?? null })).toBe("cloud");
    // 전체 검사로 목록을 한 번 더 읽지 않는다.
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(fake.remoteReceivedMethods.filter((method) => method === "thread/list")).toHaveLength(1);

    await expect(session.sendMessage("hello")).rejects.toThrow("읽기 전용");
    await expect(session.startThread("/work/example-app")).rejects.toThrow("읽기 전용");
    await expect(session.interrupt()).rejects.toThrow("읽기 전용");
    expect(fake.remoteReceivedMethods.some((method) => method.startsWith("turn/") || method === "thread/start")).toBe(false);
  });

  it("reads the dot list of a remote host once and keeps it empty when the gateway has none", async () => {
    const dot = { id: "dot-1", displayName: "Mika", status: "active", kind: "orbit", paused: false, rootThreadId: "thread-1", messagingRoomId: "room-1" };
    const withDots = await startFakeGateway(uniqueTestEndpoint(), BASIC_RECORDING, { hosts: ["cloud"], dots: [dot], dotMessages: [] });
    cleanups.push(() => withDots.close());
    const first = connectBridge(withDots.endpoint, undefined, "cloud");
    const store = sessionStore();
    const session = new DesktopSession(first.bridge, store, "test", undefined, true);
    session.start();
    cleanups.push(() => session.stop());
    first.connection.start();
    await waitFor(() => (store.snapshot.dots?.length ?? 0) === 1);
    expect(store.snapshot.dots).toEqual([dot]);
    // dot 의 방을 보기 시작하고, Gateway 가 미는 실시간 상태를 저장소에 넣는다.
    await waitFor(() => (store.snapshot as { lastDotLive?: unknown }).lastDotLive !== undefined);
    expect(withDots.remoteReceivedMethods.filter((method) => method === "akra/dots/watch")).toHaveLength(1);
    expect((store.snapshot as { lastDotLive?: unknown }).lastDotLive).toEqual({ roomId: "room-1", connected: true, working: false, activityLabel: null, usingComputer: false });
    expect(withDots.remoteReceivedMethods.filter((method) => method === "akra/dots/list")).toHaveLength(1);
    // local 접속은 dot 목록을 묻지 않는다.
    expect(withDots.receivedMethods).not.toContain("akra/dots/list");

    const withoutDots = await startFakeGateway(uniqueTestEndpoint(), BASIC_RECORDING, { hosts: ["cloud"] });
    cleanups.push(() => withoutDots.close());
    const second = connectBridge(withoutDots.endpoint, undefined, "cloud");
    const emptyStore = sessionStore();
    const emptySession = new DesktopSession(second.bridge, emptyStore, "test", undefined, true);
    emptySession.start();
    cleanups.push(() => emptySession.stop());
    second.connection.start();
    await waitFor(() => emptyStore.snapshot.dots !== undefined);
    expect(emptyStore.snapshot.dots).toEqual([]);
    expect(emptyStore.snapshot.sessionError ?? null).toBeNull();
  });

  it("opens a remote thread with thread/read and never resumes it", async () => {
    const fake = await startFakeGateway(uniqueTestEndpoint(), APPROVAL_TURN_RECORDING, { hosts: ["cloud"] });
    cleanups.push(() => fake.close());
    const { connection, bridge } = connectBridge(fake.endpoint, undefined, "cloud");
    const store = sessionStore();
    const session = new DesktopSession(bridge, store, "test", undefined, true);
    session.start();
    cleanups.push(() => session.stop());
    connection.start();
    await waitFor(() => store.snapshot.gateway !== undefined || store.snapshot.sessionError !== undefined);

    await session.openThread("01a1abfb-8f0a-7581-a8e4-c2953347f470");
    expect(store.snapshot.transcript?.threadId).toBe("01a1abfb-8f0a-7581-a8e4-c2953347f470");
    await session.openThread("another-thread");
    expect(store.snapshot.transcriptError).toBeNull();
    expect(fake.remoteReceivedMethods).toContain("thread/read");
    expect(fake.remoteReceivedMethods.some((method) => method === "thread/resume" || method === "thread/unsubscribe")).toBe(false);
  });

  it("allows sending after the user permits it, resuming the thread and sending a turn", async () => {
    const fake = await startFakeGateway(uniqueTestEndpoint(), APPROVAL_TURN_RECORDING, { hosts: ["cloud"] });
    cleanups.push(() => fake.close());
    const { connection, bridge } = connectBridge(fake.endpoint, undefined, "cloud");
    const store = sessionStore();
    const session = new DesktopSession(bridge, store, "test", undefined, true);
    session.start();
    cleanups.push(() => session.stop());
    connection.start();
    await waitFor(() => store.snapshot.gateway !== undefined || store.snapshot.sessionError !== undefined);

    await session.openThread("01a1abfb-8f0a-7581-a8e4-c2953347f470");
    expect(fake.remoteReceivedMethods).toContain("thread/read");
    expect(session.canSend()).toBe(false);

    // 사용자가 허락하면 thread/resume 으로 다시 구독하고, 턴을 보낼 수 있다.
    await session.allowSending();
    expect(session.canSend()).toBe(true);
    expect(fake.remoteReceivedMethods).toContain("thread/resume");

    // 가짜 Gateway 는 turn 응답을 녹화하지 않아 거절할 수 있다. 쓰기 method 가 Gateway 로 갔는지만 본다.
    await session.sendMessage("hello").catch(() => undefined);
    expect(fake.remoteReceivedMethods.some((method) => method === "turn/start" || method === "turn/steer")).toBe(true);
    // 허락해도 새 스레드는 만들지 않는다 (plan 25 §2).
    expect(fake.remoteReceivedMethods).not.toContain("thread/start");
  });

  it("does not reconnect to a host that the gateway does not know", async () => {
    const fake = await gateway();
    const { connection, states } = connectBridge(fake.endpoint, undefined, "missing");
    connection.start();

    await waitFor(() => states.at(-1)?.phase === "disconnected");
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(states.filter((state) => state.phase === "connecting")).toHaveLength(1);
    expect(states.at(-1)?.retryInMs).toBeUndefined();
    expect(states.at(-1)?.detail).toContain("missing");
  });
});

describe("opening a thread over the fake gateway", () => {
  it("resumes the thread and streams the recorded turn into the transcript", async () => {
    const fake = await startFakeGateway(uniqueTestEndpoint(), APPROVAL_TURN_RECORDING);
    cleanups.push(() => fake.close());
    const { connection, bridge } = connectBridge(fake.endpoint);
    const store = sessionStore();
    const session = new DesktopSession(bridge, store, "test", (callback) => setTimeout(callback, 0));
    session.start();
    cleanups.push(() => session.stop());
    connection.start();
    await waitFor(() => store.snapshot.gateway !== undefined || store.snapshot.sessionError !== undefined);

    // The recording has no thread/list, so the folder list stays empty; open the recorded thread directly.
    await session.openThread("01a1abfb-8f0a-7581-a8e4-c2953347f470");
    expect(fake.receivedMethods).toContain("thread/resume");
    // The replayed approval request waits for the user; answering it lets the recording continue.
    await waitFor(() => (store.snapshot.pendingApprovals?.length ?? 0) === 1);
    const approval = store.snapshot.pendingApprovals![0]!;
    expect(approval).toMatchObject({ kind: "command", threadId: "01a1abfb-8f0a-7581-a8e4-c2953347f470" });
    session.answerApproval(approval.requestId, "accept");
    expect(store.snapshot.pendingApprovals).toHaveLength(0);
    await waitFor(() => store.snapshot.transcript?.turns.at(-1)?.status === "completed");
    const items = store.snapshot.transcript?.turns.at(-1)?.items ?? [];
    expect(items.some((item) => item.type === "commandExecution")).toBe(true);
    expect(items.some((item) => item.type === "agentMessage" && item.text.length > 0)).toBe(true);
  });
});

describe("groupThreadsByCwd", () => {
  it("groups by cwd, newest first, with readable titles", () => {
    const thread = (id: string, cwd: string, updatedAt: number, name: string | null, preview: string) =>
      ({ id, cwd, updatedAt, name, preview }) as unknown as Thread;
    const folders = groupThreadsByCwd([
      thread("a", "/x", 1, null, ""),
      thread("b", "/y", 3, "Named", "ignored"),
      thread("c", "/x", 2, null, " preview "),
    ]);
    expect(folders.map((folder) => folder.cwd)).toEqual(["/y", "/x"]);
    expect(folders[1]?.threads.map((item) => item.title)).toEqual(["preview", "(제목 없음)"]);
  });
});
