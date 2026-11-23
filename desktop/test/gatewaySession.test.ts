// main 의 GatewayConnection 과 renderer 세션을 가짜 Gateway 에 실제로 연결해서 시험한다. 턴은 쓰지 않는다.
import { createServer, type Socket } from "node:net";

import { DEFAULT_SETTINGS } from "../src/shared/settings";
import { afterEach, describe, expect, it } from "vitest";

import type { AkraBridge, ConnectionState } from "../src/shared/bridge";
import type { ConfirmIssueAutomationModeRequest } from "../src/shared/issueConfirmation";
import { GatewayConnection } from "../src/main/gatewayConnection";
import { EMPTY_KERNEL_VIEW } from "../src/renderer/kernel/kernelConnection";
import { DesktopSession, groupThreadsByCwd, startDesktopSession, isAgentThreadSource, type SessionBridge } from "../src/renderer/session/desktopSession";
import { type DesktopState, mergeWorkspaceFolders, remoteHostKey, removeWorkspaceThread, renameWorkspaceThread, type SessionStore } from "../src/renderer/session/desktopStore";
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
    revealStoragePath: async () => undefined,
    delegatedChat: { enabled: async () => false, open: async () => false },
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
    openIssueAutomationPolicyFile: async () => undefined,
    confirmIssueDecision: async () => ({ cancelled: true }),
    confirmIssueAutomationMode: async () => ({ cancelled: true }),
    confirmIssueAutonomy: async () => ({ cancelled: true }),
    confirmIssueBindingResume: async () => ({ cancelled: true }),
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
      agent: {
        prepare: async () => { throw new Error("not used"); },
        bind: async () => undefined,
        release: async () => undefined,
        setActiveThread: async () => undefined,
        pause: async () => undefined,
        resume: async () => undefined,
        status: async () => ({ phase: "idle", threadId: null, lastAction: null, reason: null, attached: false }),
        onState: () => () => undefined,
      },
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
      repository: async () => null,
      openExternal: () => undefined,
    },
    providerCredentials: {
      legacyStatus: async () => ({ kind: "absent" }),
      migrateLegacy: async () => ({ kind: "held", reason: "unsupported" }),
      prepareLegacyDisconnect: async () => ({ kind: "prepared" }),
      cleanupLegacy: async () => ({ kind: "held", reason: "unsupported" }),
      openExternal: async () => undefined,
    },
    notion: { openExternal: () => undefined },
    dots: { getEnabled: () => Promise.resolve(false), setEnabled: () => Promise.resolve(false), openWindow: () => Promise.resolve(true), takeDraft: () => Promise.resolve(null), reportSeen: () => undefined, onSeen: () => () => undefined, hasWindow: () => Promise.resolve(false), dock: () => undefined, onDock: () => () => undefined, isSendAllowed: () => Promise.resolve(false), allowSend: () => undefined },
    remote: { open: () => undefined, close: () => undefined, send: () => undefined, onMessage: () => () => undefined, onConnectionState: () => () => undefined },
    zoom: { get: () => Promise.resolve(1), step: () => undefined, onChanged: () => () => undefined },
    isAppFocused: () => Promise.resolve(false),
    settings: { get: () => Promise.resolve(DEFAULT_SETTINGS), set: () => Promise.resolve(DEFAULT_SETTINGS), onChanged: () => () => undefined, onOpen: () => () => undefined, reportWorking: () => undefined },
    skills: {
      list: () => Promise.resolve([]),
      link: () => Promise.resolve({ ok: true }),
      check: () => Promise.resolve({ ok: true }),
      preview: () => Promise.resolve({ ok: false, message: "" }),
      apply: () => Promise.resolve({ ok: true }),
      rollback: () => Promise.resolve({ ok: true }),
    },
    vaultRestore: { open: () => Promise.resolve(), projectChanged: () => undefined },
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
    setOpenThread: (openThreadId) => Object.assign(snapshot, { openThreadId, transcript: null, transcriptError: null, writerElsewhere: false }),
    setWriterElsewhere: (writerElsewhere) => Object.assign(snapshot, { writerElsewhere }),
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
    renameThread: (threadId, title) => {
      snapshot.folders = renameWorkspaceThread(snapshot.folders ?? [], threadId, title);
    },
    removeThread: (threadId) => {
      snapshot.folders = removeWorkspaceThread(snapshot.folders ?? [], threadId);
    },
    getOpenThreadId: () => snapshot.openThreadId ?? null,
    getTranscript: () => snapshot.transcript ?? null,
    setKernel: (kernel) => Object.assign(snapshot, { kernel }),
    setDots: (dots) => Object.assign(snapshot, { dots }),
    setChats: (chats, chatsProblem) => Object.assign(snapshot, { chats, chatsProblem }),
    setChatConversation: (chatConversation) => Object.assign(snapshot, { chatConversation }),
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

