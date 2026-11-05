// renderer 의 JSON-RPC 클라이언트이다. 요청 ID, 대기 중 요청, 알림 처리기, 서버 요청 처리기를 가진다.
// 전송은 메시지 버스(문자열)에 맡긴다. 연결이 다시 맺어지면 `reset` 으로 대기 중 요청을 오류로 끝낸다.

export type JsonRpcId = number | string;

const JSON_RPC_METHOD_NOT_FOUND = -32601;
const JSON_RPC_INTERNAL_ERROR = -32603;

export class JsonRpcError extends Error {
  constructor(
    readonly code: number,
    message: string,
    readonly data?: unknown,
  ) {
    super(message);
    this.name = "JsonRpcError";
  }
}

interface PendingRequest {
  method: string;
  resolve(value: unknown): void;
  reject(error: Error): void;
}

export type NotificationListener = (method: string, params: unknown) => void;

/** 처리기가 이 값을 돌려주면 응답을 보내지 않는다. 다른 창이 이미 답해서 요청이 닫혔을 때 쓴다 (Q-03). */
export const NO_RESPONSE: unique symbol = Symbol("no-response");

export type ServerRequestHandler = (
  method: string,
  params: unknown,
  id: JsonRpcId,
) => Promise<unknown | typeof NO_RESPONSE>;

interface IncomingMessage {
  id?: JsonRpcId | null;
  method?: string;
  params?: unknown;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}

export class JsonRpcClient {
  private nextId = 1;
  private readonly pending = new Map<JsonRpcId, PendingRequest>();
  private readonly notificationListeners = new Set<NotificationListener>();
  private serverRequestHandler: ServerRequestHandler | undefined;

  constructor(private readonly sendText: (text: string) => void) {}

  request<T>(method: string, params?: unknown): Promise<T> {
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { method, resolve: resolve as (value: unknown) => void, reject });
      this.sendText(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    });
  }

  notify(method: string, params?: unknown): void {
    this.sendText(JSON.stringify({ jsonrpc: "2.0", method, params }));
  }

  onNotification(listener: NotificationListener): () => void {
    this.notificationListeners.add(listener);
    return () => this.notificationListeners.delete(listener);
  }

  /** 서버가 보낸 요청(승인 등)에 답할 처리기이다. 없으면 method not found 로 답한다. */
  setServerRequestHandler(handler: ServerRequestHandler | undefined): void {
    this.serverRequestHandler = handler;
  }

  /** 연결이 끊기거나 다시 맺어졌을 때 대기 중 요청을 모두 오류로 끝낸다. */
  reset(reason: string): void {
    const pending = [...this.pending.values()];
    this.pending.clear();
    for (const request of pending) {
      request.reject(new Error(`${request.method}: ${reason}`));
    }
  }

  get pendingCount(): number {
    return this.pending.size;
  }

  handleText(text: string): void {
    let message: IncomingMessage;
    try {
      message = JSON.parse(text) as IncomingMessage;
    } catch {
      return;
    }
    const hasId = message.id !== undefined && message.id !== null;
    if (message.method && hasId) {
      void this.answerServerRequest(message.id as JsonRpcId, message.method, message.params);
    } else if (message.method) {
      for (const listener of this.notificationListeners) {
        listener(message.method, message.params);
      }
    } else if (hasId) {
      this.settle(message.id as JsonRpcId, message);
    }
  }

  private settle(id: JsonRpcId, message: IncomingMessage): void {
    const request = this.pending.get(id);
    if (!request) {
      return;
    }
    this.pending.delete(id);
    if (message.error) {
      request.reject(new JsonRpcError(message.error.code, message.error.message, message.error.data));
    } else {
      request.resolve(message.result);
    }
  }

  private async answerServerRequest(id: JsonRpcId, method: string, params: unknown): Promise<void> {
    if (!this.serverRequestHandler) {
      this.sendError(id, JSON_RPC_METHOD_NOT_FOUND, `unsupported server request: ${method}`);
      return;
    }
    try {
      const result = await this.serverRequestHandler(method, params, id);
      if (result !== NO_RESPONSE) {
        this.sendText(JSON.stringify({ jsonrpc: "2.0", id, result }));
      }
    } catch (error) {
      const code = error instanceof JsonRpcError ? error.code : JSON_RPC_INTERNAL_ERROR;
      this.sendError(id, code, error instanceof Error ? error.message : String(error));
    }
  }

  private sendError(id: JsonRpcId, code: number, message: string): void {
    this.sendText(JSON.stringify({ jsonrpc: "2.0", id, error: { code, message } }));
  }
}
