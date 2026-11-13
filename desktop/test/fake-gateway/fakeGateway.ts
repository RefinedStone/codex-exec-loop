// 가짜 Gateway: Gateway 녹화 파일(JSONL, P1 §4.6)을 읽어 요청 method 에 맞는 응답을 재생한다.
// unix 소켓이나 Windows named pipe 에서 WebSocket 을 받는다. 턴을 쓰지 않고 화면과 접속을 시험한다 (D-13).
import { readFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { type WebSocket, WebSocketServer } from "ws";

interface RecordedLine {
  direction: "client" | "gateway";
  text: string;
}

interface JsonRpcEnvelope {
  id?: number | string | null;
  method?: string;
  result?: unknown;
  error?: unknown;
}

/** 녹화에서 요청 method → 응답 result(또는 error) 표를 만든다. 같은 method 는 처음 것을 쓴다. */
export function loadReplayTable(recordingPath: string): Map<string, Pick<JsonRpcEnvelope, "result" | "error">> {
  const lines = readFileSync(recordingPath, "utf8")
    .split(/\r?\n/)
    .filter((line) => line.trim().length > 0)
    .map((line) => JSON.parse(line) as RecordedLine);
  const methodById = new Map<string, string>();
  const table = new Map<string, Pick<JsonRpcEnvelope, "result" | "error">>();
  for (const line of lines) {
    const message = JSON.parse(line.text) as JsonRpcEnvelope;
    if (message.id === undefined || message.id === null) {
      continue;
    }
    const key = String(message.id);
    if (line.direction === "client" && message.method) {
      methodById.set(key, message.method);
    } else if (line.direction === "gateway" && !message.method) {
      const method = methodById.get(key);
      if (method && !table.has(method)) {
        table.set(method, "error" in message ? { error: message.error } : { result: message.result });
      }
    }
  }
  return table;
}

let endpointCounter = 0;

/** 시험마다 겹치지 않는 짧은 접속 주소이다 (macOS 소켓 경로 104 바이트 한도). */
export function uniqueTestEndpoint(): string {
  endpointCounter += 1;
  const name = `akra-fake-${process.pid}-${endpointCounter}`;
  return process.platform === "win32" ? `\\\\.\\pipe\\${name}` : join(tmpdir(), `${name}.sock`);
}

export interface FakeGateway {
  readonly endpoint: string;
  /** `local` 접속이 보낸 요청이다. */
  readonly receivedMethods: string[];
  /** 원격 호스트(`/host/<id>`, plan 25) 접속이 보낸 요청이다. */
  readonly remoteReceivedMethods: string[];
  /** 먼저 도착해서 쓰인 서버 요청 답이다. */
  readonly answeredRequests: Array<{ requestId: string; result: unknown }>;
  close(): Promise<void>;
}

/**
 * 녹화의 이어지는 gateway 메시지이다. 클라이언트가 요청(method)을 보내거나 서버 요청에 답(response id)하면,
 * 녹화에서 그 다음에 왔던 알림과 서버 요청을 같은 순서로 보낸다.
 */
interface ReplaySegment {
  trigger: string;
  followUps: string[];
}

function triggerKey(message: JsonRpcEnvelope): string | null {
  if (message.method) {
    return `request:${message.method}`;
  }
  return message.id === undefined || message.id === null ? null : `response:${String(message.id)}`;
}

export function loadReplaySegments(recordingPath: string): ReplaySegment[] {
  const segments: ReplaySegment[] = [];
  let current: ReplaySegment | null = null;
  for (const line of readFileSync(recordingPath, "utf8").split(/\r?\n/).filter((text) => text.trim().length > 0)) {
    const record = JSON.parse(line) as RecordedLine;
    const message = JSON.parse(record.text) as JsonRpcEnvelope;
    if (record.direction === "client") {
      const key = triggerKey(message);
      current = key ? { trigger: key, followUps: [] } : null;
      if (current) {
        segments.push(current);
      }
    } else if (current && message.method) {
      current.followUps.push(record.text);
    }
  }
  return segments;
}

/**
 * 가짜 저장소 커널이다 (plan 18 K3-6). `akra/kernel/*` 는 Gateway 가 직접 답하므로 녹화가 아니라 이 값으로 답한다.
 */
export interface FakeKernel {
  /** 붙기 결과의 저장소 신원이다. null 이면 다른 프로세스가 잠금을 쥔 것처럼 -32012 로 거절한다. */
  kernel: string | null;
  /** 조회 종류 → 결과 이벤트 */
  queries: Record<string, unknown>;
  /** 붙은 뒤 첫 명령에 응답한 다음 보낼 이벤트이다. 초기 조회가 끝났다는 신호로 쓴다. */
  eventsAfterAttach?: unknown[];
}

function answerKernel(kernel: FakeKernel, socket: WebSocket, message: JsonRpcEnvelope & { params?: Record<string, unknown> }): void {
  const respond = (body: object) => socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, ...body }));
  switch (message.method) {
    case "akra/kernel/attach":
      if (kernel.kernel === null) {
        respond({ error: { code: -32012, message: "the repository kernel runs in another process (pid 4242)" } });
        return;
      }
      respond({ result: { client_id: "1", protocol_version: 1, kernel: kernel.kernel } });
      return;
    case "akra/kernel/query": {
      const type = (message.params?.query as { type?: string } | undefined)?.type ?? "";
      respond(type in kernel.queries ? { result: kernel.queries[type] } : { error: { code: -32602, message: `unknown query ${type}` } });
      return;
    }
    case "akra/kernel/command":
      respond({ result: { processed: true } });
      return;
    default:
      respond({ result: {} });
  }
}

