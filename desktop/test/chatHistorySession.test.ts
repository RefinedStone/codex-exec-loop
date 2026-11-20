import { afterEach, describe, expect, it } from "vitest";

import type { ConnectionState } from "../src/shared/bridge";
import { DesktopSession, type SessionBridge } from "../src/renderer/session/desktopSession";
import { desktopSessionStore, remoteHostStore } from "../src/renderer/session/desktopStore";

describe("chat history across remote session connections", () => {
  let session: DesktopSession | undefined;
  let nextStore = 0;
  afterEach(() => session?.stop());

  function startRemote() {
    const state = remoteHostStore(`chat-history-test-${nextStore++}`);
    const listeners = new Set<(phase: ConnectionState) => void>();
    const receivers = new Set<(message: string) => void>();
    const chatReplies: ({ readonly result: unknown } | { readonly error: unknown })[] = [];
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
        const response = JSON.stringify({ id: request.id, ...reply });
        for (const receive of receivers) receive(response);
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
    expect(desktop.state().chatsProblem).toBe("History temporarily unavailable");
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

  it("keeps a previously loaded list on a failed manual refresh", async () => {
    const desktop = startRemote();
    const chats = [{ id: "chat", title: "History" }];
    desktop.chatReplies.push({ result: { chats } }, { error: { code: -32603, message: "History temporarily unavailable" } });
    await desktop.connect();

    await desktop.remote.reloadChats();

    expect(desktop.state()).toMatchObject({ chats, chatsAvailable: true, chatsProblem: "History temporarily unavailable" });
  });
});
