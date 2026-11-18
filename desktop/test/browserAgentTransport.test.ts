import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import { afterEach, describe, expect, it, vi } from "vitest";
import { BrowserAgentTransport } from "../src/main/browserAgentTransport";

const { spawn } = vi.hoisted(() => ({ spawn: vi.fn() }));
vi.mock("node:child_process", () => ({ spawn }));

function setup() {
  const child = Object.assign(new EventEmitter(), {
    stdin: new PassThrough(), stdout: new PassThrough(), stderr: new PassThrough(),
    exitCode: null as number | null, signalCode: null as string | null,
    kill: vi.fn(() => { child.signalCode = "SIGTERM"; child.emit("exit"); return true; }),
  });
  spawn.mockReturnValue(child);
  child.stdin.on("finish", () => {
    child.exitCode = 0;
    child.emit("exit");
  });
  const onCall = vi.fn(), onCancel = vi.fn(), onClosed = vi.fn();
  const transport = new BrowserAgentTransport({ command: "gateway", onCall, onCancel, onClosed });
  const sent: Record<string, unknown>[] = [];
  child.stdin.on("data", (chunk: Buffer) => sent.push(JSON.parse(chunk.toString())));
  const receive = (frame: unknown) => child.stdout.write(`${JSON.stringify(frame)}\n`);
  return { transport, child, receive, sent, onCall, onCancel, onClosed };
}

afterEach(() => vi.clearAllMocks());

describe("BrowserAgentTransport", () => {
  it("starts lazily and waits for the exact ready handshake", async () => {
    const h = setup();
    expect(spawn).not.toHaveBeenCalled();
    const start = h.transport.start();
    expect(spawn).toHaveBeenCalledWith("gateway", ["desktop-browser-bridge"],
      expect.objectContaining({ detached: false, windowsHide: true, stdio: "pipe" }));
    expect(h.sent).toEqual([]);
    h.receive({ type: "ready" });
    await start;
    await h.transport.close();
    expect(h.onClosed).toHaveBeenCalledOnce();
    expect(h.child.kill).not.toHaveBeenCalled();
  });

  it("correlates control IDs and relays calls, answers, and cancellation", async () => {
    const h = setup();
    const start = h.transport.start();
    h.receive({ type: "ready" });
    await start;
    const prepare = h.transport.prepare();
    await Promise.resolve();
    const id = h.sent[0]!.id;
    h.receive({ id, result: { registrationId: "r", sessionFile: "private-session" } });
    expect(await prepare).toEqual({ registrationId: "r", sessionFile: "private-session" });
    const call = { type: "call", requestId: "q", registrationId: "r", threadId: "t", tool: "browser_status", arguments: {} };
    h.receive(call);
    expect(h.onCall).toHaveBeenCalledWith(expect.objectContaining({ requestId: "q", threadId: "t" }));
    h.transport.answer("q", { content: [] });
    expect(h.sent[1]).toEqual({ command: "answer", requestId: "q", result: { content: [] } });
    h.receive({ ...call, requestId: "q2" });
    h.receive({ type: "cancel", requestId: "q2" });
    expect(h.onCancel).toHaveBeenCalledWith("q2");
    await h.transport.close();
  });

  it("uses fixed errors and rejects pending requests on broker EOF", async () => {
    const h = setup();
    const start = h.transport.start();
    h.receive({ type: "ready" });
    await start;
    const bind = h.transport.bind("r", "t");
    const rejected = expect(bind).rejects.toThrow("Browser broker rejected the request.");
    await Promise.resolve();
    h.receive({ id: h.sent[0]!.id, error: { code: "private-token", message: "/private/socket" } });
    await rejected;
    const pending = h.transport.revoke("r");
    const closed = expect(pending).rejects.toThrow("Browser broker is closed.");
    await Promise.resolve();
    h.child.stdout.emit("end");
    await closed;
    expect(h.onClosed).toHaveBeenCalledOnce();
  });

  it("rejects an invalid ready frame and cancels live work on process exit", async () => {
    const h = setup();
    const start = h.transport.start();
    const rejected = expect(start).rejects.toThrow("Browser broker is closed.");
    h.receive({ type: "ready", endpoint: "private" });
    await rejected;
    expect(h.child.kill).toHaveBeenCalledOnce();
    const live = setup();
    const ready = live.transport.start();
    live.receive({ type: "ready" });
    await ready;
    live.receive({ type: "call", requestId: "q", registrationId: "r", threadId: "t", tool: "browser_wait", arguments: {} });
    live.child.emit("exit");
    expect(live.onCancel).toHaveBeenCalledWith("q");
    expect(live.onClosed).toHaveBeenCalledOnce();
  });

  it("rejects control work if EOF follows ready before its continuation", async () => {
    const h = setup();
    const pending = h.transport.prepare();
    const rejected = expect(pending).rejects.toThrow("Browser broker is closed.");
    h.receive({ type: "ready" });
    h.child.stdout.emit("end");
    await rejected;
    expect(h.sent).toEqual([]);
  });
});
