import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { encodeFrame, object, readFrames } from "../browser-mcp/frames";
import type { BrowserToolResult } from "../shared/browserAgent";

export interface BrowserBrokerCall {
  requestId: string;
  registrationId: string;
  threadId: string;
  tool: string;
  arguments: unknown;
}

export interface BrowserAgentTransportOptions {
  command: string;
  args?: readonly string[];
  environment?: NodeJS.ProcessEnv;
  onCall: (call: BrowserBrokerCall) => void;
  onCancel: (requestId: string) => void;
  onClosed: (error: Error) => void;
}

type Pending = { resolve: (result: unknown) => void; reject: (error: Error) => void; timer: NodeJS.Timeout };

export class BrowserAgentTransport {
  private child: ChildProcessWithoutNullStreams | undefined;
  private starting: Promise<void> | undefined;
  private ready = false;
  private closed = false;
  private sequence = 0;
  private readonly pending = new Map<string, Pending>();
  private readonly calls = new Set<string>();
  private resolveReady: (() => void) | undefined;
  private rejectReady: ((error: Error) => void) | undefined;
  private readyTimer: NodeJS.Timeout | undefined;
  private removeFrames: (() => void) | undefined;

  constructor(private readonly options: BrowserAgentTransportOptions) {}

  start(): Promise<void> {
    if (this.closed) return Promise.reject(new Error("Browser broker is closed."));
    if (this.starting) return this.starting;
    this.starting = new Promise<void>((resolve, reject) => {
      this.resolveReady = resolve;
      this.rejectReady = reject;
    });
    this.readyTimer = setTimeout(() => this.finish(), 10_000);
    try {
      const child = spawn(this.options.command, [...(this.options.args ?? ["desktop-browser-bridge"])], {
        env: this.options.environment ?? process.env,
        detached: false,
        windowsHide: true,
        stdio: "pipe",
      });
      this.child = child;
      this.removeFrames = readFrames(child.stdout, (frame) => this.receive(frame), () => this.finish());
      child.stderr.resume();
      child.on("error", () => this.finish());
      child.on("exit", () => this.finish());
      child.stdout.on("end", () => this.finish());
      child.stdin.on("error", () => this.finish());
    } catch {
      this.finish();
    }
    return this.starting;
  }

  async prepare(): Promise<{ registrationId: string; sessionFile: string }> {
    const result = await this.control({ command: "prepare" });
    if (!object(result) || typeof result.registrationId !== "string" || !result.registrationId ||
      typeof result.sessionFile !== "string" || !result.sessionFile) {
      this.finish();
      throw new Error("Invalid browser broker response.");
    }
    return { registrationId: result.registrationId, sessionFile: result.sessionFile };
  }

  async bind(registrationId: string, threadId: string): Promise<void> {
    await this.control({ command: "bind", registrationId, threadId });
  }

  async revoke(registrationId: string): Promise<void> {
    await this.control({ command: "revoke", registrationId });
  }

  answer(requestId: string, result: BrowserToolResult): void {
    if (this.calls.delete(requestId)) this.send({ command: "answer", requestId, result });
  }

  fail(requestId: string, code: string, message: string): void {
    if (this.calls.delete(requestId)) this.send({ command: "answer", requestId, error: { code, message } });
  }

  async close(): Promise<void> {
    const child = this.child;
    if (!child) {
      this.finish();
      return;
    }
    if (child.exitCode !== null || child.signalCode !== null) {
      this.finish();
      return;
    }
    let timer: NodeJS.Timeout | undefined;
    const exited = new Promise<void>((resolve) => {
      const done = () => {
        clearTimeout(timer);
        child.off("exit", done);
        child.off("close", done);
        resolve();
      };
      child.once("exit", done);
      child.once("close", done);
      timer = setTimeout(() => child.kill(), 5_000);
    });
    this.finish(false);
    await exited;
  }

  private async control(command: Record<string, unknown>): Promise<unknown> {
    await this.start();
    if (this.closed) throw new Error("Browser broker is closed.");
    if (this.pending.size >= 128) throw new Error("Browser broker is busy.");
    const id = `browser-${++this.sequence}`;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => this.finish(), 10_000);
      this.pending.set(id, { resolve, reject, timer });
      this.send({ id, ...command });
    });
  }

  private send(frame: unknown): void {
    if (this.closed) return;
    try {
      if (!this.child || this.child.stdin.writableLength > 8 * 1024 * 1024) {
        this.finish();
        return;
      }
      this.child.stdin.write(encodeFrame(frame));
    } catch {
      this.finish();
    }
  }

  private receive(frame: unknown): void {
    if (this.closed) return;
    if (!object(frame)) return this.finish();
    if (!this.ready) {
      if (frame.type !== "ready" || Object.keys(frame).length !== 1) return this.finish();
      this.ready = true;
      clearTimeout(this.readyTimer);
      this.resolveReady?.();
      return;
    }
    if (typeof frame.id === "string") {
      const pending = this.pending.get(frame.id);
      if (!pending) return this.finish();
      this.pending.delete(frame.id);
      clearTimeout(pending.timer);
      if (object(frame.error) && typeof frame.error.code === "string" && typeof frame.error.message === "string") {
        pending.reject(new Error("Browser broker rejected the request."));
      } else if ("result" in frame && !("error" in frame)) {
        pending.resolve(frame.result);
      } else {
        pending.reject(new Error("Invalid browser broker response."));
        this.finish();
      }
      return;
    }
    if (frame.type === "cancel" && typeof frame.requestId === "string") {
      if (this.calls.delete(frame.requestId)) this.options.onCancel(frame.requestId);
      return;
    }
    if (frame.type === "call" && typeof frame.requestId === "string" &&
      typeof frame.registrationId === "string" && typeof frame.threadId === "string" &&
      typeof frame.tool === "string" && "arguments" in frame &&
      !this.calls.has(frame.requestId) && this.calls.size < 128) {
      this.calls.add(frame.requestId);
      this.options.onCall({
        requestId: frame.requestId, registrationId: frame.registrationId,
        threadId: frame.threadId, tool: frame.tool, arguments: frame.arguments,
      });
      return;
    }
    this.finish();
  }

  private finish(terminate = true): void {
    if (this.closed) return;
    this.closed = true;
    const error = new Error("Browser broker is closed.");
    clearTimeout(this.readyTimer);
    this.removeFrames?.();
    this.rejectReady?.(error);
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
    for (const id of this.calls) this.options.onCancel(id);
    this.calls.clear();
    this.child?.stdin.end();
    if (terminate) this.child?.kill();
    this.options.onClosed(error);
  }
}
