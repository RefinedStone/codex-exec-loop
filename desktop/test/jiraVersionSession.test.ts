import { afterEach, describe, expect, it, vi } from "vitest";

import type { ConnectionState, JiraBridge, JiraStatus } from "../src/shared/bridge";
import { DesktopSession, remoteHostBridge, type SessionBridge } from "../src/renderer/session/desktopSession";
import { desktopSessionStore, remoteHostStore } from "../src/renderer/session/desktopStore";

interface Message {
  readonly id?: string | number;
  readonly method?: string;
  readonly params?: unknown;
  readonly result?: unknown;
  readonly error?: { readonly code: number };
}

const cleanups: Array<() => void> = [];
afterEach(() => cleanups.splice(0).forEach((cleanup) => cleanup()));

function deferred<T>() {
  let resolve = (_value: T): void => { throw new Error("Promise is not initialized."); };
  let reject = (_error: Error): void => { throw new Error("Promise is not initialized."); };
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function harness(readOnly = false) {
  let status: JiraStatus = { connected: true, siteUrl: "https://jira.example/jira", deployment: "server", email: null };
  const changed = new Set<() => void>();
  const states = new Set<(state: ConnectionState) => void>();
  const messages = new Set<(text: string) => void>();
  const sent: Message[] = [];
  let providerError: number | undefined;
  const observers = new Set<(message: Message) => void>();
  const currentVersion = vi.fn<JiraBridge["currentVersion"]>(async () => ({ version: "2026-11-17T00:00:00.000Z" }));
  const jira: JiraBridge = {
    status: async () => status,
    onChanged: (listener) => (changed.add(listener), () => changed.delete(listener)),
    currentVersion,
    connect: async () => status,
    disconnect: async () => ({ connected: false }),
    summary: async () => ({ siteUrl: "", issues: [] }),
    issuePrompt: async () => "",
    openExternal: () => undefined,
  };
  const receive = (message: Message) => messages.forEach((listener) => listener(JSON.stringify({ jsonrpc: "2.0", ...message })));
  const bridge: SessionBridge = {
    jira,
    send: (text) => {
      const message: Message = JSON.parse(text);
      sent.push(message);
      observers.forEach((observer) => observer(message));
      if (message.method && message.id !== undefined) {
        // Complete outbound requests through the real JSON-RPC client.
        const result = message.method === "thread/list" ? { data: [], nextCursor: null } : {};
        queueMicrotask(() => receive(providerError !== undefined && message.method?.startsWith("akra/resourceVersion/")
          ? { id: message.id, error: { code: providerError } }
          : { id: message.id, result }));
      }
    },
    onMessage: (listener) => (messages.add(listener), () => messages.delete(listener)),
    onConnectionState: (listener) => (states.add(listener), () => states.delete(listener)),
    ready: () => undefined,
  };
  const state = remoteHostStore(crypto.randomUUID());
  const session = new DesktopSession(bridge, desktopSessionStore(state), "test", (callback) => callback(), readOnly);
  session.start();
  cleanups.push(() => session.stop());
  const next = (predicate: (message: Message) => boolean) => {
    const result = deferred<Message>();
    const timeout = setTimeout(() => result.reject(new Error("Expected session message did not arrive.")), 1_000);
    const listener = (message: Message) => {
      if (!predicate(message)) return;
      clearTimeout(timeout);
      observers.delete(listener);
      result.resolve(message);
    };
    observers.add(listener);
    return result.promise;
  };
  return {
    session, sent, next, receive, currentVersion, jira, state,
    rejectProvider: (code: number) => { providerError = code; },
    connect: () => states.forEach((listener) => listener({ phase: "connected", endpoint: "test" })),
    change: (value: JiraStatus) => { status = value; changed.forEach((listener) => listener()); },
    disconnect: () => states.forEach((listener) => listener({ phase: "disconnected", endpoint: "test" })),
  };
}

const request = { kind: "jira", key: "jira.example/jira/APP-12", url: "https://jira.example/jira/browse/APP-12" };

describe("Desktop Jira reverse version session", () => {
  it("refreshes documents after a delayed login finishes registering", async () => {
    const h = harness();
    const login = deferred<JiraStatus>();
    h.jira.status = () => login.promise;
    const initialized = h.next((message) => message.method === "akra/gateway/info");
    const refreshed = deferred<void>();
    h.session.onVersionProviderReady(() => {
      void h.session.requestGatewayService("akra/context/refresh", { threadId: "open" })
        .then(() => refreshed.resolve(), refreshed.reject);
    });
    h.connect();
    await initialized;
    await h.session.requestGatewayService("akra/context/refresh", { threadId: "open" });
    login.resolve({ connected: true, siteUrl: "https://jira.example/jira", deployment: "server", email: null });
    await refreshed.promise;
    expect(h.sent.filter((message) => message.method === "akra/context/refresh")).toHaveLength(2);
  });

  it.each([true, false])("keeps an old Gateway usable when Jira connected=%s", async (connected) => {
    const h = harness();
    const login = deferred<JiraStatus>();
    h.jira.status = () => login.promise;
    h.rejectProvider(-32601);
    const initialized = deferred<void>();
    const unsubscribe = h.state.subscribe((state) => {
      if (state.gateway !== null) {
        unsubscribe();
        initialized.resolve();
      }
    });
    cleanups.push(unsubscribe);
    h.connect();
    await initialized.promise;
    const registration = h.next((message) => message.method?.startsWith("akra/resourceVersion/") === true);
    login.resolve(connected
      ? { connected: true, siteUrl: "https://jira.example/jira", deployment: "server", email: null }
      : { connected: false });
    await registration;
    // The regular RPC round trip is an event barrier after the provider rejection.
    await h.session.requestGatewayService("akra/context/thread", { threadId: "open" });
    expect(h.state.getState().sessionError).toBeNull();
  });

  it("registers the local login and answers Gateway-owned request IDs", async () => {
    // Given a local session with an existing Jira login.
    const h = harness();
    const registered = h.next((message) => message.method === "akra/resourceVersion/register");
    h.connect();
    expect((await registered).params).toEqual({ kind: "jira", siteUrl: "https://jira.example/jira" });
    const answer = h.next((message) => message.id === "akra:resource-version:1");
    // When a Gateway reverse request arrives.
    h.receive({ id: "akra:resource-version:1", method: "akra/resourceVersion/current", params: request });
    // Then the regular JSON-RPC client answers from the narrow typed bridge.
    expect((await answer).result).toEqual({ version: "2026-11-17T00:00:00.000Z" });
    expect(h.currentVersion).toHaveBeenCalledWith({ key: request.key, url: request.url });
  });

  it("replaces registration on account change and unregisters on Jira disconnect", async () => {
    // Given a registered provider.
    const h = harness();
    const first = h.next((message) => message.method === "akra/resourceVersion/register");
    h.connect();
    await first;
    // When the account changes, then the same session replaces its registration.
    const replacement = h.next((message) => message.method === "akra/resourceVersion/register");
    h.change({ connected: true, siteUrl: "https://other.example", deployment: "cloud", email: "other@example" });
    expect((await replacement).params).toEqual({ kind: "jira", siteUrl: "https://other.example" });
    const removed = h.next((message) => message.method === "akra/resourceVersion/unregister");
    h.change({ connected: false });
    expect((await removed).params).toEqual({ kind: "jira" });
  });

  it.each(["account", "connection"])("drops a stale result after %s changes", async (change) => {
    // Given an in-flight reverse request.
    const h = harness();
    const first = h.next((message) => message.method === "akra/resourceVersion/register");
    h.connect();
    await first;
    const pending = deferred<{ version: string }>();
    h.currentVersion.mockImplementation(() => pending.promise);
    h.receive({ id: "old-provider", method: "akra/resourceVersion/current", params: request });
    // When ownership changes before completion.
    if (change === "account") h.change({ connected: false });
    else h.disconnect();
    pending.resolve({ version: "old-version" });
    await pending.promise;
    // A later server response acts as an exact event barrier after the stale response continuation.
    const barrier = h.next((message) => message.id === "barrier");
    h.receive({ id: "barrier", method: "unsupported" });
    await barrier;
    // Then the old request is not answered on the new provider session.
    expect(h.sent.some((message) => message.id === "old-provider")).toBe(false);
  });

  it("rejects reverse requests on read-only sessions without calling main", async () => {
    // Given a read-only host, even with a Jira bridge attached.
    const h = harness(true);
    h.connect();
    const answer = h.next((message) => message.id === "remote");
    // When the host requests a local Jira version.
    h.receive({ id: "remote", method: "akra/resourceVersion/current", params: request });
    // Then no local credentials are used and no provider is registered.
    expect((await answer).error?.code).toBe(-32601);
    expect(h.currentVersion).not.toHaveBeenCalled();
    expect(h.sent.some((message) => message.method?.startsWith("akra/resourceVersion/"))).toBe(false);
  });

  it("does not expose Jira on a remote transport", () => {
    // Given the actual remote transport adapter.
    const bridge = remoteHostBridge({
      open: () => undefined, close: () => undefined, send: () => undefined,
      onMessage: () => () => undefined, onConnectionState: () => () => undefined,
    }, "remote", "main");
    // Then remote sessions cannot obtain the local Jira bridge.
    expect(bridge.jira).toBeUndefined();
  });
});