/** 녹화를 여러 개 주면 응답 표는 앞의 녹화가 우선하고, 이어지는 메시지는 순서대로 이어 붙인다. */
/** 가짜 MCP hub 이다 (plan 21). `akra/mcp/*` 는 Gateway 가 직접 답하므로 녹화가 아니라 이 값으로 답한다. */
export interface FakeMcpHub {
  /** `akra/mcp/statuses` 결과의 statuses 이다. */
  statuses: unknown[];
  /** 도구 이름 → `akra/mcp/call` 결과 */
  tools: Record<string, unknown>;
}

function answerMcpHub(hub: FakeMcpHub, socket: WebSocket, message: JsonRpcEnvelope & { params?: Record<string, unknown> }): void {
  const respond = (body: object) => socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, ...body }));
  if (message.method === "akra/mcp/statuses") {
    respond({ result: { statuses: hub.statuses } });
    return;
  }
  const tool = String(message.params?.tool ?? "");
  if (message.method === "akra/mcp/call" && tool in hub.tools) {
    respond({ result: hub.tools[tool] });
    return;
  }
  respond({ error: { code: -32000, message: `fake MCP hub has no answer for ${message.method} ${tool}` } });
}

/** `akra/context/*` 가 돌려주는 문서이다. `changed` 는 가짜 Gateway 가 붙인다. */
export interface FakeThreadDocument {
  kind: string;
  key: string;
  url: string;
  title: string;
  source: string;
  firstSeenAt: string;
}

