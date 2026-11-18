import { PassThrough } from "node:stream";
import { describe, expect, it, vi } from "vitest";
import { BrowserMcpServer, type BrowserMcpOptions } from "../src/browser-mcp/server";
import { MAX_FRAME_BYTES, readFrames } from "../src/browser-mcp/frames";
import { BROWSER_TOOLS, validArguments } from "../src/browser-mcp/tools";
import { BROWSER_TOOL_NAMES } from "../src/shared/browserAgent";

function harness(call: BrowserMcpOptions["call"] = vi.fn(async () => ({ content: [{ type: "text" as const, text: "ok" }] }))) {
  const input = new PassThrough();
  const output = new PassThrough();
  const messages: Record<string, unknown>[] = [];
  const listeners = new Map<number, (value: Record<string, unknown>) => void>();
  readFrames(output, (frame) => {
    const message = frame as Record<string, unknown>;
    messages.push(message);
    listeners.get(message.id as number)?.(message);
  }, () => { throw new Error("Invalid test response"); });
  const closed = vi.fn();
  const server = new BrowserMcpServer({ input, output, call, onClosed: closed });
  const send = (frame: unknown) => input.write(`${JSON.stringify(frame)}\n`);
  const response = (id: number) => new Promise<Record<string, unknown>>((resolve) => listeners.set(id, resolve));
  const request = (id: number, method: string, params?: unknown) => {
    const next = response(id);
    send({ jsonrpc: "2.0", id, method, params });
    return next;
  };
  return { input, output, messages, closed, server, send, response, request, call };
}

describe("browser MCP stdio", () => {
  it("initializes, lists exactly the closed browser schemas, and pings", async () => {
    const h = harness();
    const initialized = await h.request(1, "initialize", { protocolVersion: "2025-06-18" });
    expect(initialized.result).toMatchObject({ protocolVersion: "2025-06-18", capabilities: { tools: {} } });
    expect((await h.request(2, "tools/list")).result).toEqual({ tools: BROWSER_TOOLS });
    expect(BROWSER_TOOLS.map((tool) => tool.name)).toEqual(BROWSER_TOOL_NAMES);
    expect(BROWSER_TOOLS.every((tool) => tool.inputSchema.additionalProperties === false)).toBe(true);
    expect((await h.request(3, "ping")).result).toEqual({});
    h.server.close();
  });

  it("requires thread metadata and rejects extra fields and unsafe navigation", async () => {
    const h = harness();
    await h.request(1, "initialize", { protocolVersion: "2025-06-18" });
    expect(await h.request(2, "tools/call", { name: "browser_status", arguments: {} }))
      .toMatchObject({ error: { code: -32602 } });
    expect(validArguments("browser_status", { owner: 3 })).toBe(false);
    expect(validArguments("browser_navigate", { url: "file:///private" })).toBe(false);
    expect(validArguments("browser_navigate", { url: "about:blank" })).toBe(true);
    expect(validArguments("browser_handoff", {})).toBe(false);
    expect(validArguments("browser_handoff", { reason: "x".repeat(241) })).toBe(false);
    expect(validArguments("browser_snapshot", { diff: true, maxChars: 1000 })).toBe(true);
    expect(validArguments("browser_wait", { event: "load" })).toBe(true);
    expect(validArguments("browser_wait", { event: "load", text: "ready" })).toBe(false);
    expect(h.call).not.toHaveBeenCalled();
    h.server.close();
  });

  it("propagates cancellation, serializes calls, and keeps private errors fixed", async () => {
    let abort: (() => void) | undefined;
    let resolveStarted!: (signal: AbortSignal) => void;
    const started = new Promise<AbortSignal>((resolve) => { resolveStarted = resolve; });
    const call = vi.fn((_thread: string, _tool: string, _args: unknown, signal: AbortSignal) => {
      resolveStarted(signal);
      return new Promise<{ content: { type: "text"; text: string }[] }>((_resolve, reject) => {
        abort = () => reject(new Error("private-token /private/socket"));
        signal.addEventListener("abort", abort, { once: true });
      });
    });
    const h = harness(call);
    await h.request(1, "initialize", { protocolVersion: "2025-06-18" });
    const next = h.request(2, "tools/call", { name: "browser_status", _meta: { threadId: "t1" } });
    const signal = await started;
    expect(await h.request(3, "tools/call", { name: "browser_status", _meta: { threadId: "t1" } }))
      .toMatchObject({ error: { code: -32000 } });
    h.send({ jsonrpc: "2.0", method: "notifications/cancelled", params: { requestId: 2 } });
    expect(signal.aborted).toBe(true);
    expect(await next).toMatchObject({ error: { code: -32800, message: "Browser call was cancelled." } });
    expect(JSON.stringify(h.messages)).not.toContain("private-token");
    h.server.close();
  });

  it("aborts pending work on EOF and closes on oversized input", async () => {
    let resolveStarted!: (signal: AbortSignal) => void;
    const started = new Promise<AbortSignal>((resolve) => { resolveStarted = resolve; });
    const h = harness(vi.fn(async (_t: string, _n: string, _a: unknown, signal: AbortSignal) => {
      resolveStarted(signal);
      return await new Promise<{ content: { type: "text"; text: string }[] }>((_resolve, reject) =>
        signal.addEventListener("abort", () => reject(new Error("closed")), { once: true }));
    }));
    await h.request(1, "initialize", { protocolVersion: "2025-06-18" });
    h.send({ jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "browser_status", _meta: { threadId: "t" } } });
    const signal = await started;
    const ended = new Promise<void>((resolve) => h.input.once("end", resolve));
    h.input.end();
    await ended;
    expect(signal.aborted).toBe(true);
    expect(h.closed).toHaveBeenCalledOnce();
    const oversized = harness();
    oversized.input.write(Buffer.alloc(MAX_FRAME_BYTES + 1, 32));
    expect(oversized.closed).toHaveBeenCalledOnce();
  });

  it("completes cancellation even if the tool does not settle", async () => {
    let resolveStarted!: () => void;
    const started = new Promise<void>((resolve) => { resolveStarted = resolve; });
    const h = harness(async () => {
      resolveStarted();
      return await new Promise(() => {});
    });
    await h.request(1, "initialize", { protocolVersion: "2025-06-18" });
    const pending = h.request(2, "tools/call", { name: "browser_status", _meta: { threadId: "t" } });
    await started;
    h.send({ jsonrpc: "2.0", method: "notifications/cancelled", params: { requestId: 2 } });
    expect(await pending).toMatchObject({ error: { code: -32800 } });
    h.server.close();
  });

  it("decodes split UTF-8 without corruption", () => {
    const stream = new PassThrough();
    const receive = vi.fn();
    readFrames(stream, receive, vi.fn());
    const frame = Buffer.from('{"text":"한"}\n');
    stream.write(frame.subarray(0, 11));
    stream.write(frame.subarray(11));
    expect(receive).toHaveBeenCalledWith({ text: "한" });
  });
});
