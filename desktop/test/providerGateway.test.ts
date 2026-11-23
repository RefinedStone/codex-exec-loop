import { afterEach, describe, expect, it, vi } from "vitest";
import { createProviderGateway, ProviderGatewayError } from "../src/renderer/work/providerGateway";
import type { GatewayRequest } from "../src/renderer/work/notionHub";
import type { OperationReceipt, ProviderCredentialState } from "../src/shared/providerCredentials";
import { connected, disconnected, deferred, linearIssue, linearSummary, nativeBridge, PROFILE, receipt, reference, SECRET_CANARY, SECOND_PROFILE } from "./providerFixtures";

const NONE = { teamId: null, projectId: null, currentCycle: false };
const cleanups: Array<() => void> = [];
afterEach(() => { cleanups.splice(0).forEach(fn => fn()); vi.useRealTimers(); });
function harness(initial = connected()) {
  let state = structuredClone(initial);
  const calls: Array<{ method: string; params: Record<string, unknown> }> = [];
  let respond: (method: string, params: Record<string, unknown>) => unknown | Promise<unknown> = method => {
    if (method.endsWith("/state")) return structuredClone(state);
    if (method.endsWith("/summary")) return linearSummary();
    if (method.endsWith("/issuePrompt")) return { text: "Synthetic draft only" };
    throw new Error("No fake response for " + method);
  };
  const changed = new Set<(value: unknown) => void>();
  const connections = new Set<() => void>();
  const native = nativeBridge();
  const request = (async (method: string, value: unknown) => {
    const params = structuredClone(value) as Record<string, unknown>;
    calls.push({ method, params });
    return respond(method, params);
  }) as GatewayRequest;
  const facade = createProviderGateway(request, native, {
    onChanged: fn => (changed.add(fn), () => { changed.delete(fn); }),
    onConnectionChanged: fn => (connections.add(fn), () => { connections.delete(fn); }),
  });
  facade.start(); cleanups.push(() => facade.dispose());
  return {
    facade, native, calls, get state() { return state; }, set state(value: ProviderCredentialState) { state = value; },
    respond: (fn: typeof respond) => { respond = fn; },
    changed: (revision: number, extra = {}) => changed.forEach(fn => fn({ provider: state.provider, profileId: state.profileId, revision, ...extra })),
    reconnect: () => connections.forEach(fn => fn()),
  };
}

