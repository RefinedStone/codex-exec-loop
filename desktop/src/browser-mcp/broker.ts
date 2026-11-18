import { open } from "node:fs/promises";
import { connect, type Socket } from "node:net";
import { encodeFrame, object, readFrames } from "./frames";
import type { BrowserToolResult } from "../shared/browserAgent";

export interface BrowserSession {
  version: 1;
  endpoint: string;
  registrationId: string;
  token: string;
}

export async function readSession(path: string | undefined): Promise<BrowserSession> {
  if (!path) throw new Error("Browser session is unavailable.");
  try {
    const file = await open(path, "r");
    try {
      const stat = await file.stat();
      if (!stat.isFile() || stat.size > 16_384 ||
        (process.platform !== "win32" && ((stat.mode & 0o077) !== 0 || stat.uid !== process.getuid?.()))) {
        throw new Error();
      }
      const data: unknown = JSON.parse(await file.readFile("utf8"));
      if (!object(data) || data.version !== 1 ||
        typeof data.endpoint !== "string" || !data.endpoint ||
        typeof data.registrationId !== "string" || !data.registrationId ||
        typeof data.token !== "string" || !data.token ||
        Object.keys(data).length !== 4) throw new Error();
      return data as unknown as BrowserSession;
    } finally { await file.close(); }
  } catch { throw new Error("Browser session is unavailable."); }
}

export class BrowserBrokerClient {
  private watch: Socket | undefined;
  private authenticated = false;
  private readonly calls = new Set<Socket>();
  private closed = false;

  constructor(private readonly session: BrowserSession, private readonly onClosed: () => void) {}

  start(): Promise<void> {
    return new Promise((resolve, reject) => {
      if (this.closed || this.watch) return reject(new Error("Browser session is unavailable."));
      const socket = connect(this.session.endpoint);
      this.watch = socket;
      let authenticated = false;
      const timer = setTimeout(() => this.close(), 10_000);
      const remove = readFrames(socket, (frame) => {
        if (authenticated || !object(frame) || !object(frame.result) ||
          frame.result.registrationId !== this.session.registrationId) {
          this.close();
          return;
        }
        authenticated = true;
        this.authenticated = true;
        clearTimeout(timer);
        resolve();
      }, () => this.close());
      socket.on("connect", () => socket.write(encodeFrame({
        kind: "watch", registrationId: this.session.registrationId, token: this.session.token,
      })));
      socket.on("error", () => this.close());
      socket.on("end", () => this.close());
      socket.on("close", () => {
        clearTimeout(timer);
        remove();
        reject(new Error("Browser session is unavailable."));
        this.close();
      });
    });
  }

  call(threadId: string, tool: string, args: unknown, signal: AbortSignal): Promise<BrowserToolResult> {
    if (this.closed || !this.authenticated || this.calls.size) return Promise.reject(new Error("Browser call is unavailable."));
    if (signal.aborted) return Promise.reject(new Error("Browser call was cancelled."));
    return new Promise((resolve, reject) => {
      const socket = connect(this.session.endpoint);
      this.calls.add(socket);
      let settled = false;
      const finish = (result?: BrowserToolResult) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        remove();
        signal.removeEventListener("abort", abort);
        this.calls.delete(socket);
        socket.destroy();
        if (result) resolve(result);
        else reject(new Error(signal.aborted ? "Browser call was cancelled." : "Browser call failed."));
      };
      const abort = () => finish();
      const timer = setTimeout(() => finish(), tool === "browser_handoff" ? 600_000 : 65_000);
      const remove = readFrames(socket, (frame) => {
        if (object(frame) && object(frame.result) && Array.isArray(frame.result.content)) {
          finish(frame.result as unknown as BrowserToolResult);
        } else finish();
      }, () => finish());
      signal.addEventListener("abort", abort, { once: true });
      socket.on("connect", () => {
        try {
          socket.write(encodeFrame({
            kind: "call", registrationId: this.session.registrationId, token: this.session.token,
            threadId, tool, arguments: args,
          }));
        } catch { finish(); }
      });
      socket.on("error", () => finish());
      socket.on("end", () => finish());
      socket.on("close", () => finish());
    });
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.watch?.destroy();
    for (const socket of this.calls) socket.destroy();
    this.onClosed();
  }
}
