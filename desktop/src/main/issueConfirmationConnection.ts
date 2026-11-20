import type { ConnectionState } from "../shared/bridge";
import { GatewayConnection } from "./gatewayConnection";

export type ConfirmationMethod =
  | "akra/kernel/attach"
  | "akra/kernel/confirmation/register"
  | "akra/kernel/confirmation/prepare"
  | "akra/kernel/command_confirmed";

export interface ConfirmationRpc {
  request(method: ConfirmationMethod, params: unknown): Promise<unknown>;
  close(): void;
}

type Pending = {
  resolve(value: unknown): void;
  reject(error: Error): void;
  timer: NodeJS.Timeout;
};

/** 일반 renderer relay와 공유하지 않는 main 전용 연결이다. 끊어진 확인은 재연결하여 재생하지 않는다. */
export class IssueConfirmationConnection implements ConfirmationRpc {
  private readonly connection: GatewayConnection;
  private starting: Promise<void> | undefined;
  private resolveReady: (() => void) | undefined;
  private rejectReady: ((error: Error) => void) | undefined;
  private readyTimer: NodeJS.Timeout | undefined;
  private connected = false;
  private closed = false;
  private sequence = 0;
  private readonly pending = new Map<number, Pending>();

  constructor(endpoint: string, private readonly onClosed: () => void) {
    this.connection = new GatewayConnection({
      endpoint,
      onMessage: (text) => this.receive(text),
      onState: (state) => this.stateChanged(state),
    });
  }

  async request(method: ConfirmationMethod, params: unknown): Promise<unknown> {
    await this.start();
    if (this.closed || !this.connected) throw this.failure();
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => this.finish(true), 15_000);
      this.pending.set(id, { resolve, reject, timer });
      if (!this.connection.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }))) this.finish(true);
    });
  }

  close(): void {
    this.finish(false);
  }

  private start(): Promise<void> {
    if (this.closed) return Promise.reject(this.failure());
    if (this.starting) return this.starting;
    this.starting = new Promise((resolve, reject) => {
      this.resolveReady = resolve;
      this.rejectReady = reject;
    });
    this.readyTimer = setTimeout(() => this.finish(true), 15_000);
    try {
      this.connection.start();
    } catch {
      this.finish(true);
    }
    return this.starting;
  }

  private stateChanged(state: ConnectionState): void {
    if (this.closed) return;
    if (state.phase === "connected") {
      if (this.connected) {
        this.finish(true);
        return;
      }
      this.connected = true;
      clearTimeout(this.readyTimer);
      this.resolveReady?.();
      this.resolveReady = undefined;
      this.rejectReady = undefined;
    } else if (state.phase === "disconnected" || this.connected) {
      this.finish(true);
    }
  }

  private receive(text: string): void {
    if (this.closed) return;
    try {
      if (Buffer.byteLength(text, "utf8") > 256 * 1024) throw this.failure();
      const message: unknown = JSON.parse(text);
      if (!isRecord(message)) throw this.failure();
      // 이 연결의 kernel 알림은 renderer로 전달하지 않는다.
      if (typeof message.id !== "number") return;
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      clearTimeout(pending.timer);
      if ("error" in message || !("result" in message)) pending.reject(this.failure());
      else pending.resolve(message.result);
    } catch {
      this.finish(true);
    }
  }

  private finish(notify: boolean): void {
    if (this.closed) return;
    this.closed = true;
    this.connected = false;
    clearTimeout(this.readyTimer);
    this.connection.close();
    const error = this.failure();
    this.rejectReady?.(error);
    this.rejectReady = undefined;
    this.resolveReady = undefined;
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
    if (notify) this.onClosed();
  }

  private failure(): Error {
    return new Error("확인 전용 연결을 사용할 수 없습니다.");
  }
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
