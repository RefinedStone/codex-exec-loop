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
  const messageListeners = new Set<(text: string) => void>();
  const stateListeners = new Set<(state: ConnectionState) => void>();
  const bridge: AkraBridge = {
    send: (text) => sent.push(JSON.parse(text) as Sent),
    onMessage: (listener) => (messageListeners.add(listener), () => messageListeners.delete(listener)),
    onConnectionState: (listener) => (stateListeners.add(listener), () => stateListeners.delete(listener)),
  };
  const state: Partial<DesktopState> = { pendingApprovals: [], openThreadId: null, transcript: null, folders: [] };
  const store: SessionStore = {
    setConnection: () => undefined,
    setSession: () => undefined,
    setSessionError: () => undefined,
    setThreadActive: () => undefined,
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
  return { session, sent, state, receive, reply, openWith };
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