describe("provider Gateway facade", () => {
  it("sends a metadata-only native connect and never falls back to token IPC", async () => {
    const h = harness(disconnected("jira"));
    h.respond((method, params) => {
      if (method.endsWith("/state")) return structuredClone(h.state);
      if (method.endsWith("/connect")) {
        h.state = connected("jira");
        return receipt({ provider: "jira", operationId: String(params.operationId), connection: reference("jira") });
      }
      throw new Error("unexpected");
    });
    await h.facade.jira.connect({ siteUrl: "https://jira.example/jira", deployment: "server" });
    const connect = h.calls.find(call => call.method.endsWith("/connect"))!;
    expect(connect.params).toEqual({ profileId: PROFILE, provider: "jira", operationId: expect.any(String), expectedRevision: 0,
      configuration: { siteUrl: "https://jira.example/jira", deployment: "server" } });
    await expect(h.facade.jira.connect({ token: SECRET_CANARY } as never)).rejects.toThrow();
    expect(JSON.stringify(h.calls)).not.toContain(SECRET_CANARY);
    expect(h.calls.filter(call => call.method.endsWith("/connect"))).toHaveLength(1);
    expect(h.native.migrateLegacy).not.toHaveBeenCalled();
  });

  it("shows unsupported on an old Gateway without inspecting local credentials", async () => {
    const h = harness();
    h.respond(() => { throw { code: -32601, message: SECRET_CANARY }; });
    await expect(h.facade.linear.status()).rejects.toMatchObject({ reason: "unsupported" });
    await expect(h.facade.linear.legacyStatus()).rejects.not.toThrow(SECRET_CANARY);
    expect(h.native.legacyStatus).not.toHaveBeenCalled();
    expect(h.native.migrateLegacy).not.toHaveBeenCalled();
  });

  it.each(["locked", "conflict", "busy"] as const)("uses the actual native error kind %s without exposing its raw message", async kind => {
    const h = harness();
    h.respond(() => { throw { code: -32000, message: SECRET_CANARY, data: { kind } }; });
    await expect(h.facade.linear.status()).rejects.toMatchObject({ reason: kind });
    await expect(h.facade.linear.status()).rejects.not.toThrow(SECRET_CANARY);
  });

  it("coalesces the initial 30-issue page and filters cached team/project/cycle without fetching", async () => {
    const h = harness();
    const page = deferred<unknown>();
    h.respond(method => method.endsWith("/state") ? structuredClone(h.state) : page.promise);
    const a = h.facade.linear.summary(NONE, true);
    const b = h.facade.linear.summary({ ...NONE, teamId: "team-b" }, false);
    await vi.waitFor(() => expect(h.calls.filter(call => call.method.endsWith("/summary"))).toHaveLength(1));
    page.resolve(linearSummary(Array.from({ length: 30 }, (_, index) => linearIssue(index + 1, index % 2 ? {
      team: { id: "team-b", key: "B", name: "B" }, project: null, cycle: null,
    } : {}))));
    expect((await a).issues).toHaveLength(30);
    expect((await b).issues).toHaveLength(15);
    expect((await h.facade.linear.summary({ ...NONE, projectId: "project-a", currentCycle: true }, false)).issues).toHaveLength(15);
    expect(h.calls.filter(call => call.method.endsWith("/summary"))).toHaveLength(1);
    expect(h.calls.find(call => call.method.endsWith("/summary"))?.params).toEqual({ connection: reference(), filters: NONE, refresh: true });
  });

  it("joins an explicit refresh when filters change over an existing cache", async () => {
    const h = harness();
    await h.facade.linear.summary(NONE, true);
    const next = deferred<unknown>();
    h.respond(method => method.endsWith("/state") ? structuredClone(h.state) : next.promise);
    const refresh = h.facade.linear.summary(NONE, true);
    let settled = false;
    const filtered = h.facade.linear.summary({ ...NONE, teamId: "team-b" }, false).then(value => { settled = true; return value; });
    await Promise.resolve();
    expect(settled).toBe(false);
    next.resolve(linearSummary([linearIssue(2, { team: { id: "team-b", key: "B", name: "B" } })]));
    await refresh;
    expect((await filtered).issues.map(issue => issue.id)).toEqual(["issue-2"]);
    expect(h.calls.filter(call => call.method.endsWith("/summary"))).toHaveLength(2);
  });

  it("keeps cached and in-flight reads through same-revision cancelled reconnect phases", async () => {
    const h = harness(); await h.facade.linear.status();
    const detail = deferred<unknown>();
    await h.facade.linear.summary(NONE, true);
    h.respond((method, params) => {
      if (method.endsWith("/state")) return structuredClone(h.state);
      if (method.endsWith("/issuePrompt")) return detail.promise;
      if (method.endsWith("/connect")) {
        const cancelled = receipt({ operationId: String(params.operationId), expectedRevision: 1, status: "cancelled", connection: null, error: "cancelled" });
        h.state = { ...h.state, operation: cancelled };
        h.changed(1);
        return cancelled;
      }
      throw new Error("must use cache");
    });
    const read = h.facade.linear.issuePrompt("issue-1");
    await expect(h.facade.linear.connect()).rejects.toMatchObject({ reason: "cancelled" });
    detail.resolve({ text: "Existing account draft" });
    expect(await read).toBe("Existing account draft");
    expect((await h.facade.linear.summary(NONE, false)).issues).toHaveLength(1);
    expect(h.calls.filter(call => call.method.endsWith("/summary"))).toHaveLength(1);
  });

  it.each(["revision", "profile", "transport", "lagged"])("fences old list and detail responses on %s change", async change => {
    const h = harness(); await h.facade.linear.status();
    const old = deferred<unknown>();
    h.respond(method => method.endsWith("/state") ? structuredClone(h.state) : old.promise);
    const list = h.facade.linear.summary(NONE, true);
    const detail = h.facade.linear.issuePrompt("issue-1");
    const rejectedList = expect(list).rejects.toMatchObject({ reason: "conflict" });
    const rejectedDetail = expect(detail).rejects.toMatchObject({ reason: "conflict" });
    await vi.waitFor(() => expect(h.calls.filter(call => call.method.startsWith("akra/providers/"))).toHaveLength(2));
    if (change === "revision") { h.state = disconnected("linear", 2); h.changed(2); }
    if (change === "profile") {
      h.state = { ...disconnected(), profileId: SECOND_PROFILE }; h.changed(0);
    }
    if (change === "transport") h.reconnect();
    if (change === "lagged") h.changed(1, { lagged: true });
    old.resolve(linearSummary());
    await rejectedList; await rejectedDetail;
    expect(h.native.migrateLegacy).not.toHaveBeenCalled();
  });

  it("fences a stale state response behind a newer revision notification", async () => {
    const h = harness(); const old = deferred<unknown>();
    let count = 0;
    h.respond(() => ++count === 1 ? old.promise : structuredClone(disconnected("linear", 2)));
    const pending = h.facade.linear.status();
    h.state = disconnected("linear", 2); h.changed(2);
    const latest = h.facade.linear.status();
    old.resolve(connected());
    expect((await latest).activeConnection).toBeNull();
    expect((await pending).revision).toBe(2);
    expect(count).toBe(2);
  });

  it("recovers a lost connect response with the same operation ID and no automatic reconnect", async () => {
    const h = harness(disconnected());
    let original: OperationReceipt | undefined;
    h.respond((method, params) => {
      if (method.endsWith("/state")) return structuredClone(h.state);
      if (method.endsWith("/connect")) {
        original = receipt({ operationId: String(params.operationId) });
        h.state = { ...connected(), operation: original };
        throw new Error(SECRET_CANARY);
      }
      if (method.endsWith("/operation")) return original;
      throw new Error("unexpected");
    });
    await expect(h.facade.linear.connect()).rejects.not.toThrow(SECRET_CANARY);
    expect((await h.facade.linear.retry()).status).toBe("connected");
    const connect = h.calls.find(call => call.method.endsWith("/connect"))!;
    expect(h.calls.filter(call => call.method.endsWith("/connect"))).toHaveLength(1);
    expect(h.calls.find(call => call.method.endsWith("/operation"))?.params.operationId).toBe(connect.params.operationId);
  });

  it("keeps a 140-second native prompt deadline above the Gateway owner deadline", async () => {
    vi.useFakeTimers();
    const h = harness(disconnected());
    const pending = deferred<unknown>();
    h.respond(method => method.endsWith("/state") ? structuredClone(h.state) : pending.promise);
    let settled = false;
    const connect = h.facade.linear.connect().finally(() => { settled = true; });
    const refused = expect(connect).rejects.toBeInstanceOf(ProviderGatewayError);
    await vi.advanceTimersByTimeAsync(120_001);
    expect(settled).toBe(false);
    await vi.advanceTimersByTimeAsync(20_000);
    await refused;
    expect(h.calls.filter(call => call.method.endsWith("/connect"))).toHaveLength(1);
  });

  it("cancels the displayed newer operation after a lost response was confirmed terminal", async () => {
    const h = harness(disconnected());
    h.respond((method, params) => {
      if (method.endsWith("/state")) return structuredClone(h.state);
      if (method.endsWith("/connect")) {
        h.state = { ...connected(), operation: receipt({ operationId: String(params.operationId) }) };
        throw new Error("response lost");
      }
      if (method.endsWith("/cancel")) {
        const cancelled = receipt({ operationId: String(params.operationId), expectedRevision: 1, status: "cancelled", connection: null, error: "cancelled" });
        h.state = { ...h.state, operation: cancelled };
        return cancelled;
      }
      throw new Error("unexpected");
    });
    await expect(h.facade.linear.connect()).rejects.toThrow();
    h.state = { ...h.state, operation: receipt({ operationId: "other-client-B", expectedRevision: 1,
      status: "awaitingInput", connection: null }) };
    h.changed(1);
    await h.facade.linear.status();
    await expect(h.facade.linear.cancel()).rejects.toMatchObject({ reason: "cancelled" });
    expect(h.calls.find(call => call.method.endsWith("/cancel"))?.params.operationId).toBe("other-client-B");
  });

  it("prepares a correlated legacy snapshot before sending the public disconnect", async () => {
    const h = harness();
    const prepared = deferred<{ kind: "prepared" }>();
    h.native.prepareLegacyDisconnect.mockImplementation(() => prepared.promise);
    h.respond((method, params) => {
      if (method.endsWith("/state")) return structuredClone(h.state);
      if (method.endsWith("/disconnect")) {
        const done = receipt({ operationId: String(params.operationId), expectedRevision: 1, kind: "disconnect", connection: null });
        h.state = { ...disconnected("linear", 2), operation: done };
        return done;
      }
      throw new Error("unexpected");
    });
    const result = h.facade.linear.disconnect();
    await vi.waitFor(() => expect(h.native.prepareLegacyDisconnect).toHaveBeenCalledTimes(1));
    expect(h.calls.filter(call => call.method.endsWith("/disconnect"))).toHaveLength(0);
    const request = h.native.prepareLegacyDisconnect.mock.calls[0]![0];
    expect(request).toMatchObject({ profileId: PROFILE, provider: "linear", expectedRevision: 1, operationId: expect.any(String) });
    prepared.resolve({ kind: "prepared" });
    expect((await result).status).toBe("disconnected");
    expect(h.calls.find(call => call.method.endsWith("/disconnect"))?.params).toEqual(request);
  });

  it.each(["held", "rejected"])("still revokes Gateway authority when legacy preparation is %s", async failure => {
    const h = harness();
    if (failure === "held") h.native.prepareLegacyDisconnect.mockResolvedValue({ kind: "held", reason: "unavailable" });
    else h.native.prepareLegacyDisconnect.mockRejectedValue(new Error(SECRET_CANARY));
    h.native.cleanupLegacy.mockResolvedValue({ kind: "held", reason: "notFound" });
    h.respond((method, params) => {
      if (method.endsWith("/state")) return structuredClone(h.state);
      if (method.endsWith("/disconnect")) {
        const done = receipt({ operationId: String(params.operationId), expectedRevision: 1, kind: "disconnect", connection: null });
        h.state = { ...disconnected("linear", 2), operation: done };
        return done;
      }
      throw new Error("revoked read reached Gateway");
    });
    await expect(h.facade.linear.disconnect()).rejects.toThrow("연결은 해제했습니다");
    expect(h.calls.filter(call => call.method.endsWith("/disconnect"))).toHaveLength(1);
    expect((await h.facade.linear.status()).activeConnection).toBeNull();
    await expect(h.facade.linear.issuePrompt("issue-1")).rejects.toMatchObject({ reason: "notConnected" });
  });

  it("uses the original disconnect operation for explicit deletion retry while reads remain blocked", async () => {
    const h = harness(); let disconnects = 0; let operation: OperationReceipt;
    h.respond((method, params) => {
      if (method.endsWith("/state")) return structuredClone(h.state);
      if (method.endsWith("/disconnect")) {
        disconnects++;
        operation = receipt({ kind: "disconnect", operationId: String(params.operationId), expectedRevision: 1,
          connection: null, cleanupPending: disconnects === 1 });
        h.state = { ...disconnected("linear", 2), operation };
        return operation;
      }
      if (method.endsWith("/operation")) return operation;
      throw new Error("disconnected read reached Gateway");
    });
    h.native.cleanupLegacy.mockImplementation(async request => ({ kind: "committed", operationId: request.operationId, cleanupPending: disconnects === 1 }));
    await expect(h.facade.linear.disconnect()).rejects.toThrow("삭제 다시 시도");
    await expect(h.facade.linear.issuePrompt("issue-1")).rejects.toMatchObject({ reason: "notConnected" });
    await h.facade.linear.status();
    expect(disconnects).toBe(1); // State queries never retry keyring deletion.
    expect((await h.facade.linear.retry()).activeConnection).toBeNull();
    const calls = h.calls.filter(call => call.method.endsWith("/disconnect"));
    expect(calls).toHaveLength(2);
    expect(calls[1]!.params).toEqual(calls[0]!.params);
    expect(h.native.prepareLegacyDisconnect).toHaveBeenCalledTimes(1); // Replay cannot resnapshot a later file.
  });

  it("passes the current opaque connection to main for native link validation", async () => {
    const h = harness(connected("jira"));
    await h.facade.jira.openExternal("https://jira.example/jira/browse/APP-1");
    expect(h.native.openExternal).toHaveBeenCalledWith({
      connection: reference("jira"), url: "https://jira.example/jira/browse/APP-1",
    });
    h.native.openExternal.mockRejectedValue(new Error(SECRET_CANARY));
    await expect(h.facade.jira.openExternal("https://evil.example/")).rejects.not.toThrow(SECRET_CANARY);
  });
});