describe("Gateway handshake cancellation", () => {
  it("never starts a daemon when closed before the WebSocket handshake completes", async () => {
    const endpoint = uniqueTestEndpoint();
    const server = createServer(); // Accept transport but never answer the WebSocket upgrade.
    await new Promise<void>((resolve, reject) => {
      server.once("error", reject);
      server.listen(endpoint, resolve);
    });
    cleanups.push(() => new Promise<void>((resolve) => server.close(() => resolve())));

    let daemonStarts = 0;
    const connection = new GatewayConnection({
      endpoint,
      onMessage: () => undefined,
      onState: () => undefined,
      startDaemon: () => { daemonStarts += 1; },
      reconnectInitialMs: 20,
    });
    cleanups.push(() => connection.close());

    const accepted = new Promise<Socket>((resolve) => server.once("connection", resolve));
    connection.start();
    const socket = await accepted;
    const disconnected = new Promise<void>((resolve) => socket.once("close", resolve));
    connection.close();
    // Make the peer release its unfinished handshake so both close handlers run.
    socket.destroy();
    await disconnected;
    await new Promise<void>((resolve) => setImmediate(resolve));
    expect(daemonStarts).toBe(0);
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

/** Controllable JSON-RPC transport for out-of-order resume responses (without model calls). */
function controlledResumeBridge() {
  type Request = { id: number; method: string; params?: { threadId?: string } };
  const requests: Request[] = [];
  const messageListeners = new Set<(text: string) => void>();
  const stateListeners = new Set<(state: ConnectionState) => void>();
  const emit = (message: object) => {
    for (const listener of messageListeners) listener(JSON.stringify({ jsonrpc: "2.0", ...message }));
  };
  const respond = (id: number, result: unknown) => emit({ id, result });
  const fail = (id: number, message: string) => emit({ id, error: { code: -32000, message } });
  const bridge: SessionBridge = {
    send: (text) => {
      const message = JSON.parse(text) as Partial<Request>;
      if (typeof message.id !== "number" || !message.method) return;
      const request = message as Request;
      requests.push(request);
      if (request.method === "thread/resume" || request.method === "thread/turns/list") return;
      const result = request.method === "thread/list" ? { data: [], nextCursor: null }
        : request.method === "akra/gateway/info"
          ? { gatewayVersion: "test", codexVersion: null, startedAt: "", upstreamRestarts: 0 }
          : {};
      queueMicrotask(() => respond(request.id, result));
    },
    onMessage: (listener) => {
      messageListeners.add(listener);
      return () => messageListeners.delete(listener);
    },
    onConnectionState: (listener) => {
      stateListeners.add(listener);
      return () => stateListeners.delete(listener);
    },
    ready: () => queueMicrotask(() => {
      for (const listener of stateListeners) listener({ phase: "connected", endpoint: "controlled" });
    }),
  };
  const pending = (method: string) => requests.filter((request) => request.method === method);
  return { bridge, pending, respond, fail, emit };
}

function controlledThread(id: string, title: string, turnId?: string): Thread {
  return {
    id, name: title, preview: "", cwd: "/work", status: { type: "idle" },
    turns: turnId ? [{ id: turnId, status: "completed", items: [] }] : [],
  } as unknown as Thread;
}

describe("out-of-order resume navigation", () => {
  async function setup() {
    const controlled = controlledResumeBridge();
    const store = sessionStore();
    const session = new DesktopSession(controlled.bridge, store, "test");
    session.start();
    cleanups.push(() => session.stop());
    await waitFor(() => store.snapshot.gateway !== undefined);
    return { ...controlled, store, session };
  }

  it("keeps the second A transcript when the first A response returns after A → B → A", async () => {
    const { session, store, pending, respond } = await setup();
    const firstA = session.openThread("A");
    await waitFor(() => pending("thread/resume").length === 1);
    const openB = session.openThread("B");
    await waitFor(() => pending("thread/resume").length === 2);
    const latestA = session.openThread("A");
    await waitFor(() => pending("thread/resume").length === 3);
    const [oldA, b, newA] = pending("thread/resume");
    respond(newA!.id, { thread: controlledThread("A", "latest", "latest-turn") });
    await latestA;
    expect(store.snapshot.transcript?.title).toBe("latest");
    respond(oldA!.id, { thread: controlledThread("A", "stale", "old-turn") });
    await firstA;
    respond(b!.id, { thread: controlledThread("B", "wrong", "b-turn") });
    await openB;
    expect(store.snapshot.transcript?.title).toBe("latest");
    expect(store.snapshot.transcript?.turns.map((turn) => turn.id)).toEqual(["latest-turn"]);
  });

  it("rejects an old A page and an old error after a new A resume", async () => {
    const { session, store, pending, respond, fail } = await setup();
    const firstA = session.openThread("A");
    await waitFor(() => pending("thread/resume").length === 1);
    respond(pending("thread/resume")[0]!.id, { thread: controlledThread("A", "old") });
    await waitFor(() => pending("thread/turns/list").length === 1);
    const openB = session.openThread("B");
    await waitFor(() => pending("thread/resume").length === 2);
    const latestA = session.openThread("A");
    await waitFor(() => pending("thread/resume").length === 3);
    respond(pending("thread/resume")[2]!.id, { thread: controlledThread("A", "latest", "latest-turn") });
    await latestA;
    respond(pending("thread/turns/list")[0]!.id, {
      data: [{ id: "stale-page", status: "completed", items: [] }], nextCursor: null,
    });
    await firstA;
    fail(pending("thread/resume")[1]!.id, "stale B failure");
    await openB;
    expect(store.snapshot.transcript?.title).toBe("latest");
    expect(store.snapshot.transcript?.turns.map((turn) => turn.id)).toEqual(["latest-turn"]);
    expect(store.snapshot.transcriptError).toBeNull();
  });
  async function openPaged(fixture: Awaited<ReturnType<typeof setup>>, id: string, title: string) {
    const { session, pending, respond } = fixture;
    const open = session.openThread(id);
    respond(pending("thread/resume").at(-1)!.id, { thread: controlledThread(id, title) });
    await waitFor(() => pending("thread/turns/list").at(-1)?.params?.threadId === id);
    respond(pending("thread/turns/list").at(-1)!.id, {
      data: [{ id: title + "-turn", status: "inProgress", items: [] }], nextCursor: "same-cursor",
    });
    await open;
  }

  it("merges older turns into the latest transcript while streaming changes it", async () => {
    const fixture = await setup();
    await openPaged(fixture, "A", "latest");
    const { session, store, pending, respond, emit } = fixture;
    const older = session.loadOlderTurns();
    const page = pending("thread/turns/list").at(-1)!;
    const plan = [{ step: "new streamed plan", status: "inProgress" }];
    emit({ method: "turn/plan/updated", params: { threadId: "A", turnId: "latest-turn", explanation: null, plan } });
    await waitFor(() => store.snapshot.transcript?.turns.at(-1)?.plan?.[0]?.step === "new streamed plan");
    respond(page.id, { data: [{ id: "older-turn", status: "completed", items: [] }], nextCursor: "next-cursor" });
    await older;
    expect(store.snapshot.transcript?.turns.map((turn) => turn.id)).toEqual(["older-turn", "latest-turn"]);
    expect(store.snapshot.transcript?.turns.at(-1)?.plan).toEqual(plan);
    expect(store.snapshot.transcript?.olderTurnsCursor).toBe("next-cursor");
  });

  it("rejects old A pagination after A → B → A even when the cursor repeats", async () => {
    const fixture = await setup();
    await openPaged(fixture, "A", "first");
    const older = fixture.session.loadOlderTurns();
    const oldPage = fixture.pending("thread/turns/list").at(-1)!;
    await openPaged(fixture, "B", "B");
    await openPaged(fixture, "A", "new-A");
    fixture.respond(oldPage.id, { data: [{ id: "stale-older", status: "completed", items: [] }], nextCursor: null });
    await older;
    expect(fixture.store.snapshot.transcript?.turns.map((turn) => turn.id)).toEqual(["new-A-turn"]);
    expect(fixture.store.snapshot.transcript?.olderTurnsCursor).toBe("same-cursor");
  });

  it("applies the first response only when two older-page requests use the same cursor", async () => {
    const fixture = await setup();
    await openPaged(fixture, "A", "A");
    const first = fixture.session.loadOlderTurns();
    const second = fixture.session.loadOlderTurns();
    const [firstPage, secondPage] = fixture.pending("thread/turns/list").slice(-2);
    fixture.respond(secondPage!.id, { data: [{ id: "first-result", status: "completed", items: [] }], nextCursor: "next" });
    await second;
    fixture.respond(firstPage!.id, { data: [{ id: "duplicate-result", status: "completed", items: [] }], nextCursor: null });
    await first;
    expect(fixture.store.snapshot.transcript?.turns.map((turn) => turn.id)).toEqual(["first-result", "A-turn"]);
    expect(fixture.store.snapshot.transcript?.olderTurnsCursor).toBe("next");
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

describe("issue interests over the shared Gateway session", () => {
  it("keeps one renderer attach across confirmed issue actions, events and reconnection", async () => {
    let mode: "off" | "observe" = "off";
    let revision = 3;
    const commands: Array<Record<string, unknown>> = [];
    const confirmations: ConfirmIssueAutomationModeRequest[] = [];
    const state = () => ({
      type: "issue_automation", status: "ready", mode, epoch: 1, revision,
      binding: null, binding_id: null, collecting: false, consecutive_failures: 0,
      last_attempt_at_ms: 100, last_success_at_ms: 90, last_failure: null, next_poll_after_ms: null,
    });
    const page = () => ({
      type: "issues", status: "ready", revision, items: [], next_after: null,
      groups: [{ id: "collection", label: "수집한 이슈", item_indices: [] }],
    });
    const fake = await startFakeGateway(uniqueTestEndpoint(), BASIC_RECORDING, {
      kernel: {
        kernel: "shared-repository",
        queries: {},
        onRequest(method, params) {
          const query = params.query as { type?: string } | undefined;
          if (method === "akra/kernel/query" && query?.type === "issue_automation") return { result: state() };
          if (method === "akra/kernel/query" && query?.type === "issues") return { result: page() };
          if (method === "akra/kernel/command") {
            commands.push(params);
            return { result: { processed: true } };
          }
          return undefined;
        },
      },
    });
    cleanups.push(() => fake.close());
    const { connection, bridge } = connectBridge(fake.endpoint);
    // native 확인과 main 전용 전송은 별도 회귀가 검사한다. 여기서는 승인 결과와 서버의 저장 상태를 잇는다.
    bridge.confirmIssueAutomationMode = async (request) => {
      confirmations.push(structuredClone(request));
      mode = request.mode;
      revision += 1;
      return { cancelled: false, response: { processed: true } };
    };
    const store = sessionStore();
    const session = new DesktopSession(bridge, store, "test");
    session.start();
    cleanups.push(() => session.stop());
    await session.showKernel("/work/example-app", ["issues"]);
    connection.start();
    await waitFor(() => store.getKernel().issues.page?.status === "ready");
    expect(fake.receivedMethods.filter((method) => method === "akra/kernel/attach")).toHaveLength(1);
    expect(commands).toHaveLength(0);
    expect(confirmations).toHaveLength(0);
    expect(store.getKernel().issues.automation).toMatchObject({ mode: "off", revision: 3 });

    await session.setIssueAutomationMode("observe");
    expect(confirmations).toEqual([{ workspace: "/work/example-app", mode: "observe", key: expect.stringMatching(/^[0-9a-f-]{36}$/) }]);
    expect(commands).toHaveLength(0);
    expect(store.getKernel().issues.automation).toMatchObject({ mode: "observe", revision: 4 });
    await session.syncIssues();
    expect(commands).toHaveLength(1);
    expect(commands[0]).toEqual({ command: { type: "sync_issues" } });

    revision = 5;
    fake.sendKernelEvent({ ...state(), type: "issues" });
    await waitFor(() => {
      const current = store.getKernel().issues.page;
      return current?.status === "ready" && current.revision === 5 && !store.getKernel().issues.loading;
    });
    fake.disconnectClients();
    await waitFor(() => fake.receivedMethods.filter((method) => method === "akra/kernel/attach").length === 2);
    await waitFor(() => store.getKernel().issues.page?.status === "ready");
    expect(store.getKernel().issues.pageNumber).toBe(1);
    expect(commands).toHaveLength(1);
    expect(confirmations).toHaveLength(1);
    expect(store.getKernel().issues.automation).toMatchObject({ mode: "observe", revision: 5 });
    expect(fake.receivedMethods).not.toContain("turn/start");
    expect(fake.receivedMethods).not.toContain("turn/steer");

    await session.showKernel(null);
    expect(fake.receivedMethods.filter((method) => method === "akra/kernel/detach")).toHaveLength(1);
    expect(store.getKernel()).toEqual(EMPTY_KERNEL_VIEW);
  });
});
