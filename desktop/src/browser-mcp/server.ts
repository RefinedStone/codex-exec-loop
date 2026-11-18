import type { Readable, Writable } from "node:stream";
import { isBrowserToolName, type BrowserToolResult } from "../shared/browserAgent";
import { encodeFrame, object, readFrames } from "./frames";
import { BROWSER_TOOLS, validArguments } from "./tools";

type RpcId = string | number;
export interface BrowserMcpOptions {
  input: Readable;
  output: Writable;
  call: (threadId: string, tool: string, args: unknown, signal: AbortSignal) => Promise<BrowserToolResult>;
  onClosed: () => void;
}

export class BrowserMcpServer {
  private readonly pending = new Map<RpcId, AbortController>();
  private initialized = false;
  private closed = false;
  private readonly removeFrames: () => void;

  constructor(private readonly options: BrowserMcpOptions) {
    this.removeFrames = readFrames(options.input, (frame) => this.receive(frame), () => this.close());
    options.input.on("end", () => this.close());
    options.input.on("error", () => this.close());
    options.output.on("error", () => this.close());
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.removeFrames();
    for (const controller of this.pending.values()) controller.abort();
    this.pending.clear();
    this.options.onClosed();
  }

  private send(frame: unknown): void {
    if (this.closed) return;
    try {
      if (this.options.output.writableLength > 8 * 1024 * 1024) return this.close();
      this.options.output.write(encodeFrame(frame));
    } catch { this.close(); }
  }

  private error(id: RpcId | null, code: number, message: string): void {
    this.send({ jsonrpc: "2.0", id, error: { code, message } });
  }

  private receive(frame: unknown): void {
    if (!object(frame) || frame.jsonrpc !== "2.0" || typeof frame.method !== "string") {
      this.error(null, -32600, "Invalid request.");
      return;
    }
    const params = object(frame.params) ? frame.params : {};
    if (frame.method === "notifications/cancelled") {
      if (typeof params.requestId === "string" || typeof params.requestId === "number") {
        this.pending.get(params.requestId)?.abort();
      }
      return;
    }
    if (!("id" in frame)) return;
    if (typeof frame.id !== "string" && !(typeof frame.id === "number" && Number.isSafeInteger(frame.id))) {
      this.error(null, -32600, "Invalid request.");
      return;
    }
    const id = frame.id;
    if (this.pending.has(id)) return this.error(id, -32600, "Duplicate request.");
    const result = (value: unknown) => this.send({ jsonrpc: "2.0", id, result: value });
    if (frame.method === "initialize") {
      if (typeof params.protocolVersion !== "string") return this.error(id, -32602, "Invalid initialization.");
      const protocolVersion = ["2024-11-05", "2025-03-26", "2025-06-18"].includes(params.protocolVersion)
        ? params.protocolVersion : "2025-06-18";
      this.initialized = true;
      return result({
        protocolVersion, capabilities: { tools: { listChanged: false } },
        serverInfo: { name: "akra_browser", version: "1.0.0" },
      });
    }
    if (frame.method === "ping") return result({});
    if (!this.initialized) return this.error(id, -32000, "Browser server is not initialized.");
    if (frame.method === "tools/list") return result({ tools: BROWSER_TOOLS });
    if (frame.method !== "tools/call") return this.error(id, -32601, "Method not found.");
    if (!isBrowserToolName(params.name) || !validArguments(params.name, params.arguments ?? {}) ||
      !object(params._meta) || typeof params._meta.threadId !== "string" ||
      !params._meta.threadId || params._meta.threadId.length > 256) {
      return this.error(id, -32602, "Invalid browser tool arguments or thread metadata.");
    }
    if (this.pending.size) return this.error(id, -32000, "Browser server is busy.");
    const controller = new AbortController();
    this.pending.set(id, controller);
    let settled = false;
    const complete = (value?: BrowserToolResult) => {
      if (settled) return;
      settled = true;
      this.pending.delete(id);
      controller.signal.removeEventListener("abort", cancelled);
      if (controller.signal.aborted) this.error(id, -32800, "Browser call was cancelled.");
      else if (value) result(value);
      else this.error(id, -32000, "Browser call failed.");
    };
    const cancelled = () => complete();
    controller.signal.addEventListener("abort", cancelled, { once: true });
    const threadId = params._meta.threadId;
    void Promise.resolve().then(() => {
      if (controller.signal.aborted) throw new Error("Browser call was cancelled.");
      return this.options.call(threadId, params.name as string, params.arguments ?? {}, controller.signal);
    }).then(
      (value) => {
        complete(value);
      },
      () => complete(),
    );
  }
}