export async function startFakeGateway(
  endpoint: string,
  recordingPaths: string | string[],
  options: {
    kernel?: FakeKernel;
    onRequest?(method: string): void;
    mcpHub?: FakeMcpHub;
    threadDocuments?: Record<string, FakeThreadDocument[]>;
    /** 스레드 id → `akra/context/refresh` 가 바뀐 것으로 찾는 문서의 키이다. `akra/context/markRead` 뒤에는 다시 찾지 않는다. */
    changedThreadDocuments?: Record<string, string[]>;
    /**
     * `local` 외에 중계하는 호스트 id 이다 (plan 25 S1a). 접속은 요청 경로 `/host/<id>` 로 고른다. 모르는 id 는 1008 로
     * 닫는다. 원격 접속은 같은 녹화를 재생하고, `akra/kernel/*` 는 거절하고, `akra/context/*` 는 빈 목록으로 답한다.
     */
    hosts?: string[];
    /** 원격 접속의 `akra/dots/list` 답이다 (D-19). 없으면 모르는 method 로 답한다. */
    dots?: unknown[];
    /** 원격 접속의 `akra/dots/messages` 답이다. 방 id 는 보지 않는다. */
    dotMessages?: unknown[];
  } = {},
): Promise<FakeGateway> {
  // 작업 컨텍스트의 바뀜 상태이다: `스레드 id\n문서 키`. 읽은 문서는 같은 버전으로 다시 바뀜이 되지 않는다.
  const changedDocuments = new Set<string>();
  const readDocuments = new Set<string>();
  const paths = Array.isArray(recordingPaths) ? recordingPaths : [recordingPaths];
  const table = new Map<string, Pick<JsonRpcEnvelope, "result" | "error">>();
  for (const path of paths) {
    for (const [method, recorded] of loadReplayTable(path)) {
      if (!table.has(method)) {
        table.set(method, recorded);
      }
    }
  }
  const segments = paths.flatMap((path) => loadReplaySegments(path));
  const receivedMethods: string[] = [];
  const remoteReceivedMethods: string[] = [];
  const hostIds = ["local", ...(options.hosts ?? [])];
  const answeredRequests: Array<{ requestId: string; result: unknown }> = [];
  const server: Server = createServer();
  const sockets = new WebSocketServer({ server });
  // app-server 처럼 서버 요청은 접속마다 따로 받고, 먼저 온 답만 쓴다 (Q-03 실측).
  const resolvedRequestIds = new Set<string>();
  /** 서버 요청 id → 그 요청을 받은 접속들과 스레드 id */
  const openServerRequests = new Map<string, { threadId: string; sockets: Set<WebSocket> }>();
  sockets.on("connection", (socket, request) => {
    const path = request.url ?? "/";
    const host = path.startsWith("/host/") ? (path.slice("/host/".length).split("/")[0] ?? "") : "local";
    if (!hostIds.includes(host)) {
      socket.close(1008, "unknown host");
      return;
    }
    const remote = host !== "local";
    let kernelEventsSent = false;
    // 녹화 재생은 접속마다 따로 한다. 창 두 개가 같은 스레드를 열면 둘 다 같은 기록을 받는다.
    const used = new Set<ReplaySegment>();
    const replayFollowUps = (message: JsonRpcEnvelope) => {
      const key = triggerKey(message);
      const segment = segments.find((candidate) => !used.has(candidate) && candidate.trigger === key);
      if (!segment) {
        return;
      }
      used.add(segment);
      for (const text of segment.followUps) {
        const followUp = JSON.parse(text) as JsonRpcEnvelope & { params?: { threadId?: string } };
        if (followUp.method && followUp.id !== undefined && followUp.id !== null) {
          const requestId = String(followUp.id);
          const open = openServerRequests.get(requestId) ?? { threadId: followUp.params?.threadId ?? "", sockets: new Set() };
          open.sockets.add(socket);
          openServerRequests.set(requestId, open);
        }
        socket.send(text);
      }
    };
    socket.on("message", (data) => {
      const message = JSON.parse(data.toString()) as JsonRpcEnvelope;
      if (!message.method) {
        // 서버 요청(승인 등)에 대한 클라이언트 답이다. 먼저 온 답이면 다른 접속에 resolved 를 알리고 녹화를 이어서
        // 보낸다. 이미 닫힌 요청에 온 늦은 답은 무시한다.
        const requestId = message.id === undefined || message.id === null ? null : String(message.id);
        if (requestId && openServerRequests.has(requestId)) {
          if (resolvedRequestIds.has(requestId)) {
            return;
          }
          resolvedRequestIds.add(requestId);
          const open = openServerRequests.get(requestId)!;
          answeredRequests.push({ requestId, result: message.result });
          for (const other of open.sockets) {
            if (other !== socket) {
              other.send(
                JSON.stringify({
                  jsonrpc: "2.0",
                  method: "serverRequest/resolved",
                  params: { threadId: open.threadId, requestId: message.id },
                }),
              );
            }
          }
        }
        replayFollowUps(message);
        return;
      }
      (remote ? remoteReceivedMethods : receivedMethods).push(message.method);
      options.onRequest?.(message.method);
      const hasId = message.id !== undefined && message.id !== null;
      if (remote && hasId && message.method.startsWith("akra/kernel/")) {
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, error: { code: -32601, message: "kernel methods are local only" } }));
        return;
      }
      if (remote && hasId && message.method.startsWith("akra/context/")) {
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { resources: [] } }));
        return;
      }
      if (remote && hasId && options.dots && message.method === "akra/dots/list") {
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { dots: options.dots } }));
        return;
      }
      if (remote && hasId && options.dotMessages && message.method === "akra/dots/messages") {
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { messages: options.dotMessages } }));
        return;
      }
      if (remote && hasId && options.dotMessages && message.method === "akra/dots/send") {
        const params = (message as { params?: { text?: unknown } }).params;
        options.dotMessages.push({ id: `sent-${options.dotMessages.length}`, createdAt: "2026-11-14T02:00:00Z", author: "Me", fromDot: false, text: String(params?.text ?? "") });
        // dot 은 조금 뒤에 답한다. 클라이언트가 다시 읽어야 보인다.
        const messages = options.dotMessages;
        setTimeout(() => messages.push({ id: `reply-${messages.length}`, createdAt: "2026-11-14T02:00:03Z", author: "Mika", fromDot: true, text: "On it." }), 1_000);
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { sent: true } }));
        return;
      }
      if (options.hosts && hasId && message.method === "akra/gateway/info") {
        const recorded = (table.get(message.method)?.result ?? {}) as object;
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { ...recorded, host, hosts: hostIds.map((id) => ({ id })) } }));
        return;
      }
      if (options.kernel && message.method.startsWith("akra/kernel/")) {
        answerKernel(options.kernel, socket, message);
        if (message.method === "akra/kernel/attach") kernelEventsSent = false;
        if (message.method === "akra/kernel/command" && !kernelEventsSent) {
          kernelEventsSent = true;
          for (const event of options.kernel.eventsAfterAttach ?? []) {
            socket.send(JSON.stringify({ jsonrpc: "2.0", method: "akra/kernel/event", params: { event } }));
          }
        }
        return;
      }
      // 작업 컨텍스트 (plan 22): 스레드 id → `akra/context/*` 의 resources 이다. 목록 조회는 바뀜을 찾지 않는다.
      if (options.threadDocuments && message.method.startsWith("akra/context/")) {
        const params = (message as { params?: { threadId?: unknown; key?: unknown } }).params;
        const threadId = String(params?.threadId ?? "");
        if (message.method === "akra/context/refresh") {
          for (const key of options.changedThreadDocuments?.[threadId] ?? []) {
            if (!readDocuments.has(`${threadId}\n${key}`)) changedDocuments.add(`${threadId}\n${key}`);
          }
        }
        if (message.method === "akra/context/markRead") {
          const document = `${threadId}\n${String(params?.key ?? "")}`;
          changedDocuments.delete(document);
          readDocuments.add(document);
        }
        const resources = (options.threadDocuments[threadId] ?? []).map((document) => ({ ...document, changed: changedDocuments.has(`${threadId}\n${document.key}`) }));
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { resources } }));
        return;
      }
      if (options.mcpHub && message.method.startsWith("akra/mcp/")) {
        answerMcpHub(options.mcpHub, socket, message);
        return;
      }
      if (message.id !== undefined && message.id !== null) {
        // 원격 호스트는 `thread/read` 로 연다. 녹화에는 같은 모양의 `thread/resume` 응답만 있다.
        const recorded = table.get(message.method) ?? (message.method === "thread/read" ? table.get("thread/resume") : undefined) ?? {
          error: { code: -32601, message: `fake gateway has no recording for ${message.method}` },
        };
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, ...recorded }));
      }
      replayFollowUps(message);
    });
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(endpoint, () => resolve());
  });
  return {
    endpoint,
    receivedMethods,
    remoteReceivedMethods,
    answeredRequests,
    close: async () => {
      for (const client of sockets.clients) {
        client.terminate();
      }
      await new Promise<void>((resolve) => sockets.close(() => resolve()));
      await new Promise<void>((resolve) => server.close(() => resolve()));
    },
  };
}

export const BASIC_RECORDING = fileURLToPath(new URL("./fixtures/basic.jsonl", import.meta.url));
export const APPROVAL_TURN_RECORDING = fileURLToPath(new URL("./fixtures/approval-turn.jsonl", import.meta.url));
export const INTERRUPTED_TURN_RECORDING = fileURLToPath(new URL("./fixtures/interrupted-turn.jsonl", import.meta.url));
/** approval-turn 의 스레드를 사이드바에 보여 주는 thread/list 녹화이다 (화면 시험용). */
export const APPROVAL_THREAD_LIST_RECORDING = fileURLToPath(
  new URL("./fixtures/approval-thread-list.jsonl", import.meta.url),
);
/** 업무 패널의 "스레드로 넘기기" 가 만드는 새 스레드이다. interrupted-turn 의 스레드에서 turns 를 비웠다. */
export const THREAD_START_RECORDING = fileURLToPath(new URL("./fixtures/thread-start.jsonl", import.meta.url));
