import { afterEach, describe, expect, it } from "vitest";
import type { ConnectionState } from "../src/shared/bridge";
import { DesktopSession, type SessionBridge } from "../src/renderer/session/desktopSession";
import { desktopSessionStore, remoteHostStore } from "../src/renderer/session/desktopStore";

interface Message {
  id?: string | number; method?: string; params?: unknown; result?: unknown; error?: { code: number };
}
const cleanups: Array<() => void> = [];
afterEach(() => cleanups.splice(0).forEach(cleanup => cleanup()));

function harness(readOnly = false) {
  const states = new Set<(state: ConnectionState) => void>();
  const messages = new Set<(text: string) => void>();
  const sent: Message[] = [];
  const receive = (message: Message) => messages.forEach(listener => listener(JSON.stringify({ jsonrpc: "2.0", ...message })));
  const bridge: SessionBridge = {
    send: text => {
      const message: Message = JSON.parse(text);
      sent.push(message);
      if (message.method && message.id !== undefined) {
        const result = message.method === "thread/list" ? { data: [], nextCursor: null } : {};
        queueMicrotask(() => receive({ id: message.id, result }));
      }
    },
    onMessage: listener => (messages.add(listener), () => messages.delete(listener)),
    onConnectionState: listener => (states.add(listener), () => states.delete(listener)),
    ready: () => undefined,
  };
  const state = remoteHostStore(crypto.randomUUID());
  const session = new DesktopSession(bridge, desktopSessionStore(state), "test", callback => callback(), readOnly);
  session.start();
  cleanups.push(() => session.stop());
  return {
    session, sent, receive, state,
    connect: () => states.forEach(listener => listener({ phase: "connected", endpoint: "test" })),
    disconnect: () => states.forEach(listener => listener({ phase: "disconnected", endpoint: "test" })),
  };
}
async function barrier(h: ReturnType<typeof harness>) {
  await h.session.requestGatewayService("akra/context/thread", { threadId: "open" });
}
const changed = (revision: number, extra = {}) => ({
  method: "akra/providerCredentials/changed",
  params: { provider: "jira", profileId: "a".repeat(64), revision, ...extra },
});

describe("Gateway-owned Jira version session", () => {
  it("refreshes context on active revision change and lag without reverse registration", async () => {
    const h = harness(); let refreshed = 0;
    h.session.onVersionProviderReady(() => { refreshed++; });
    h.connect();
    await barrier(h);
    h.receive(changed(1)); h.receive(changed(1));
    expect(refreshed).toBe(1); // Same-revision operation phases do not reread documents.
    h.receive(changed(1, { lagged: true }));
    h.receive(changed(2));
    expect(refreshed).toBe(3);
    expect(h.sent.some(message => message.method?.startsWith("akra/resourceVersion/"))).toBe(false);
    expect(h.sent.some(message => message.method?.startsWith("akra/providerCredentials/"))).toBe(false);
  });

  it("keeps an old Gateway usable without Desktop credential registration", async () => {
    const h = harness();
    h.connect();
    await barrier(h);
    expect(h.state.getState().sessionError).toBeNull();
    expect(h.sent.some(message => message.method?.includes("resourceVersion"))).toBe(false);
  });

  it.each([false, true])("refuses the retired reverse credential request with remote=%s", async readOnly => {
    const h = harness(readOnly);
    h.connect();
    await barrier(h);
    h.receive({
      id: "retired-version", method: "akra/resourceVersion/current",
      params: { kind: "jira", key: "jira.example/APP-1", url: "https://jira.example/browse/APP-1" },
    });
    await barrier(h);
    expect(h.sent.find(message => message.id === "retired-version")?.error?.code).toBe(-32601);
    expect(h.sent.some(message => message.method?.startsWith("akra/resourceVersion/"))).toBe(false);
  });

  it("never sends local credential or business reads through a remote host", async () => {
    const h = harness(true);
    h.connect();
    await barrier(h);
    await expect(h.session.requestGatewayService("akra/providerCredentials/state", { provider: "jira" })).rejects.toThrow("원격");
    await expect(h.session.requestGatewayService("akra/providers/linear/summary", {})).rejects.toThrow("원격");
    let refreshed = 0;
    h.session.onVersionProviderReady(() => { refreshed++; });
    h.receive(changed(1));
    expect(refreshed).toBe(0);
    expect(h.sent.some(message => message.method?.startsWith("akra/provider"))).toBe(false);
  });

  it("reconsiders the same profile/revision after a transport restart", async () => {
    const h = harness(); let refreshed = 0;
    h.session.onVersionProviderReady(() => { refreshed++; });
    h.connect(); await barrier(h); h.receive(changed(1));
    h.disconnect(); h.connect(); await barrier(h); h.receive(changed(1));
    expect(refreshed).toBe(2);
  });
});
