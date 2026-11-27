// 입력과 승인 응답이 어떤 요청을 보내는지 시험한다. 서버 역할은 손으로 쓴 응답으로 대신한다 (턴 없음).
import { describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS } from "../src/shared/settings";

import type { AkraBridge, ConnectionState } from "../src/shared/bridge";
import { BROWSER_TOOL_NAMES } from "../src/shared/browserAgent";
import { EMPTY_KERNEL_VIEW } from "../src/renderer/kernel/kernelConnection";
import { DesktopSession, FORK_RESPONSE_TIMEOUT_MS, forkBlockReason } from "../src/renderer/session/desktopSession";
import { type DesktopState, mergeWorkspaceFolders, removeWorkspaceThread, renameWorkspaceThread, type SessionStore } from "../src/renderer/session/desktopStore";
import type { Transcript } from "../src/renderer/conversation/transcript";

interface Sent {
  id?: number | string;
  method?: string;
  params?: Record<string, unknown>;
  result?: unknown;
}

function harness(readOnly = false, scheduler: (callback: () => void) => void = (callback) => callback()) {
  const sent: Sent[] = [];
  const requestListeners = new Set<(message: Sent) => void>();
  let readyCalls = 0;
  const messageListeners = new Set<(text: string) => void>();
  const stateListeners = new Set<(state: ConnectionState) => void>();
  const bridge: AkraBridge = {
    diagnostics: { preview: async () => { throw new Error("not used"); }, save: async () => ({ status: "cancelled" }), discard: async () => undefined },
    confirmTaskCriterionWaiver: async () => ({ cancelled: true }),
    revealStoragePath: async () => undefined,
    delegatedChat: { enabled: async () => false, open: async () => false },
    send: (text) => {
      const message = JSON.parse(text) as Sent;
      sent.push(message);
      requestListeners.forEach((listener) => listener(message));
    },
    onMessage: (listener) => (messageListeners.add(listener), () => messageListeners.delete(listener)),
    onConnectionState: (listener) => (stateListeners.add(listener), () => stateListeners.delete(listener)),
    ready: () => {
      readyCalls += 1;
    },
    chooseFolder: async () => null,
    openFolder: async () => false,
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
        prepare: vi.fn(async () => ({ registrationId: "registration-1", sessionFile: "/private/browser.json", command: "/app/electron", args: ["/app/browser-mcp.js"] })),
        bind: vi.fn(async () => undefined),
        release: vi.fn(async () => undefined),
        setActiveThread: vi.fn(async () => undefined),
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
    zoom: { get: () => Promise.resolve({}), step: () => undefined, onChanged: () => () => undefined, onCommand: () => () => undefined },
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
  const state: Partial<DesktopState> = { pendingApprovals: [], openThreadId: null, transcript: null, folders: [] };
  const store: SessionStore = {
    setConnection: () => undefined,
    setSession: (session) => Object.assign(state, session),
    setSessionError: () => undefined,
    setThreadActive: () => undefined,
    setThreadFailed: () => undefined,
    setActiveThreads: (threadIds) => Object.assign(state, { activeThreadIds: threadIds }),
    setOpenThread: (openThreadId) => Object.assign(state, { openThreadId, transcript: null, writerElsewhere: false }),
    setTranscript: (transcript, transcriptError = null) => Object.assign(state, { transcript, transcriptError }),
    setWriterElsewhere: (writerElsewhere) => Object.assign(state, { writerElsewhere }),
    updateTranscript: (update) => {
      if (state.transcript) state.transcript = update(state.transcript);
    },
    addApproval: (approval) => {
      state.pendingApprovals = [...(state.pendingApprovals ?? []), approval];
    },
    removeApproval: (requestId) => {
      state.pendingApprovals = (state.pendingApprovals ?? []).filter((item) => String(item.requestId) !== String(requestId));
    },
    clearApprovals: () => {
      state.pendingApprovals = [];
    },
    addThreadToFolder: (cwd, thread) => {
      state.folders = [{ cwd, threads: [thread] }];
    },
    mergeFolders: (folders) => {
      state.folders = mergeWorkspaceFolders(state.folders ?? [], folders);
    },
    renameThread: (threadId, title) => {
      state.folders = renameWorkspaceThread(state.folders ?? [], threadId, title);
    },
    removeThread: (threadId) => {
      state.folders = removeWorkspaceThread(state.folders ?? [], threadId);
    },
    getOpenThreadId: () => state.openThreadId ?? null,
    getActiveThreadIds: () => state.activeThreadIds ?? [],
    getTranscript: () => state.transcript ?? null,
    setKernel: (kernel) => Object.assign(state, { kernel }),
    setDots: (dots) => Object.assign(state, { dots }),
    setChats: (chats, chatsProblem) => Object.assign(state, { chats, chatsProblem }),
    setChatConversation: (chatConversation) => Object.assign(state, { chatConversation }),
    applyDotLive: (live) => Object.assign(state, { lastDotLive: live }),
    getKernel: () => state.kernel ?? EMPTY_KERNEL_VIEW,
  };
  const session = new DesktopSession(bridge, store, "test", scheduler, readOnly);
  session.start();
  const receive = (message: object) => messageListeners.forEach((listener) => listener(JSON.stringify(message)));
  /** 마지막으로 보낸 요청에 결과로 답한다. */
  const reply = (result: unknown) => receive({ jsonrpc: "2.0", id: sent.at(-1)?.id, result });
  const openWith = (transcript: Transcript) => {
    state.openThreadId = transcript.threadId;
    state.transcript = transcript;
  };
  const emitState = (phase: ConnectionState["phase"]) =>
    stateListeners.forEach((listener) => listener({ phase, endpoint: "test" }));
  /** 아직 답하지 않은 요청 중 method 가 맞는 마지막 요청에 답한다. */
  const replyTo = (method: string, result: unknown) => {
    const request = [...sent].reverse().find((message) => message.method === method && message.id !== undefined);
    receive({ jsonrpc: "2.0", id: request?.id, result });
  };
  const nextRequest = (method: string) => new Promise<Sent>((resolve) => {
    const listener = (message: Sent) => {
      if (message.method !== method) return;
      requestListeners.delete(listener);
      resolve(message);
    };
    requestListeners.add(listener);
  });
  return { session, sent, state, receive, reply, replyTo, openWith, emitState, nextRequest, agent: bridge.browser.agent, readyCalls: () => readyCalls };
}

const transcript = (status: "inProgress" | "completed"): Transcript => ({
  threadId: "t1",
  title: "t",
  cwd: "/work/example-app",
  status: null,
  turns: [{ id: "turn-1", status, errorMessage: null, items: [], diff: null, plan: null, startedAt: null, durationMs: null }],
  olderTurnsCursor: null,
  tokenUsage: null,
});

describe("model settings on session input", () => {
  const catalog = (efforts = ["low", "high"]) => ({ data: [{ model: "model-a", displayName: "Model A", hidden: false,
    supportedReasoningEfforts: efforts.map((reasoningEffort) => ({ reasoningEffort, description: reasoningEffort })),
    defaultReasoningEffort: "low" }], nextCursor: null });
  const ready = async (h: ReturnType<typeof harness>) => {
    h.emitState("connected"); h.replyTo("initialize", {}); await Promise.resolve();
    const loading = h.session.nextTurnSettings.refresh();
    h.replyTo("model/list", catalog()); await loading;
    h.receive({ method: "thread/settings/updated", params: { threadId: "t1", threadSettings: { model: "model-a", effort: "low" } } });
  };

  it("adds model settings only to a new turn and consumes them after acknowledgment", async () => {
    const h = harness(); await ready(h); h.openWith(transcript("completed"));
    h.session.nextTurnSettings.setModel("t1", "model-a"); h.session.nextTurnSettings.setEffort("t1", "high");
    const sending = h.session.sendMessage("new task");
    expect(h.sent.at(-1)).toMatchObject({ method: "turn/start", params: { threadId: "t1", model: "model-a", effort: "high" } });
    expect(Object.keys(h.sent.at(-1)!.params!).sort()).toEqual(["effort", "input", "model", "threadId"]);
    h.replyTo("turn/start", { turn: { id: "new-turn" } }); await sending;
    expect(h.session.nextTurnSettings.snapshot("t1").draft).toEqual({ model: null, effort: null });
    expect(h.session.nextTurnSettings.snapshot("t1").current).toBeNull();
    h.receive({ method: "thread/settings/updated", params: { threadId: "t1", threadSettings: { model: "model-a", effort: "high" } } });
    expect(h.session.nextTurnSettings.snapshot("t1").current?.effort).toBe("high");
    h.session.stop();
  });

  it("keeps the pending settings through steer and applies them to the next reserved turn", async () => {
    const h = harness(); await ready(h); h.openWith(transcript("inProgress"));
    h.session.nextTurnSettings.setModel("t1", "model-a"); h.session.nextTurnSettings.setEffort("t1", "high");
    const steer = h.session.sendMessage("additional instruction");
    expect(h.sent.at(-1)).toMatchObject({ method: "turn/steer", params: { expectedTurnId: "turn-1" } });
    expect(h.sent.at(-1)?.params).not.toHaveProperty("model");
    expect(h.sent.at(-1)?.params).not.toHaveProperty("effort");
    h.replyTo("turn/steer", { turnId: "turn-1" }); await steer;
    expect(h.session.nextTurnSettings.snapshot("t1").draft.effort).toBe("high");
    h.session.queueMessage("next task");
    h.receive({ method: "turn/completed", params: { threadId: "t1", turn: { id: "turn-1", status: "completed", items: [], error: null } } });
    expect(h.sent.at(-1)).toMatchObject({ method: "turn/start", params: { model: "model-a", effort: "high" } });
    h.replyTo("turn/start", { turn: { id: "queued-turn" } });
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    h.session.stop();
  });

  it("preserves model settings through compaction and consumes them only on the released reserved turn", async () => {
    const h = harness(); await ready(h); h.openWith(transcript("completed"));
    h.session.nextTurnSettings.setModel("t1", "model-a"); h.session.nextTurnSettings.setEffort("t1", "high");
    const draft = { model: "model-a", effort: "high" };
    h.session.compaction.start("t1");
    expect(h.sent.at(-1)).toMatchObject({ method: "thread/compact/start", params: { threadId: "t1" } });
    expect(Object.keys(h.sent.at(-1)!.params!)).toEqual(["threadId"]);
    h.replyTo("thread/compact/start", {});
    h.session.nextTurnQueue.enqueue("t1", "after compact", null);
    expect(() => h.session.resumeQueuedMessages()).toThrow();
    h.receive({ method: "turn/started", params: { threadId: "t1", turn: { id: "compact-turn", status: "inProgress", items: [], error: null } } });
    expect(h.sent.some(message => message.method === "turn/start" || message.method === "turn/steer")).toBe(false);
    expect(h.session.nextTurnSettings.snapshot("t1").draft).toEqual(draft);

    const item = { type: "contextCompaction", id: "compact-item" };
    h.receive({ method: "item/completed", params: { threadId: "t1", turnId: "compact-turn", item } });
    h.receive({ method: "turn/completed", params: { threadId: "t1", turn: { id: "compact-turn", status: "completed", items: [item], error: null } } });
    expect(h.session.compaction.snapshot("t1").phase).toBe("completed");
    expect(h.session.nextTurnSettings.snapshot("t1").draft).toEqual(draft);
    expect(h.sent.some(message => message.method === "turn/start")).toBe(false);
    h.session.resumeQueuedMessages();
    expect(h.sent.filter(message => message.method === "turn/start")).toHaveLength(1);
    expect(h.sent.at(-1)).toMatchObject({ method: "turn/start", params: { threadId: "t1", model: "model-a", effort: "high" } });
    h.replyTo("turn/start", { turn: { id: "after-compact-turn" } });
    await flush();
    expect(h.session.nextTurnSettings.snapshot("t1").draft).toEqual({ model: null, effort: null });
    h.session.stop();
  });

  it("holds a reservation before sending when the supported effort disappears", async () => {
    const h = harness(); await ready(h); h.openWith(transcript("inProgress"));
    h.session.nextTurnSettings.setModel("t1", "model-a"); h.session.nextTurnSettings.setEffort("t1", "high");
    h.session.queueMessage("wait for valid settings");
    const loading = h.session.nextTurnSettings.refresh(); h.replyTo("model/list", catalog(["low"])); await loading;
    h.receive({ method: "turn/completed", params: { threadId: "t1", turn: { id: "turn-1", status: "completed", items: [], error: null } } });
    expect(h.sent.some((message) => message.method === "turn/start")).toBe(false);
    expect(h.session.nextTurnQueue.snapshot("t1")).toMatchObject({ items: [{ state: "queued" }], paused: expect.stringContaining("지원 목록") });
    h.session.nextTurnSettings.reset("t1"); h.session.resumeQueuedMessages();
    expect(h.sent.at(-1)).toMatchObject({ method: "turn/start" });
    expect(h.sent.at(-1)?.params).not.toHaveProperty("effort");
    h.replyTo("turn/start", { turn: { id: "after-review" } });
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    h.session.stop();
  });
});

describe("queued session input", () => {
  it("resumes a paused reservation between the terminal notification and its transcript frame", async () => {
    const frames: Array<() => void> = [];
    const h = harness(false, (callback) => frames.push(callback));
    h.emitState("connected");
    h.openWith(transcript("inProgress"));
    h.session.queueMessage("after completed turn");
    h.session.nextTurnQueue.pause("t1", "paused for review");
    const completed = { method: "turn/completed", params: { threadId: "t1", turn: { id: "turn-1", status: "completed", items: [], error: null } } };
    h.receive(completed);
    expect(h.state.transcript?.turns.at(-1)?.status).toBe("inProgress");
    const starts = () => h.sent.filter((message) => message.method === "turn/start");
    expect(starts()).toHaveLength(0);
    h.session.resumeQueuedMessages();
    expect(starts()).toHaveLength(1);
    expect(starts()[0]?.params).toMatchObject({ threadId: "t1", input: [{ type: "text", text: "after completed turn" }] });
    h.replyTo("turn/start", { turn: { id: "next-turn" } });
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    for (const frame of frames.splice(0)) frame();
    h.receive(completed);
    expect(starts()).toHaveLength(1);
    expect(h.session.nextTurnQueue.snapshot("t1").waitingTurnId).toBe("next-turn");
    h.session.stop();
  });

  it("keeps an acknowledged manual turn ahead of paused reservations before the start notification", async () => {
    const { session, sent, receive, replyTo, openWith, emitState } = harness();
    emitState("connected");
    openWith(transcript("inProgress"));
    session.queueMessage("reserved");
    session.nextTurnQueue.pause("t1", "paused for review");
    receive({ method: "turn/completed", params: { threadId: "t1", turn: { id: "turn-1", status: "completed", items: [], error: null } } });
    const manual = session.sendMessage("manual first");
    replyTo("turn/start", { turn: { id: "acknowledged" } });
    await manual;
    const steer = session.sendMessage("more detail");
    expect(sent.at(-1)).toMatchObject({ method: "turn/steer", params: { expectedTurnId: "acknowledged" } });
    replyTo("turn/steer", { turnId: "acknowledged" });
    await steer;
    session.resumeQueuedMessages();
    expect(sent.filter((message) => message.method === "turn/start")).toHaveLength(1);
    expect(session.nextTurnQueue.snapshot("t1").waitingTurnId).toBe("acknowledged");
    session.stop();
  });

  it("reserves the current thread and starts once only after its terminal notification", async () => {
    const { session, sent, receive, replyTo, openWith, emitState } = harness();
    emitState("connected");
    openWith(transcript("inProgress"));
    session.queueMessage("next task");
    expect(sent.filter((message) => message.method?.startsWith("turn/"))).toHaveLength(0);
    receive({ method: "turn/completed", params: { threadId: "t1", turn: { id: "turn-1", status: "completed", items: [], error: null } } });
    const starts = () => sent.filter((message) => message.method === "turn/start");
    expect(starts()).toHaveLength(1);
    expect(starts()[0]?.params).toMatchObject({ threadId: "t1", input: [{ type: "text", text: "next task" }] });
    receive({ method: "turn/completed", params: { threadId: "t1", turn: { id: "turn-1", status: "completed", items: [], error: null } } });
    replyTo("turn/start", { turn: { id: "queued-turn" } });
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    expect(starts()).toHaveLength(1);
    expect(sent.some((message) => message.method === "turn/steer")).toBe(false);
    session.stop();
  });

  it("pauses reservations before interrupt and does not dispatch on the cancelled turn", async () => {
    const { session, sent, receive, replyTo, openWith, emitState } = harness();
    emitState("connected");
    openWith(transcript("inProgress"));
    session.queueMessage("not after cancel");
    const stopping = session.interrupt();
    replyTo("turn/interrupt", {});
    await stopping;
    receive({ method: "turn/completed", params: { threadId: "t1", turn: { id: "turn-1", status: "completed", items: [], error: null } } });
    expect(sent.filter((message) => message.method === "turn/start")).toHaveLength(0);
    expect(session.nextTurnQueue.snapshot("t1").paused).not.toBeNull();
    session.stop();
  });
});

describe("sending input", () => {
  it("starts a new turn when nothing is running", async () => {
    const { session, sent, reply, openWith } = harness();
    openWith(transcript("completed"));
    const pending = session.sendMessage("  hello  ");
    reply({ turn: { id: "turn-2" } });
    await pending;
    expect(sent.at(-1)).toMatchObject({
      method: "turn/start",
      params: { threadId: "t1", input: [{ type: "text", text: "hello", text_elements: [] }] },
    });
  });

  it("steers the running turn with its id", async () => {
    const { session, sent, reply, openWith } = harness();
    openWith(transcript("inProgress"));
    const pending = session.sendMessage("also do this");
    reply({ turnId: "turn-1" });
    await pending;
    expect(sent.at(-1)).toMatchObject({ method: "turn/steer", params: { threadId: "t1", expectedTurnId: "turn-1" } });
  });

  it("ignores empty input and interrupts only a running turn", async () => {
    const { session, sent, reply, openWith } = harness();
    openWith(transcript("completed"));
    await session.sendMessage("   ");
    await session.interrupt();
    expect(sent).toHaveLength(0);

    openWith(transcript("inProgress"));
    const pending = session.interrupt();
    reply({});
    await pending;
    expect(sent.at(-1)).toMatchObject({ method: "turn/interrupt", params: { threadId: "t1", turnId: "turn-1" } });
  });

  it("starts a thread in a folder and opens it", async () => {
    const { session, sent, state, reply } = harness();
    const pending = session.startThread("/work/example-app");
    expect(sent.at(-1)).toMatchObject({ method: "thread/start", params: { cwd: "/work/example-app" } });
    reply({ thread: { id: "new-thread", cwd: "/work/example-app", name: null, preview: "", status: { type: "idle" }, turns: [], updatedAt: 1 } });
    await pending;
    expect(state.openThreadId).toBe("new-thread");
    expect(state.folders?.[0]?.threads[0]?.id).toBe("new-thread");
  });
});

describe("browser conversations", () => {
  const thread = { id: "browser-thread", cwd: "/work/example-app", name: null, preview: "", status: { type: "idle" }, turns: [], updatedAt: 1 };
  const browserStatus = {
    name: "akra_browser",
    runtimeStatus: "connected",
    toolsError: null,
    tools: Object.fromEntries(BROWSER_TOOL_NAMES.map((name) => [name, { name }])),
  };

  it("rejects remote hosts before preparing or starting a browser thread", async () => {
    const { session, sent, agent } = harness(true);
    await expect(session.startBrowserThread("/work/example-app", agent)).rejects.toThrow();
    expect(agent.prepare).not.toHaveBeenCalled();
    expect(sent).toEqual([]);
    session.stop();
  });

  it("does not start a thread if preparation fails", async () => {
    const { session, sent, agent } = harness();
    vi.mocked(agent.prepare).mockRejectedValueOnce(new Error("prepare failed"));
    await expect(session.startBrowserThread("/work/example-app", agent)).rejects.toThrow("prepare failed");
    expect(agent.release).not.toHaveBeenCalled();
    expect(sent).toEqual([]);
    session.stop();
  });

  it("prepares, starts, selects, binds and verifies tools before displaying the thread", async () => {
    const { session, sent, state, reply, nextRequest, agent } = harness();
    const started = nextRequest("thread/start");
    const pending = session.startBrowserThread("/work/example-app", agent);
    expect(await started).toMatchObject({
      params: { cwd: "/work/example-app", config: {
        "mcp_servers.akra_browser.command": "/app/electron",
        "mcp_servers.akra_browser.args": ["/app/browser-mcp.js"],
        "mcp_servers.akra_browser.env.ELECTRON_RUN_AS_NODE": "1",
        "mcp_servers.akra_browser.env.AKRA_BROWSER_SESSION_FILE": "/private/browser.json",
      } },
    });
    expect(agent.prepare).toHaveBeenCalledOnce();
    expect(state.openThreadId).toBeNull();
    const listed = nextRequest("mcpServerStatus/list");
    reply({ thread });
    expect((await listed).params).toEqual({ threadId: thread.id, detail: "toolsAndAuthOnly" });
    expect(agent.setActiveThread).toHaveBeenCalledWith(thread.id);
    expect(agent.bind).toHaveBeenCalledWith("registration-1", thread.id);
    expect(vi.mocked(agent.setActiveThread).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(agent.bind).mock.invocationCallOrder[0]!);
    expect(state.openThreadId).toBeNull();
    reply({ data: [browserStatus], nextCursor: null });
    expect(await pending).toBe(thread.id);
    expect(state.openThreadId).toBe(thread.id);
    expect(state.transcript?.threadId).toBe(thread.id);
    expect(agent.release).not.toHaveBeenCalled();
    expect(sent.map((message) => message.method)).toEqual(["thread/start", "mcpServerStatus/list"]);
    session.stop();
  });

  it.each(["thread/start", "selection", "bind", "mcpServerStatus/list", "missing tools", "disconnected runtime"] as const)("releases the registration after %s fails", async (failure) => {
    const { session, state, reply, receive, nextRequest, agent, openWith } = harness();
    openWith(transcript("completed"));
    if (failure === "bind") vi.mocked(agent.bind).mockRejectedValueOnce(new Error("bind failed"));
    if (failure === "selection") vi.mocked(agent.setActiveThread).mockRejectedValueOnce(new Error("selection failed"));
    const started = nextRequest("thread/start");
    const pending = session.startBrowserThread("/work/example-app", agent);
    const rejected = expect(pending).rejects.toThrow();
    const request = await started;
    if (failure === "thread/start") {
      receive({ jsonrpc: "2.0", id: request.id, error: { code: -32000, message: "start failed" } });
    } else {
      const listed = failure !== "bind" && failure !== "selection" ? nextRequest("mcpServerStatus/list") : null;
      reply({ thread });
      if (listed) {
        const statusRequest = await listed;
        if (failure === "mcpServerStatus/list") {
          receive({ jsonrpc: "2.0", id: statusRequest.id, error: { code: -32000, message: "status failed" } });
        } else {
          reply({ data: [{ ...browserStatus, ...(failure === "missing tools" ? { tools: {} } : { runtimeStatus: "failed" }) }], nextCursor: null });
        }
      }
    }
    await rejected;
    expect(agent.release).toHaveBeenCalledExactlyOnceWith("registration-1");
    expect(state.openThreadId).toBe("t1");
    expect(agent.setActiveThread).toHaveBeenLastCalledWith("t1");
    session.stop();
  });

  it("reads thread-scoped status pages until it finds the browser server", async () => {
    const { session, reply, nextRequest, agent } = harness();
    const started = nextRequest("thread/start");
    const pending = session.startBrowserThread("/work/example-app", agent);
    await started;
    const listed = nextRequest("mcpServerStatus/list");
    reply({ thread });
    await listed;
    const secondPage = nextRequest("mcpServerStatus/list");
    reply({ data: [], nextCursor: "next-page" });
    expect((await secondPage).params).toEqual({ threadId: thread.id, detail: "toolsAndAuthOnly", cursor: "next-page" });
    reply({ data: [browserStatus], nextCursor: null });
    await pending;
    session.stop();
  });
});

describe("approvals", () => {
  const request = { jsonrpc: "2.0", id: 7, method: "item/commandExecution/requestApproval", params: { threadId: "t1", turnId: "turn-1", itemId: "i1", command: "npm test", kind: "command" } };

  it("answers with the chosen decision", async () => {
    const { session, sent, state, receive } = harness();
    receive(request);
    expect(state.pendingApprovals).toHaveLength(1);
    session.answerApproval(7, "decline");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(sent.at(-1)).toMatchObject({ id: 7, result: { decision: "decline" } });
    expect(state.pendingApprovals).toHaveLength(0);
  });

  it("drops the prompt without answering when another window resolved it", async () => {
    const { session, sent, state, receive } = harness();
    receive(request);
    receive({ jsonrpc: "2.0", method: "serverRequest/resolved", params: { threadId: "t1", requestId: 7 } });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(state.pendingApprovals).toHaveLength(0);
    expect(sent).toHaveLength(0);
    // A late click after the resolution sends nothing either.
    session.answerApproval(7, "accept");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(sent).toHaveLength(0);
  });

  it("rejects server requests it does not handle", async () => {
    const { sent, receive } = harness();
    receive({ jsonrpc: "2.0", id: 9, method: "item/tool/requestUserInput", params: { threadId: "t1" } });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(sent.at(-1)).toMatchObject({ id: 9, error: { code: -32601 } });
  });
});

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("review follow-ups", () => {
  it("asks main to connect only after its listeners are registered", () => {
    const { readyCalls } = harness();
    expect(readyCalls()).toBe(1);
  });

  it("reads every thread-list page and marks threads that are already active", async () => {
    const { sent, state, replyTo, emitState } = harness();
    emitState("connected");
    await flush();
    replyTo("initialize", {});
    await flush();
    replyTo("akra/gateway/info", { gatewayVersion: "1", codexVersion: "codex-cli 0.159.0", startedAt: "", upstreamRestarts: 0 });
    await flush();
    const thread = (id: string, cwd: string, active: boolean) => ({
      id,
      cwd,
      name: id,
      preview: "",
      updatedAt: 1,
      status: active ? { type: "active", activeFlags: [] } : { type: "idle" },
    });
    replyTo("thread/list", { data: [thread("a", "/one", false)], nextCursor: "page-2", backwardsCursor: null });
    await flush();
    const second = [...sent].reverse().find((message) => message.method === "thread/list");
    expect(second?.params).toMatchObject({ cursor: "page-2", useStateDbOnly: true });
    replyTo("thread/list", { data: [thread("b", "/two", true)], nextCursor: null, backwardsCursor: null });
    await flush();
    expect(state.folders?.map((folder) => folder.cwd).sort()).toEqual(["/one", "/two"]);
    expect(state.activeThreadIds).toEqual(["b"]);
  });

  it("shows the state-DB list first, then adds threads only the rollout scan finds", async () => {
    const { sent, state, replyTo, emitState } = harness();
    emitState("connected");
    await flush();
    replyTo("initialize", {});
    await flush();
    replyTo("akra/gateway/info", { gatewayVersion: "1", codexVersion: "codex-cli 0.159.0", startedAt: "", upstreamRestarts: 0 });
    await flush();
    const thread = (id: string, cwd: string, updatedAt: number) => ({
      id,
      cwd,
      name: id,
      preview: "",
      updatedAt,
      status: { type: "idle" },
    });
    const lists = () => sent.filter((message) => message.method === "thread/list");
    expect(lists()[0]?.params).toMatchObject({ useStateDbOnly: true });
    replyTo("thread/list", { data: [thread("a", "/one", 2)], nextCursor: null, backwardsCursor: null });
    await flush();
    expect(state.folders?.flatMap((folder) => folder.threads.map((item) => item.id))).toEqual(["a"]);

    // The scan repeats a thread across pages; it must appear once.
    expect(lists()[1]?.params).toMatchObject({ useStateDbOnly: false });
    replyTo("thread/list", { data: [thread("a", "/one", 2), thread("old", "/one", 1)], nextCursor: "p2", backwardsCursor: null });
    await flush();
    replyTo("thread/list", { data: [thread("old", "/one", 1), thread("c", "/three", 3)], nextCursor: null, backwardsCursor: null });
    await flush();
    expect(state.folders?.map((folder) => [folder.cwd, folder.threads.map((item) => item.id)])).toEqual([
      ["/three", ["c"]],
      ["/one", ["a", "old"]],
    ]);
  });

  it("keeps notifications for the newly opened thread when an older resume finishes first", async () => {
    const { session, sent, state, receive, emitState } = harness();
    emitState("connected");
    void session.openThread("A");
    void session.openThread("B");
    await flush();
    const resumes = sent.filter((message) => message.method === "thread/resume");
    const thread = (id: string) => ({ thread: { id, name: id, preview: "", cwd: "/w", status: { type: "idle" }, turns: [{ id: "t", status: "inProgress", error: null, items: [{ type: "agentMessage", id: "m", text: "", phase: null, memoryCitation: null, delivery: null, questions: null }] }] } });
    // The stale resume for A completes first.
    receive({ jsonrpc: "2.0", id: resumes[0]?.id, result: thread("A") });
    await flush();
    // A delta for B arrives before B's resume response; it must not be lost.
    receive({ jsonrpc: "2.0", method: "item/agentMessage/delta", params: { threadId: "B", turnId: "t", itemId: "m", delta: "hi" } });
    receive({ jsonrpc: "2.0", id: resumes[1]?.id, result: thread("B") });
    await flush();
    const item = state.transcript?.turns[0]?.items[0];
    expect(state.transcript?.threadId).toBe("B");
    expect(item?.type === "agentMessage" && item.text).toBe("hi");
  });

  it("labels write-to-stdin approvals separately from new commands", () => {
    const { state, receive } = harness();
    receive({ jsonrpc: "2.0", id: 3, method: "item/commandExecution/requestApproval", params: { threadId: "t1", turnId: "x", itemId: "i", kind: "writeStdin" } });
    expect(state.pendingApprovals?.[0]?.kind).toBe("writeStdin");
  });
});

describe("paged thread loading", () => {
  const turn = (id: string) => ({ id, items: [], itemsView: "full", status: "completed", error: null, startedAt: null, completedAt: null, durationMs: null });

  it("resumes without turns, reads the newest page, then prepends older pages", async () => {
    const { session, sent, state, replyTo, emitState } = harness();
    emitState("connected");
    const opened = session.openThread("big");
    await flush();
    const resume = sent.find((message) => message.method === "thread/resume");
    expect(resume?.params).toMatchObject({ threadId: "big", excludeTurns: true });
    replyTo("thread/resume", { thread: { id: "big", name: "big", preview: "", cwd: "/w", status: { type: "idle" }, turns: [] } });
    await flush();
    const first = [...sent].reverse().find((message) => message.method === "thread/turns/list");
    expect(first?.params).toMatchObject({ threadId: "big", sortDirection: "desc", itemsView: "full" });
    expect(first?.params).not.toHaveProperty("cursor");
    replyTo("thread/turns/list", { data: [turn("t3"), turn("t2")], nextCursor: "older", backwardsCursor: null });
    await opened;
    expect(state.transcript?.turns.map((item) => item.id)).toEqual(["t2", "t3"]);
    expect(state.transcript?.olderTurnsCursor).toBe("older");

    const older = session.loadOlderTurns();
    await flush();
    expect([...sent].reverse().find((message) => message.method === "thread/turns/list")?.params).toMatchObject({ cursor: "older" });
    replyTo("thread/turns/list", { data: [turn("t1")], nextCursor: null, backwardsCursor: null });
    await older;
    expect(state.transcript?.turns.map((item) => item.id)).toEqual(["t1", "t2", "t3"]);
    expect(state.transcript?.olderTurnsCursor).toBeNull();
  });

  it("treats a turn-list error for a brand-new thread as an empty history", async () => {
    const { session, state, receive, sent, replyTo, emitState } = harness();
    emitState("connected");
    const opened = session.openThread("fresh");
    await flush();
    replyTo("thread/resume", { thread: { id: "fresh", name: null, preview: "", cwd: "/w", status: { type: "idle" }, turns: [] } });
    await flush();
    const list = [...sent].reverse().find((message) => message.method === "thread/turns/list");
    receive({ jsonrpc: "2.0", id: list?.id, error: { code: -32600, message: "no rollout found" } });
    await opened;
    expect(state.transcript?.turns).toEqual([]);
    expect(state.transcriptError ?? null).toBeNull();
  });
});

describe("turn list errors on an existing thread", () => {
  it("shows the error instead of an empty history", async () => {
    const { session, state, receive, sent, replyTo, emitState } = harness();
    emitState("connected");
    const opened = session.openThread("old");
    await flush();
    replyTo("thread/resume", { thread: { id: "old", name: "old", preview: "", cwd: "/w", status: { type: "idle" }, turns: [] } });
    await flush();
    const list = [...sent].reverse().find((message) => message.method === "thread/turns/list");
    receive({ jsonrpc: "2.0", id: list?.id, error: { code: -32603, message: "internal error" } });
    await opened;
    expect(state.transcript ?? null).toBeNull();
  });
});

describe("a thread whose writer lock another Codex process holds", () => {
  const turn = (id: string) => ({ id, items: [], itemsView: "full", status: "completed", error: null, startedAt: null, completedAt: null, durationMs: null });
  const thread = (id: string) => ({ id, name: id, preview: "", cwd: "/w", status: { type: "idle" }, turns: [] });
  const lastId = (sent: Sent[], method: string) => [...sent].reverse().find((message) => message.method === method)?.id;

  it("reads it without subscribing, refuses to send, and subscribes again on reconnect", async () => {
    const { session, sent, state, receive, replyTo, emitState } = harness();
    emitState("connected");
    const opened = session.openThread("locked");
    await flush();
    receive({ jsonrpc: "2.0", id: lastId(sent, "thread/resume"), error: { code: -32603, message: "thread locked already has an active writer" } });
    await flush();
    expect(sent.find((message) => message.method === "thread/read")?.params).toMatchObject({ threadId: "locked", includeTurns: false });
    replyTo("thread/read", { thread: thread("locked") });
    await flush();
    replyTo("thread/turns/list", { data: [turn("t1")], nextCursor: null, backwardsCursor: null });
    await opened;
    expect(state.transcript?.turns.map((item) => item.id)).toEqual(["t1"]);
    expect(state.transcriptError ?? null).toBeNull();
    expect(state.writerElsewhere).toBe(true);

    await expect(session.sendMessage("hello")).rejects.toThrow("다른 Codex 앱");
    expect(sent.some((message) => message.method === "turn/start")).toBe(false);

    const reopened = session.reopenThread();
    await flush();
    replyTo("thread/resume", { thread: thread("locked") });
    await flush();
    replyTo("thread/turns/list", { data: [turn("t2"), turn("t1")], nextCursor: null, backwardsCursor: null });
    await reopened;
    expect(state.writerElsewhere).toBe(false);
    expect(state.transcript?.turns.map((item) => item.id)).toEqual(["t1", "t2"]);
    const sending = session.sendMessage("hello");
    await flush();
    expect(sent.at(-1)).toMatchObject({ method: "turn/start", params: { threadId: "locked" } });
    replyTo("turn/start", {});
    await sending;
  });

  it("shows other resume errors instead of reading", async () => {
    const { session, sent, state, receive, emitState } = harness();
    emitState("connected");
    const opened = session.openThread("broken");
    await flush();
    receive({ jsonrpc: "2.0", id: lastId(sent, "thread/resume"), error: { code: -32603, message: "internal error" } });
    await opened;
    expect(sent.some((message) => message.method === "thread/read")).toBe(false);
    expect(state.transcriptError).toBe("internal error");
  });

  it("does not unsubscribe from a read-only thread when another thread opens", async () => {
    const { session, sent, receive, replyTo, emitState } = harness();
    emitState("connected");
    const opened = session.openThread("locked");
    await flush();
    receive({ jsonrpc: "2.0", id: lastId(sent, "thread/resume"), error: { code: -32603, message: "thread locked already has an active writer" } });
    await flush();
    replyTo("thread/read", { thread: thread("locked") });
    await flush();
    replyTo("thread/turns/list", { data: [], nextCursor: null, backwardsCursor: null });
    await opened;
    void session.openThread("other");
    await flush();
    expect(sent.some((message) => message.method === "thread/unsubscribe")).toBe(false);
  });
});

describe("status changes while listing threads", () => {
  it("prefer the newer status notification over the listed status", async () => {
    const { state, receive, replyTo, emitState } = harness();
    emitState("connected");
    await flush();
    replyTo("initialize", {});
    await flush();
    replyTo("akra/gateway/info", { gatewayVersion: "1", codexVersion: null, startedAt: "", upstreamRestarts: 0 });
    await flush();
    // While the list request is pending, thread "a" finishes and thread "b" starts.
    receive({ jsonrpc: "2.0", method: "thread/status/changed", params: { threadId: "a", status: { type: "idle" } } });
    receive({ jsonrpc: "2.0", method: "thread/status/changed", params: { threadId: "b", status: { type: "active", activeFlags: [] } } });
    const listed = (id: string, active: boolean) => ({ id, cwd: "/w", name: id, preview: "", updatedAt: 1, status: active ? { type: "active", activeFlags: [] } : { type: "idle" } });
    replyTo("thread/list", { data: [listed("a", true), listed("b", false)], nextCursor: null, backwardsCursor: null });
    await flush();
    expect(state.activeThreadIds).toEqual(["b"]);
  });
});

describe("thread lifecycle notifications from other windows", () => {
  it("adds started threads, renames, removes archived or deleted threads, and restores unarchived ones without a new list", async () => {
    const { state, receive, replyTo, emitState, sent } = harness();
    emitState("connected");
    await flush();
    replyTo("initialize", {});
    await flush();
    replyTo("akra/gateway/info", { gatewayVersion: "1", codexVersion: null, startedAt: "", upstreamRestarts: 0 });
    await flush();
    const thread = (id: string, cwd: string, updatedAt: number, extra: object = {}) => ({
      id, cwd, name: null, preview: id, updatedAt, ephemeral: false, parentThreadId: null, status: { type: "idle" }, ...extra,
    });
    replyTo("thread/list", { data: [thread("a", "/w", 2), thread("b", "/w", 1)], nextCursor: null, backwardsCursor: null });
    await flush();
    const listed = () => state.folders?.map((folder) => [folder.cwd, folder.threads.map((item) => `${item.id}:${item.title}`)]);
    const notify = (method: string, params: object) => receive({ jsonrpc: "2.0", method, params });

    notify("thread/started", { thread: thread("c", "/new", 3) });
    notify("thread/started", { thread: thread("sub", "/new", 4, { parentThreadId: "c" }) });
    notify("thread/started", { thread: thread("tmp", "/new", 4, { ephemeral: true }) });
    notify("thread/name/updated", { threadId: "a", threadName: "Renamed" });
    expect(listed()).toEqual([["/new", ["c:c"]], ["/w", ["a:Renamed", "b:b"]]]);

    notify("thread/archived", { threadId: "b" });
    notify("thread/deleted", { threadId: "c" });
    expect(listed()).toEqual([["/w", ["a:Renamed"]]]);

    notify("thread/unarchived", { threadId: "b" });
    await flush();
    expect(sent.at(-1)).toMatchObject({ method: "thread/read", params: { threadId: "b", includeTurns: false } });
    replyTo("thread/read", { thread: thread("b", "/w", 1) });
    await flush();
    expect(listed()).toEqual([["/w", ["a:Renamed", "b:b"]]]);
  });
});

describe("thread lifecycle notifications while listing threads", () => {
  it("keeps a thread deleted or renamed during the list request out of, or renamed in, both list results", async () => {
    const { state, receive, replyTo, emitState } = harness();
    emitState("connected");
    await flush();
    replyTo("initialize", {});
    await flush();
    replyTo("akra/gateway/info", { gatewayVersion: "1", codexVersion: null, startedAt: "", upstreamRestarts: 0 });
    await flush();
    // While the state-DB list request is pending, "gone" is deleted and "kept" is renamed.
    receive({ jsonrpc: "2.0", method: "thread/deleted", params: { threadId: "gone" } });
    receive({ jsonrpc: "2.0", method: "thread/name/updated", params: { threadId: "kept", threadName: "Renamed" } });
    const thread = (id: string) => ({ id, cwd: "/w", name: id, preview: "", updatedAt: 1, status: { type: "idle" } });
    const listed = () => state.folders?.flatMap((folder) => folder.threads.map((item) => `${item.id}:${item.title}`));
    replyTo("thread/list", { data: [thread("gone"), thread("kept")], nextCursor: null, backwardsCursor: null });
    await flush();
    expect(listed()).toEqual(["kept:Renamed"]);
    // The rollout scan still finds the deleted thread; it must not come back.
    replyTo("thread/list", { data: [thread("gone"), thread("kept")], nextCursor: null, backwardsCursor: null });
    await flush();
    expect(listed()).toEqual(["kept:Renamed"]);
  });
});

describe("threads added while listing threads", () => {
  it("keeps a thread started or unarchived during the first list on a host without a rollout scan", async () => {
    const { state, receive, replyTo, emitState } = harness(true);
    emitState("connected");
    await flush();
    replyTo("initialize", {});
    await flush();
    replyTo("akra/gateway/info", { gatewayVersion: "1", codexVersion: null, startedAt: "", upstreamRestarts: 0 });
    await flush();
    const thread = (id: string, updatedAt: number) => ({ id, cwd: "/w", name: id, preview: "", updatedAt, ephemeral: false, parentThreadId: null, status: { type: "idle" } });
    // While the list request is pending, "new" starts and "back" is unarchived.
    receive({ jsonrpc: "2.0", method: "thread/started", params: { thread: thread("new", 3) } });
    receive({ jsonrpc: "2.0", method: "thread/unarchived", params: { threadId: "back" } });
    await flush();
    replyTo("thread/read", { thread: thread("back", 2) });
    await flush();
    replyTo("thread/list", { data: [thread("old", 1)], nextCursor: null, backwardsCursor: null });
    await flush();
    expect(state.folders?.flatMap((folder) => folder.threads.map((item) => item.id))).toEqual(["new", "back", "old"]);
  });
});

describe("forking a conversation (#2625)", () => {
  const turn = (id: string, status = "completed") => ({ id, items: [], itemsView: "full", status, error: null, startedAt: null, completedAt: null, durationMs: null });
  const thread = (id: string, extra: object = {}) => ({ id, name: id, preview: "", cwd: "/w", updatedAt: 1, ephemeral: false, parentThreadId: null, forkedFromId: null, threadSource: null, status: { type: "idle" }, turns: [], ...extra });
  const lastId = (sent: Sent[], method: string) => [...sent].reverse().find((message) => message.method === method)?.id;
  const sentFor = (sent: Sent[], threadId: string) => sent.filter((message) => message.params?.threadId === threadId).map((message) => message.method);

  /** 다른 Codex 앱이 쓰기 잠금을 가진 "locked" 를 읽기 전용으로 연다. */
  async function openLocked() {
    const h = harness();
    h.emitState("connected");
    const opened = h.session.openThread("locked");
    await flush();
    h.receive({ jsonrpc: "2.0", id: lastId(h.sent, "thread/resume"), error: { code: -32603, message: "thread locked already has an active writer" } });
    await flush();
    h.replyTo("thread/read", { thread: thread("locked") });
    await flush();
    h.replyTo("thread/turns/list", { data: [turn("t1")], nextCursor: null, backwardsCursor: null });
    await opened;
    h.sent.length = 0;
    return h;
  }

  /** 원본 읽기와 경계 읽기에 답한다. 그다음 요청은 thread/fork 이다. */
  async function answerBoundary(h: Awaited<ReturnType<typeof openLocked>>, turns: object[], settle: () => Promise<unknown> = flush) {
    await settle();
    h.replyTo("thread/read", { thread: thread("locked") });
    await settle();
    h.replyTo("thread/turns/list", { data: turns, nextCursor: null, backwardsCursor: null });
    await settle();
  }

  it.each(["resume", "writer-elsewhere", "read-only"])("recovers a listed fork's origin when opening through %s after a fresh session", async (mode) => {
    const h = harness(mode === "read-only");
    const listed = { id: "child", title: "Newer title", updatedAt: 2, branch: null, failed: true };
    h.state.folders = [{ cwd: "/w", threads: [listed] }];
    h.emitState("connected");
    const opening = h.session.openThread("child");
    await flush();
    if (mode === "writer-elsewhere") {
      h.receive({ jsonrpc: "2.0", id: lastId(h.sent, "thread/resume"), error: { code: -32603, message: "thread child already has an active writer" } });
      await flush();
    }
    h.replyTo(mode === "resume" ? "thread/resume" : "thread/read", {
      thread: thread("child", { forkedFromId: "source", turns: [turn("inherited")] }),
    });
    await opening;
    expect(h.state.folders?.[0]?.threads).toEqual([{ ...listed, forkedFromId: "source" }]);
    // A later list response has no lineage; it must not clear the recovered origin or overwrite newer UI state.
    h.state.folders = mergeWorkspaceFolders(h.state.folders ?? [], [{ cwd: "/w", threads: [{ ...listed, title: "Stale title", failed: false }] }]);
    expect(h.state.folders?.[0]?.threads).toEqual([{ ...listed, forkedFromId: "source" }]);
    expect(listed).not.toHaveProperty("forkedFromId");
    expect(h.sent.some((request) => request.method === "thread/fork" || request.method?.startsWith("turn/"))).toBe(false);
    h.session.stop();
  });

  it.each([
    ["resume", "thread/archived"],
    ["resume", "thread/deleted"],
    ["writer-elsewhere", "thread/archived"],
    ["writer-elsewhere", "thread/deleted"],
    ["read-only", "thread/archived"],
    ["read-only", "thread/deleted"],
  ])("keeps a fork removed when opening through %s receives %s before its response", async (mode, method) => {
    const h = harness(mode === "read-only");
    const listed = { id: "child", title: "Child", updatedAt: 2, branch: null, failed: false };
    const other = { ...listed, id: "other", title: "Other" };
    h.state.folders = [{ cwd: "/w", threads: [listed, other] }];
    h.emitState("connected");
    const opening = h.session.openThread("child");
    await flush();
    if (mode === "writer-elsewhere") {
      h.receive({ jsonrpc: "2.0", id: lastId(h.sent, "thread/resume"), error: { code: -32603, message: "thread child already has an active writer" } });
      await flush();
    }
    h.receive({ jsonrpc: "2.0", method, params: { threadId: "child" } });
    expect(h.state.folders?.[0]?.threads).toEqual([other]);
    // Removing the sidebar entry does not navigate away or invalidate the pending open.
    expect(h.state.openThreadId).toBe("child");
    h.replyTo(mode === "resume" ? "thread/resume" : "thread/read", {
      thread: thread("child", { forkedFromId: "source", turns: [turn("inherited")] }),
    });
    await opening;
    expect(h.state.folders?.[0]?.threads).toEqual([other]);
    expect(h.sent.some((request) => request.method === "thread/fork" || request.method?.startsWith("turn/"))).toBe(false);
    h.session.stop();
  });

  it("does not recover lineage from a thread-open response after navigating away", async () => {
    const h = harness();
    h.state.folders = [{ cwd: "/w", threads: [{ id: "child", title: "Child", updatedAt: 1, branch: null, failed: false }] }];
    h.emitState("connected");
    const stale = h.session.openThread("child");
    const staleRequest = lastId(h.sent, "thread/resume");
    const current = h.session.openThread("other");
    h.receive({ jsonrpc: "2.0", id: staleRequest, result: { thread: thread("child", { forkedFromId: "source", turns: [turn("old")] }) } });
    await stale;
    h.replyTo("thread/resume", { thread: thread("other", { turns: [turn("current")] }) });
    await current;
    expect(h.state.openThreadId).toBe("other");
    expect(h.state.folders?.[0]?.threads[0]).not.toHaveProperty("forkedFromId");
    h.session.stop();
  });

  it("forks a locked thread through its newest finished turn and opens the fork without touching the source", async () => {
    const h = await openLocked();
    const claim = vi.fn(() => true);
    const forking = h.session.forkThread("locked", claim);
    await answerBoundary(h, [turn("t3", "inProgress"), turn("t2")]);
    expect(h.sent.find((message) => message.method === "thread/turns/list")?.params).toMatchObject({ threadId: "locked", limit: 2, sortDirection: "desc" });
    // The other app's running turn t3 is excluded: the boundary is pinned to t2.
    expect(h.sent.at(-1)).toMatchObject({ method: "thread/fork", params: { threadId: "locked", lastTurnId: "t2", excludeTurns: true, ephemeral: false } });
    h.replyTo("thread/fork", { thread: thread("child", { forkedFromId: "locked" }) });
    await flush();
    h.replyTo("thread/turns/list", { data: [turn("t2"), turn("t1")], nextCursor: null, backwardsCursor: null });
    await expect(forking).resolves.toEqual({ kind: "opened", threadId: "child" });

    expect(claim).toHaveBeenCalledWith("child");
    expect(h.state.openThreadId).toBe("child");
    expect(h.state.writerElsewhere).toBe(false);
    expect(h.state.transcript?.turns.map((item) => item.id)).toEqual(["t1", "t2"]);
    expect(h.state.folders?.flatMap((folder) => folder.threads).find((item) => item.id === "child")?.forkedFromId).toBe("locked");
    // The source gets only reads; the fork is already subscribed, so it gets no resume.
    expect(sentFor(h.sent, "locked")).toEqual(["thread/read", "thread/turns/list", "thread/fork"]);
    expect(sentFor(h.sent, "child")).toEqual(["thread/turns/list"]);

    // A duplicate thread/started notification does not add the fork twice.
    h.receive({ jsonrpc: "2.0", method: "thread/started", params: { thread: thread("child", { forkedFromId: "locked" }) } });
    expect(h.state.folders?.flatMap((folder) => folder.threads).filter((item) => item.id === "child")).toHaveLength(1);
    const sending = h.session.sendMessage("hello");
    await flush();
    expect(h.sent.at(-1)).toMatchObject({ method: "turn/start", params: { threadId: "child" } });
    h.replyTo("turn/start", {});
    await sending;
  });

  it("sends one fork for repeated clicks on the same source", async () => {
    const h = await openLocked();
    const first = h.session.forkThread("locked", () => true);
    const second = h.session.forkThread("locked", () => true);
    expect(second).toBe(first);
    await answerBoundary(h, [turn("t1")]);
    h.replyTo("thread/fork", { thread: thread("child", { forkedFromId: "locked" }) });
    await flush();
    h.replyTo("thread/turns/list", { data: [], nextCursor: null, backwardsCursor: null });
    await first;
    expect(h.sent.filter((message) => message.method === "thread/fork")).toHaveLength(1);
    expect(h.sent.filter((message) => message.method === "thread/read")).toHaveLength(1);
  });

  it("keeps the screen and only lists the fork when the user moved away, and ends the subscription the fork made", async () => {
    const h = await openLocked();
    const forking = h.session.forkThread("locked", () => false);
    await answerBoundary(h, [turn("t1")]);
    const transcriptBefore = h.state.transcript;
    h.replyTo("thread/fork", { thread: thread("child", { forkedFromId: "locked" }) });
    await expect(forking).resolves.toEqual({ kind: "created", threadId: "child" });
    expect(h.state.openThreadId).toBe("locked");
    expect(h.state.transcript).toBe(transcriptBefore);
    expect(h.state.writerElsewhere).toBe(true);
    expect(h.state.folders?.flatMap((folder) => folder.threads).map((item) => item.id)).toContain("child");
    expect(sentFor(h.sent, "child")).toEqual(["thread/unsubscribe"]);
    expect(sentFor(h.sent, "locked")).not.toContain("thread/unsubscribe");
  });

  it("keeps the fork's subscription when the user already opened the fork", async () => {
    const h = await openLocked();
    const claim = vi.fn(() => true);
    const forking = h.session.forkThread("locked", claim);
    await answerBoundary(h, [turn("t1")]);
    const forkRequest = lastId(h.sent, "thread/fork");
    void h.session.openThread("child");
    await flush();
    h.receive({ jsonrpc: "2.0", id: forkRequest, result: { thread: thread("child", { forkedFromId: "locked" }) } });
    await expect(forking).resolves.toEqual({ kind: "created", threadId: "child" });
    expect(claim).not.toHaveBeenCalled();
    expect(sentFor(h.sent, "child")).not.toContain("thread/unsubscribe");
  });

  it("reports an unknown result after a dropped connection and never retries by itself", async () => {
    const h = await openLocked();
    const forking = h.session.forkThread("locked", () => true);
    await answerBoundary(h, [turn("t1")]);
    h.emitState("disconnected");
    const outcome = await forking;
    expect(outcome.kind).toBe("unknown");
    expect(h.session.forkOutcomeUnknown("locked")).toBe(true);
    expect(h.sent.filter((message) => message.method === "thread/fork")).toHaveLength(1);
    expect(h.state.openThreadId).toBe("locked");
  });

  it("reports an unknown result when the fork response does not come in time, and only lists a late success", async () => {
    const h = await openLocked();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const settle = () => vi.advanceTimersByTimeAsync(0);
      const claim = vi.fn(() => true);
      const forking = h.session.forkThread("locked", claim);
      await answerBoundary(h, [turn("t1")], settle);
      const lostRequest = lastId(h.sent, "thread/fork");
      const transcriptBefore = h.state.transcript;
      await vi.advanceTimersByTimeAsync(FORK_RESPONSE_TIMEOUT_MS);
      const outcome = await forking;
      expect(outcome).toMatchObject({ kind: "unknown" });
      expect(outcome.kind === "unknown" && outcome.reason).toContain("사이드바");
      // The screen asks before a second fork of this source.
      expect(h.session.forkOutcomeUnknown("locked")).toBe(true);
      expect(h.sent.filter((message) => message.method === "thread/fork")).toHaveLength(1);

      // A late success after the timeout only adds the fork to the sidebar and ends its subscription.
      h.receive({ jsonrpc: "2.0", id: lostRequest, result: { thread: thread("child", { forkedFromId: "locked" }) } });
      await settle();
      expect(claim).not.toHaveBeenCalled();
      expect(h.state.openThreadId).toBe("locked");
      expect(h.state.transcript).toBe(transcriptBefore);
      expect(h.state.writerElsewhere).toBe(true);
      expect(h.state.folders?.flatMap((folder) => folder.threads).map((item) => item.id)).toContain("child");
      expect(sentFor(h.sent, "child")).toEqual(["thread/unsubscribe"]);
      expect(h.sent.filter((message) => message.method === "thread/fork")).toHaveLength(1);

      // After the user confirms, the next fork is a new request, not the lost one.
      const retry = h.session.forkThread("locked", () => false);
      expect(retry).not.toBe(forking);
      await answerBoundary(h, [turn("t1")], settle);
      expect(h.sent.filter((message) => message.method === "thread/fork")).toHaveLength(2);
      expect(h.session.forkOutcomeUnknown("locked")).toBe(false);
      h.replyTo("thread/fork", { thread: thread("child-2", { forkedFromId: "locked" }) });
      await expect(retry).resolves.toEqual({ kind: "created", threadId: "child-2" });
    } finally {
      vi.useRealTimers();
    }
  });

  it("refuses without creating anything when the server rejects the fork or no turn has finished", async () => {
    const h = await openLocked();
    const rejected = h.session.forkThread("locked", () => true);
    await answerBoundary(h, [turn("t1")]);
    h.receive({ jsonrpc: "2.0", id: lastId(h.sent, "thread/fork"), error: { code: -32600, message: "turn t1 is in progress" } });
    expect(await rejected).toMatchObject({ kind: "refused" });
    expect(h.session.forkOutcomeUnknown("locked")).toBe(false);
    expect(h.state.openThreadId).toBe("locked");

    h.sent.length = 0;
    const empty = h.session.forkThread("locked", () => true);
    await answerBoundary(h, [turn("t9", "inProgress")]);
    expect(await empty).toEqual({ kind: "refused", reason: "끝난 턴이 없는 대화는 분기할 수 없습니다." });
    expect(h.sent.some((message) => message.method === "thread/fork")).toBe(false);
  });

  it("does not fork remote, temporary, sub-agent, dot, or browser conversations", async () => {
    const remote = harness(true);
    remote.emitState("connected");
    expect(await remote.session.forkThread("t1", () => true)).toMatchObject({ kind: "refused" });
    expect(remote.sent.some((message) => message.method === "thread/fork")).toBe(false);

    const blocked = { id: "t1", ephemeral: false, parentThreadId: null, threadSource: null };
    expect(forkBlockReason(blocked, null)).toBeNull();
    expect(forkBlockReason({ ...blocked, ephemeral: true }, null)).not.toBeNull();
    expect(forkBlockReason({ ...blocked, parentThreadId: "parent" }, null)).not.toBeNull();
    expect(forkBlockReason({ ...blocked, threadSource: "aeon_child" as never }, null)).not.toBeNull();
    const browserTurn = { ...transcript("completed"), turns: [{ ...transcript("completed").turns[0]!, items: [{ type: "mcpToolCall", id: "i", server: "akra_browser" } as never] }] };
    expect(forkBlockReason(blocked, browserTurn)).toContain("브라우저");
  });
});

