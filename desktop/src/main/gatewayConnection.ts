// 창 하나의 Gateway 접속이다 (04 §4: 창마다 접속 1개). 메시지는 해석하지 않고 문자열 그대로 주고받는다.
// 접속이 끊기면 1초에서 시작해 최대 30초 간격으로 다시 접속한다. 접속에 실패하면 데몬을 한 번 시작한다.
// electron 에 의존하지 않으므로 Node 에서 바로 시험할 수 있다.
import { connect } from "node:net";

import WebSocket from "ws";

import type { ConnectionState } from "../shared/bridge";

const RECONNECT_INITIAL_MS = 1_000;
const RECONNECT_MAX_MS = 30_000;
/** 데몬 시작을 너무 자주 시도하지 않도록 한 번 시도한 뒤 이 시간 동안은 다시 시도하지 않는다. */
const DAEMON_START_COOLDOWN_MS = 10_000;

export interface GatewayConnectionOptions {
  endpoint: string;
  onMessage(text: string): void;
  onState(state: ConnectionState): void;
  /** 접속이 실패했을 때 데몬을 시작한다. 없으면 시작하지 않는다. */
  startDaemon?: () => void;
  reconnectInitialMs?: number;
  reconnectMaxMs?: number;
  now?: () => number;
}

export function nextReconnectDelay(currentMs: number, maxMs = RECONNECT_MAX_MS): number {
  return Math.min(currentMs * 2, maxMs);
}

export class GatewayConnection {
  private socket: WebSocket | undefined;
  private retryTimer: NodeJS.Timeout | undefined;
  private retryDelayMs: number;
  private lastDaemonStartAt = Number.NEGATIVE_INFINITY;
  private closed = false;

  constructor(private readonly options: GatewayConnectionOptions) {
    this.retryDelayMs = options.reconnectInitialMs ?? RECONNECT_INITIAL_MS;
  }

  start(): void {
    this.connect();
  }

  /** 연결되어 있지 않으면 메시지를 버린다. renderer 는 연결 상태를 보고 다시 보낸다. */
  send(text: string): boolean {
    if (this.socket?.readyState !== WebSocket.OPEN) {
      return false;
    }
    this.socket.send(text);
    return true;
  }

  close(): void {
    this.closed = true;
    clearTimeout(this.retryTimer);
    this.socket?.close();
    this.socket = undefined;
  }

  private connect(): void {
    if (this.closed) {
      return;
    }
    const { endpoint } = this.options;
    this.options.onState({ phase: "connecting", endpoint });
    // 호스트 이름은 의미가 없다. 실제 통로는 unix 소켓이나 named pipe 이다.
    const socket = new WebSocket("ws://localhost/", {
      createConnection: () => connect(endpoint),
      perMessageDeflate: false,
    });
    this.socket = socket;
    let opened = false;
    let failure: string | undefined;

    socket.on("open", () => {
      opened = true;
      this.retryDelayMs = this.options.reconnectInitialMs ?? RECONNECT_INITIAL_MS;
      this.options.onState({ phase: "connected", endpoint });
    });
    socket.on("message", (data, isBinary) => {
      if (!isBinary) {
        this.options.onMessage(data.toString());
      }
    });
    socket.on("error", (error: NodeJS.ErrnoException) => {
      failure = error.code ?? error.message;
    });
    socket.on("close", (code) => {
      if (this.socket === socket) {
        this.socket = undefined;
      }
      if (!opened) {
        this.maybeStartDaemon();
      }
      this.scheduleReconnect(opened ? `closed ${code}` : failure);
    });
  }

  private maybeStartDaemon(): void {
    const now = this.options.now?.() ?? Date.now();
    if (!this.options.startDaemon || now - this.lastDaemonStartAt < DAEMON_START_COOLDOWN_MS) {
      return;
    }
    this.lastDaemonStartAt = now;
    this.options.startDaemon();
  }

  private scheduleReconnect(detail: string | undefined): void {
    if (this.closed) {
      return;
    }
    const delay = this.retryDelayMs;
    this.options.onState({
      phase: "disconnected",
      endpoint: this.options.endpoint,
      retryInMs: delay,
      ...(detail ? { detail } : {}),
    });
    this.retryDelayMs = nextReconnectDelay(delay, this.options.reconnectMaxMs ?? RECONNECT_MAX_MS);
    this.retryTimer = setTimeout(() => this.connect(), delay);
  }
}
