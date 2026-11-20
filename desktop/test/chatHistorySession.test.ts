import { afterEach, describe, expect, it } from "vitest";

import type { ConnectionState } from "../src/shared/bridge";
import { DesktopSession, type SessionBridge } from "../src/renderer/session/desktopSession";
import { desktopSessionStore, remoteHostStore } from "../src/renderer/session/desktopStore";

type Reply = { readonly result: object } | { readonly error: { readonly code: number; readonly message: string; readonly data?: object } };
const first = { id: "account-a-chat", title: "Account A", messages: [{ id: "a1", role: "user", text: "Account A body" }] };
const second = { id: "account-b-chat", title: "Account B", messages: [] };
const limited = { error: { code: -32029, message: "Limited", data: { retryAfterSeconds: 30 } } } as const;

function holdReply() {
  let release: (reply: Reply) => void = () => { throw new TypeError("Reply resolver is not initialized"); };
  const promise = new Promise<Reply>((resolve) => { release = resolve; });
  return { promise, release };
}

describe("chat history across remote session connections", () => {
  let session: DesktopSession | undefined;
  let nextStore = 0;
  afterEach(() => session?.stop());

  function startRemote() {
    const state = remoteHostStore(`chat-history-test-${nextStore++}`);
    const listeners = new Set<(phase: ConnectionState) => void>();
    const receivers = new Set<(message: string) => void>();
    const chatReplies: (Reply | Promise<Reply>)[] = [];
    const calls: string[] = [];
    const bridge: SessionBridge = {
      chatHistory: true,
      ready: () => undefined,
      onConnectionState(listener) {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      onMessage(listener) {
        receivers.add(listener);
        return () => receivers.delete(listener);
      },
      send(text) {
        const request: { readonly id?: number; readonly method: string } = JSON.parse(text);
        if (request.id === undefined) return;
        calls.push(request.method);
        let reply;
        switch (request.method) {
          case "akra/chats/list":
          case "akra/chats/read":
            reply = chatReplies.shift() ?? { error: { code: -32603, message: "Unexpected history request" } };
            break;
          case "akra/gateway/info":
            reply = { result: { codexVersion: "0.0.0" } };
            break;
          case "thread/list":
            reply = { result: { data: [], nextCursor: null } };
            break;
          case "akra/dots/list":
            reply = { result: { dots: [] } };
            break;
          default:
            reply = { result: {} };
        }
        void Promise.resolve(reply).then((answer) => {
          const response = JSON.stringify({ id: request.id, ...answer });
          for (const receive of receivers) receive(response);
        });
      },
    };
    const remote = new DesktopSession(bridge, desktopSessionStore(state), "test", undefined, true);
    session = remote;
    remote.start();
    return {
      state: state.getState,
      remote,
      chatReplies,
      historyReads: () => calls.filter((method) => method === "akra/chats/list").length,
      async connect() {
        for (const listener of listeners) {
          listener({ phase: "disconnected", endpoint: "test" });
          listener({ phase: "connected", endpoint: "test" });
        }
        await new Promise<void>((resolve) => setImmediate(resolve));
      },
    };
  }

  it("keeps a failed first list on reconnect until the user refreshes", async () => {
    const desktop = startRemote();
    desktop.chatReplies.push({ error: { code: -32603, message: "History temporarily unavailable" } }, { result: { chats: [] } });

    await desktop.connect();
    expect(desktop.state().chatsProblem).toBe("History temporarily unavailable");
    await desktop.connect();
    expect(desktop.historyReads()).toBe(1);
    expect(desktop.state().chatsAvailable).toBe(true);
    await desktop.remote.reloadChats();
    expect(desktop.state()).toMatchObject({ chats: [], chatsAvailable: true, chatsProblem: null });
  });

  it.each([
    { kind: "an empty success", reply: { result: { chats: [] } } },
    { kind: "the history rate limit", reply: { error: { code: -32029, message: "Limited", data: { retryAfterSeconds: 30 } } } },
  ])("does not automatically read again after $kind", async ({ reply }) => {
    const desktop = startRemote();
    desktop.chatReplies.push(reply);
    await desktop.connect();

    await desktop.connect();

    expect(desktop.historyReads()).toBe(1);
  });

  it("clears a previously loaded list and body on a failed manual refresh", async () => {
    const desktop = startRemote();
    desktop.chatReplies.push({ result: { chats: [first] } }, { result: first }, { error: { code: -32603, message: "History temporarily unavailable" } });
    await desktop.connect();
    await desktop.remote.readChat(first.id);

    await desktop.remote.reloadChats();

    expect(desktop.state()).toMatchObject({ chats: [], chatConversation: null, chatsAvailable: true, chatsProblem: "History temporarily unavailable" });
  });

  it("keeps an unsupported history section hidden after reconnect", async () => {
    const desktop = startRemote();
    desktop.chatReplies.push({ error: { code: -32601, message: "Method not found" } });
    await desktop.connect();
    await desktop.connect();
    expect(desktop.historyReads()).toBe(1);
    expect(desktop.state().chatsAvailable).toBe(false);
  });

  it("clears the previous connection's private history without automatically fetching it again", async () => {
    const desktop = startRemote();
    desktop.chatReplies.push({ result: { chats: [first] } }, { result: first });
    await desktop.connect();
    await desktop.remote.readChat(first.id);
    expect(desktop.state().chatConversation).toEqual(first);
    await desktop.connect();
    expect(desktop.state()).toMatchObject({ chats: [], chatConversation: null, chatsAvailable: true });
    expect(desktop.historyReads()).toBe(1);
  });

  it("keeps the selected conversation when a superseded read fails", async () => {
    const desktop = startRemote();
    const delayed = holdReply();
    desktop.chatReplies.push({ result: { chats: [first, second] } }, delayed.promise, { result: second });
    await desktop.connect();
    const superseded = desktop.remote.readChat(first.id);
    await desktop.remote.readChat(second.id);
    delayed.release(limited);
    await superseded;
    expect(desktop.state()).toMatchObject({ chats: [first, second], chatConversation: second, chatsProblem: null });
  });

  it.each(["list", "read"] as const)("ignores an old reply to the other request after an uncached %s error", async (failing) => {
    const desktop = startRemote();
    desktop.chatReplies.push({ result: { chats: [first] } }, { result: first });
    await desktop.connect();
    await desktop.remote.readChat(first.id);
    expect(desktop.state().chatConversation).toEqual(first);
    const old = holdReply();
    desktop.chatReplies.push(old.promise, limited);
    const pending = failing === "list" ? desktop.remote.readChat(first.id) : desktop.remote.reloadChats();

    await (failing === "list" ? desktop.remote.reloadChats() : desktop.remote.readChat(first.id));
    desktop.chatReplies.push({ result: { chats: [second] } }, { result: second });
    await desktop.remote.reloadChats();
    await desktop.remote.readChat(second.id);
    old.release(failing === "list" ? { result: first } : { result: { chats: [first] } });
    await pending;

    expect(desktop.state()).toMatchObject({ chats: [second], chatConversation: second, chatsProblem: null });
  });

  it("discards an open body and its in-flight refresh when a successful list omits its ID", async () => {
    const desktop = startRemote();
    desktop.chatReplies.push({ result: { chats: [first] } }, { result: first });
    await desktop.connect();
    await desktop.remote.readChat(first.id);
    expect(desktop.state().chatConversation).toEqual(first);
    const old = holdReply();
    desktop.chatReplies.push(old.promise, { result: { chats: [second] } });
    const pending = desktop.remote.readChat(first.id);

    await desktop.remote.reloadChats();
    old.release({ result: first });
    await pending;

    expect(desktop.state()).toMatchObject({ chats: [second], chatConversation: null });
  });
});
