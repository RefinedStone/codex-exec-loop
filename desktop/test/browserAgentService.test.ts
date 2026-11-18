import { afterEach, describe, expect, it, vi } from "vitest";
import { BrowserAgentService, type BrowserAgentHost } from "../src/main/browserAgentService";
import type { BrowserBrokerCall } from "../src/main/browserAgentTransport";
import type { BrowserToolResult } from "../src/shared/browserAgent";

function setup() {
  let callbacks!: Parameters<ConstructorParameters<typeof BrowserAgentService>[1]>[0];
  let change!: Parameters<BrowserAgentHost["onViewChange"]>[0];
  const view = { generation: 1, visible: true, profileId: "default", url: "about:blank", title: "" };
  const result: BrowserToolResult = { content: [{ type: "text", text: "done" }] };
  const engine = { attached: true, execute: vi.fn(async () => result), dispose: vi.fn() };
  const broker = {
    start: vi.fn(async () => undefined),
    prepare: vi.fn(async () => ({ registrationId: "r", sessionFile: "private-file" })),
    bind: vi.fn(async () => undefined), revoke: vi.fn(async () => undefined),
    answer: vi.fn(), fail: vi.fn(), close: vi.fn(async () => undefined),
  };
  const host: BrowserAgentHost = {
    command: "electron", args: ["browserMcp.js"], view: () => view,
    automation: () => engine, notify: vi.fn(),
    onViewChange: (listener) => { change = listener; return () => undefined; },
  };
  const service = new BrowserAgentService(host, (listeners) => { callbacks = listeners; return broker; });
  const call = (tool: string, args: unknown = {}, threadId = "t") => {
    const frame: BrowserBrokerCall = { requestId: "q", registrationId: "r", threadId, tool, arguments: args };
    callbacks.onCall(frame);
  };
  const bind = async () => {
    await service.prepare(1);
    service.setActiveThread(1, "t");
    await service.bind(1, "r", "t");
  };
  return { service, broker, engine, view, call, bind, closeBroker: () => callbacks.onClosed(new Error("closed")), change: (kind: Parameters<typeof change>[1]) => change(1, kind) };
}

afterEach(() => vi.useRealTimers());
describe("BrowserAgentService", () => {
  it("does not revive a closed broker when a hidden view returns", async () => {
    const h = setup();
    await h.bind();
    h.closeBroker();
    h.change("hidden");
    h.change("shown");
    expect(h.service.status(1).phase).toBe("unavailable");
    expect(() => h.service.resume(1)).toThrow();
    await h.service.close();
  });
  it("allows a 45-second condition wait to finish after 35 seconds", async () => {
    vi.useFakeTimers();
    const h = setup();
    await h.bind();
    let finish!: (value: BrowserToolResult) => void;
    h.engine.execute.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const answered = new Promise<void>(resolve => h.broker.answer.mockImplementation(() => resolve()));
    h.call("browser_wait", { text: "ready", timeoutMs: 45_000 });
    await vi.advanceTimersByTimeAsync(35_000);
    expect(h.broker.fail).not.toHaveBeenCalled();
    finish({ content: [{ type: "text", text: "ready" }] });
    await answered;
    await h.service.close();
  });
  it("rejects another window and thread without executing browser actions", async () => {
    const h = setup();
    await h.bind();
    await expect(h.service.release(2, "r")).rejects.toThrow();
    h.call("browser_click", { ref: "e1" }, "other");
    expect(h.broker.fail).toHaveBeenCalledWith("q", "not_authorized", expect.any(String));
    expect(h.engine.execute).not.toHaveBeenCalled();
    await h.service.close();
  });

  it("pauses on hide and requires explicit resume after the view returns", async () => {
    const h = setup();
    await h.bind();
    h.view.visible = false;
    h.change("hidden");
    expect(h.service.status(1).phase).toBe("paused");
    expect(h.engine.dispose).toHaveBeenCalledOnce();
    h.view.visible = true;
    h.change("shown");
    h.call("browser_snapshot");
    expect(h.broker.fail).toHaveBeenCalledWith("q", "paused", expect.any(String));
    h.service.resume(1);
    expect(h.service.status(1).phase).toBe("active");
    await h.service.close();
  });

  it("completes human handoff only after explicit resume", async () => {
    const h = setup();
    await h.bind();
    const answered = new Promise<void>((resolve) => h.broker.answer.mockImplementation(() => resolve()));
    h.call("browser_handoff", { reason: "Sign in manually" });
    expect(h.service.status(1).phase).toBe("handoff");
    expect(h.broker.answer).not.toHaveBeenCalled();
    h.service.resume(1);
    await answered;
    expect(h.service.status(1).phase).toBe("active");
    await h.service.close();
  });

  it("aborts pending work when the selected thread changes", async () => {
    const h = setup();
    await h.bind();
    let signal: AbortSignal | undefined;
    h.engine.execute.mockImplementation((_tool?: unknown, _args?: unknown, active?: AbortSignal) => {
      signal = active;
      return new Promise<BrowserToolResult>(() => undefined);
    });
    const failed = new Promise<void>((resolve) => h.broker.fail.mockImplementation(() => resolve()));
    h.call("browser_wait", { text: "ready" });
    h.service.setActiveThread(1, "other");
    await failed;
    expect(signal?.aborted).toBe(true);
    expect(h.broker.answer).not.toHaveBeenCalled();
    await h.service.close();
  });
});
