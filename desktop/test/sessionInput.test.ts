// 입력과 승인 응답이 어떤 요청을 보내는지 시험한다. 서버 역할은 손으로 쓴 응답으로 대신한다 (턴 없음).
import { describe, expect, it } from "vitest";

import type { AkraBridge, ConnectionState } from "../src/shared/bridge";
import { DesktopSession } from "../src/renderer/session/desktopSession";
import type { DesktopState, SessionStore } from "../src/renderer/session/desktopStore";
import type { Transcript } from "../src/renderer/conversation/transcript";

interface Sent {
  id?: number | string;
  method?: string;
  params?: Record<string, unknown>;
  result?: unknown;
}

function harness() {
  const sent: Sent[] = [];
  let readyCalls = 0;
  const messageListeners = new Set<(text: string) => void>();
  const stateListeners = new Set<(state: ConnectionState) => void>();
  const bridge: AkraBridge = {
    send: (text) => sent.push(JSON.parse(text) as Sent),
    onMessage: (listener) => (messageListeners.add(listener), () => messageListeners.delete(listener)),
    onConnectionState: (listener) => (stateListeners.add(listener), () => stateListeners.delete(listener)),
    ready: () => {
      readyCalls += 1;
    },
    chooseFolder: async () => null,
  };
  const state: Partial<DesktopState> = { pendingApprovals: [], openThreadId: null, transcript: null, folders: [] };
  const store: SessionStore = {
    setConnection: () => undefined,
    setSession: (session) => Object.assign(state, session),
    setSessionError: () => undefined,
    setThreadActive: () => undefined,
    setActiveThreads: (threadIds) => Object.assign(state, { activeThreadIds: threadIds }),
    setOpenThread: (openThreadId) => Object.assign(state, { openThreadId, transcript: null }),
    setTranscript: (transcript) => Object.assign(state, { transcript }),
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
    getOpenThreadId: () => state.openThreadId ?? null,
    getTranscript: () => state.transcript ?? null,
  };
  const session = new DesktopSession(bridge, store, "test", (callback) => callback());
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
  return { session, sent, state, receive, reply, replyTo, openWith, emitState, readyCalls: () => readyCalls };
}

const transcript = (status: "inProgress" | "completed"): Transcript => ({
  threadId: "t1",
  title: "t",
  cwd: "/work/example-app",
  status: null,
  turns: [{ id: "turn-1", status, errorMessage: null, items: [] }],
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
    expect(second?.params).toMatchObject({ cursor: "page-2" });
    replyTo("thread/list", { data: [thread("b", "/two", true)], nextCursor: null, backwardsCursor: null });
    await flush();
    expect(state.folders?.map((folder) => folder.cwd).sort()).toEqual(["/one", "/two"]);
    expect(state.activeThreadIds).toEqual(["b"]);
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