describe("manual compaction and session input exclusion", () => {
  const compactItem = { type: "contextCompaction", id: "recovered-item" };
  const compactTurn = { id: "compact-turn", status: "completed", items: [compactItem], error: null };
  const resumed = (id = "t1", turns: object[] = [compactTurn]) => ({ thread: {
    id, name: "recovered", preview: "", cwd: "/work/example-app", status: { type: "idle" }, turns,
  } });
  it("blocks start, steer and queued dispatch until the matching compact turn succeeds", async () => {
    const h = harness(); h.emitState("connected"); h.openWith(transcript("completed"));
    h.session.compaction.start("t1");
    h.replyTo("thread/compact/start", {});
    await expect(h.session.sendMessage("blocked start")).rejects.toThrow("문맥 압축");
    expect(() => h.session.queueMessage("blocked reservation")).toThrow("문맥 압축");
    h.session.nextTurnQueue.enqueue("t1", "held reservation", null);
    expect(() => h.session.resumeQueuedMessages()).toThrow();
    const item = { type: "contextCompaction", id: "compact-item" };
    h.receive({ method: "turn/started", params: { threadId: "t1", turn: { id: "compact-turn", status: "inProgress", items: [], error: null } } });
    await expect(h.session.sendMessage("blocked steer")).rejects.toThrow("문맥 압축");
    expect(h.sent.some(message => message.method === "turn/start" || message.method === "turn/steer")).toBe(false);
    h.receive({ method: "item/completed", params: { threadId: "t1", turnId: "compact-turn", item } });
    h.receive({ method: "turn/completed", params: { threadId: "t1", turn: { id: "compact-turn", status: "completed", items: [item], error: null } } });
    expect(h.session.compaction.snapshot("t1").phase).toBe("completed");
    h.session.resumeQueuedMessages();
    expect(h.sent.filter(message => message.method === "turn/start")).toHaveLength(1);
    h.replyTo("turn/start", { turn: { id: "reserved-turn" } });
    await flush(); h.session.stop();
  });

  it("rejects compaction while a normal start response or its acknowledged turn is outstanding", async () => {
    const h = harness(); h.emitState("connected"); h.openWith(transcript("completed"));
    const sending = h.session.sendMessage("first");
    expect(() => h.session.compaction.start("t1")).toThrow("전송 중");
    h.replyTo("turn/start", { turn: { id: "new-turn" } }); await sending;
    // 아직 transcript 프레임에 시작 알림이 없더라도 A06의 waitingTurnId가 차단한다.
    expect(() => h.session.compaction.start("t1")).toThrow("전송 중");
    h.receive({ method: "turn/completed", params: { threadId: "t1", turn: { id: "new-turn", status: "completed", items: [], error: null } } });
    h.session.compaction.start("t1");
    expect(h.sent.filter(message => message.method === "thread/compact/start")).toHaveLength(1);
    h.session.stop();
  });

  it("keeps an uncertain compaction blocking after transport loss without another model request", async () => {
    const h = harness(); h.emitState("connected"); h.openWith(transcript("completed"));
    h.session.compaction.start("t1"); h.emitState("disconnected");
    await flush();
    expect(h.session.compaction.snapshot("t1").phase).toBe("unknown");
    expect(h.session.compaction.isBlocking("t1")).toBe(true);
    expect(h.sent.filter(message => message.method === "thread/compact/start")).toHaveLength(1);
    h.session.stop();
  });

  it("does not treat a previous connection's paused waiting ID as a current pending turn", async () => {
    const h = harness(); h.emitState("connected"); h.openWith(transcript("completed"));
    const sending = h.session.sendMessage("old connection");
    h.replyTo("turn/start", { turn: { id: "unobserved-finish" } }); await sending;
    h.emitState("disconnected"); h.emitState("connected");
    const reopening = h.session.reopenThread();
    h.replyTo("thread/resume", resumed("t1", [{ id: "unobserved-finish", status: "completed", items: [], error: null }]));
    await reopening;
    expect(h.session.nextTurnQueue.snapshot("t1").waitingTurnId).toBe("unobserved-finish");
    h.session.compaction.start("t1");
    expect(h.sent.filter(message => message.method === "thread/compact/start")).toHaveLength(1);
    h.session.stop();
  });

  it("reloads the actual transcript before a recovered compaction permits a new start", async () => {
    const h = harness(); h.emitState("connected"); h.openWith(transcript("completed"));
    h.session.compaction.start("t1");
    h.receive({ method: "turn/started", params: { threadId: "t1", turn: { ...compactTurn, status: "inProgress", items: [] } } });
    h.emitState("disconnected"); h.emitState("connected");
    const checking = h.session.compaction.refresh("t1");
    h.replyTo("thread/turns/list", { data: [compactTurn], nextCursor: null, backwardsCursor: null });
    await flush();
    expect(h.session.compaction.isBlocking("t1")).toBe(true);
    h.replyTo("thread/resume", resumed()); await checking;
    expect(h.state.transcript?.turns.at(-1)?.status).toBe("completed");
    expect(h.session.compaction.isBlocking("t1")).toBe(false);
    const sending = h.session.sendMessage("fresh start");
    expect(h.sent.at(-1)?.method).toBe("turn/start");
    h.replyTo("turn/start", { turn: { id: "fresh-turn" } }); await sending;
    expect(h.sent.filter(message => message.method === "thread/compact/start")).toHaveLength(1);
    h.session.stop();
  });

  it("keeps unknown if reopening swallowed an error or the user selected another conversation", async () => {
    for (const moveAway of [false, true]) {
      const h = harness(); h.emitState("connected"); h.openWith(transcript("completed"));
      h.session.compaction.start("t1");
      const checking = h.session.compaction.refresh("t1");
      h.replyTo("thread/turns/list", { data: [compactTurn, { id: "turn-1", status: "completed", items: [] }], nextCursor: null, backwardsCursor: null });
      await flush();
      const reloadId = [...h.sent].reverse().find(message => message.method === "thread/resume")?.id;
      if (moveAway) {
        const moved = h.session.openThread("other"); await flush();
        h.receive({ id: reloadId, result: resumed() });
        h.replyTo("thread/resume", resumed("other")); await moved;
      } else h.receive({ id: reloadId, error: { code: -32603, message: "synthetic reload failure" } });
      await checking;
      expect(h.session.compaction.snapshot("t1").phase).toBe("unknown");
      expect(h.session.compaction.isBlocking("t1")).toBe(true);
      if (moveAway) expect(h.state.openThreadId).toBe("other");
      else expect(h.state.transcript).toBeNull();
      h.session.stop();
    }
  });
});
