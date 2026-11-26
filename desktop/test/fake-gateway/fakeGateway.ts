// 가짜 Gateway: Gateway 녹화 파일(JSONL, P1 §4.6)을 읽어 요청 method 에 맞는 응답을 재생한다.
// unix 소켓이나 Windows named pipe 에서 WebSocket 을 받는다. 턴을 쓰지 않고 화면과 접속을 시험한다 (D-13).
import { readFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { WebSocket, WebSocketServer } from "ws";
import { FakeChatHistory, type FakeChatAccount, type FakeChatOptions } from "./chatHistory.ts";

interface RecordedLine {
  direction: "client" | "gateway";
  text: string;
}

interface JsonRpcEnvelope {
  id?: number | string | null;
  method?: string;
  readonly params?: Record<string, unknown>;
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
  setDotLiveState(connected: boolean, working: boolean): void;
  /** `akra/chats/*` 의 조회 제한을 켜고 끈다 (plan 25). 처음 값은 `chatsRateLimited` 옵션이다. */
  setChatsRateLimited(limited: boolean): void;
  switchChatAccount(account: FakeChatAccount): void;
  /** 서버를 유지한 채 접속만 끊어 재접속을 시험한다. */
  disconnectClients(): void;
  /** 붙어 있는 local 커널 연결에만 상태 이벤트를 보낸다. */
  sendExtensionChange(kernel: string, change?: Record<string, unknown>): void;
  sendKernelEvent(event: unknown): void;
  sendProviderChange(change: unknown): void;
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
export type FakeKernelReply = (
  | { result: unknown; error?: never }
  | { error: { code: number; message: string }; result?: never }
) & {
  events?: unknown[];
  /** 선택한 시험 요청만 명시적으로 해제할 때까지 보류한다. 생략하면 기존 동기 순서다. */
  replyAfter?: Promise<void>;
};

export interface FakeKernel {
  /** 붙기 결과의 저장소 신원이다. null 이면 다른 프로세스가 잠금을 쥔 것처럼 -32012 로 거절한다. */
  kernel: string | null;
  /** 조회 종류 → 결과. 상태 이벤트와 목록 페이지는 서로 다른 응답이다. */
  queries: Record<string, unknown>;
  /** cursor/명령별 답이다. undefined이면 기본 가짜 응답을 사용한다. */
  onRequest?(method: string, params: Record<string, unknown>): FakeKernelReply | undefined;
  /** 붙은 뒤 첫 명령에 응답한 다음 보낼 이벤트이다. 초기 조회가 끝났다는 신호로 쓴다. */
  eventsAfterAttach?: unknown[];
}

function answerKernel(kernel: FakeKernel, socket: WebSocket, message: JsonRpcEnvelope & { params?: Record<string, unknown> }): boolean {
  const respond = (body: object) => socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, ...body }));
  const custom = kernel.onRequest?.(message.method ?? "", message.params ?? {});
  if (custom !== undefined) {
    const reply = () => {
      respond(custom.error ? { error: custom.error } : { result: custom.result });
      for (const event of custom.events ?? []) {
        socket.send(JSON.stringify({ jsonrpc: "2.0", method: "akra/kernel/event", params: { event } }));
      }
    };
    if (custom.replyAfter) {
      void custom.replyAfter.then(() => { if (socket.readyState === WebSocket.OPEN) reply(); });
    } else reply();
    return custom.error === undefined;
  }
  switch (message.method) {
    case "akra/kernel/attach":
      if (kernel.kernel === null) {
        respond({ error: { code: -32012, message: "the repository kernel runs in another process (pid 4242)" } });
        return false;
      }
      respond({ result: { client_id: "1", protocol_version: 1, kernel: kernel.kernel } });
      return true;
    case "akra/kernel/query": {
      const type = (message.params?.query as { type?: string } | undefined)?.type ?? "";
      respond(type in kernel.queries ? { result: kernel.queries[type] } : { error: { code: -32602, message: `unknown query ${type}` } });
      return true;
    }
    case "akra/kernel/command":
      respond({ result: { processed: true } });
      return true;
    default:
      respond({ result: {} });
      return true;
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
    onRequest?(method: string, params: Record<string, unknown>): void;
    /** Synthetic native-owner provider responses; never calls a real provider or credential store. */
    providerServices?(method: string, params: Record<string, unknown>): FakeKernelReply | Promise<FakeKernelReply>;
    /** app-server 요청의 합성 답이다 (예: thread/fork, #2625). undefined 를 돌려주면 다음 규칙(녹화)으로 답한다. */
    appServer?(method: string, params: Record<string, unknown>): Pick<JsonRpcEnvelope, "result" | "error"> | undefined;
    /** 화면의 정확한 대화 이동을 시험하는 읽기 응답이다. 생략하면 기존 녹화를 사용한다. */
    readThread?(params: Record<string, unknown>): unknown;
    mcpHub?: FakeMcpHub;
    threadDocuments?: Record<string, FakeThreadDocument[]>;
    /** `akra/git/graph` 의 답이다 (plan 27). 요청의 params 를 받아 쪽 하나를 돌려준다. 없으면 모르는 method 로 답한다. */
    gitGraph?(params: { cwd?: unknown; cursor?: unknown; limit?: unknown }): unknown;
    /** `akra/git/refs` 의 답이다 (plan 27 G2). 없으면 모르는 method 로 답한다. */
    gitRefs?(params: { cwd?: unknown }): unknown;
    /** `akra/git/commit` 의 답이다 (plan 27 G2). 없으면 모르는 method 로 답한다. */
    gitCommit?(params: { cwd?: unknown; commit?: unknown }): unknown;
    /** `akra/git/plan`, `execute`, `operations`, `undo` 의 답이다 (plan 27 G3). 없으면 모르는 method 로 답한다. */
    gitPlan?(params: { cwd?: unknown; operation?: unknown }): unknown;
    gitExecute?(params: { cwd?: unknown; operation?: unknown; expectation?: unknown }): unknown;
    gitOperations?(params: { cwd?: unknown }): unknown;
    gitUndo?(params: { cwd?: unknown; opId?: unknown }): unknown;
    /** `akra/issue/handoffCopy` 의 답이다 (#2428). 없으면 모르는 method 로 답한다. */
    issueHandoffCopy?(params: { owner?: unknown; repository?: unknown; number?: unknown; cwd?: unknown }): unknown;
    /** 이슈 수정/생성 메서드의 답이다 (#2426, #2436). 예외는 JSON-RPC 오류로 답하며 없으면 모르는 method 로 답한다. */
    issue?(method: string, params: Record<string, unknown>): unknown;
    /** `skills/list` 의 답이다 (#2383). 요청의 params 를 받는다. 없으면 녹화가 없는 method 로 답한다. */
    skills?(params: { cwds?: unknown; forceReload?: unknown }): unknown;
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
  } & FakeChatOptions = {},
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
  const kernelClients = new Set<WebSocket>();
  const localClients = new Set<WebSocket>();
  const extensionClients = new Set<WebSocket>();
  // app-server 처럼 서버 요청은 접속마다 따로 받고, 먼저 온 답만 쓴다 (Q-03 실측).
  const resolvedRequestIds = new Set<string>();
  /** 서버 요청 id → 그 요청을 받은 접속들과 스레드 id */
  const openServerRequests = new Map<string, { threadId: string; sockets: Set<WebSocket> }>();
  /** `akra/dots/watch` 로 방을 보는 접속이다. 그 접속에만 `akra/dots/live` 를 민다. */
  const watchedRooms = new Map<WebSocket, string>();
  let dotLiveConnected = true;
  const chats = new FakeChatHistory(options);
  const pushDotLive = (socket: WebSocket, working: boolean) => {
    const roomId = watchedRooms.get(socket);
    if (roomId === undefined || socket.readyState !== socket.OPEN) return;
    socket.send(JSON.stringify({ jsonrpc: "2.0", method: "akra/dots/live", params: { roomId, connected: dotLiveConnected, working, activityLabel: working ? "Thinking" : null, usingComputer: false } }));
  };
  sockets.on("connection", (socket, request) => {
    const path = request.url ?? "/";
    const host = path.startsWith("/host/") ? (path.slice("/host/".length).split("/")[0] ?? "") : "local";
    if (!hostIds.includes(host)) {
      socket.close(1008, "unknown host");
      return;
    }
    const remote = host !== "local";
    if (!remote) localClients.add(socket);
    socket.on("close", () => { kernelClients.delete(socket); extensionClients.delete(socket); localClients.delete(socket); });
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
      options.onRequest?.(message.method, message.params ?? {});
      const hasId = message.id !== undefined && message.id !== null;
      if (hasId && (message.method.startsWith("akra/providerCredentials/") || message.method.startsWith("akra/providers/"))) {
        if (remote || !options.providerServices) {
          socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, error: { code: -32601, message: "provider fixture unsupported" } }));
        } else {
          void Promise.resolve().then(() => options.providerServices!(message.method!, message.params ?? {})).then(reply => {
            if (socket.readyState === socket.OPEN) socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id,
              ...(reply.error ? { error: reply.error } : { result: reply.result }) }));
          }, () => {
            if (socket.readyState === socket.OPEN) socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id,
              error: { code: -32000, message: "synthetic provider fixture failed" } }));
          });
        }
        return;
      }
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
      const chatReply = remote && hasId ? chats.reply(message.method, message.params) : undefined;
      if (chatReply !== undefined) {
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, ...chatReply }));
        return;
      }
      if (remote && hasId && options.dotMessages && message.method === "akra/dots/messages") {
        // dot 은 가장 새 내 글까지 읽었다고 답한다.
        const mine = (options.dotMessages as Array<{ fromDot?: boolean; createdAt?: string }>).filter((item) => item.fromDot === false);
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { messages: options.dotMessages, dotReadAt: mine.at(-1)?.createdAt ?? null } }));
        return;
      }
      if (remote && hasId && options.dotMessages && message.method === "akra/dots/watch") {
        const roomId = String((message as { params?: { roomId?: unknown } }).params?.roomId ?? "");
        watchedRooms.set(socket, roomId);
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { watching: true } }));
        pushDotLive(socket, false);
        return;
      }
      if (remote && hasId && options.dotMessages && message.method === "akra/dots/send") {
        const params = (message as { params?: { text?: unknown } }).params;
        options.dotMessages.push({ id: `sent-${options.dotMessages.length}`, createdAt: "2026-11-14T02:00:00Z", author: "Me", fromDot: false, text: String(params?.text ?? "") });
        // dot 은 바로 답을 만들기 시작하고 조금 뒤에 답한다. 방을 보는 접속에는 실시간 상태를 민다. 답보다 먼저 중간
        // 메시지를 방에 넣는다. 실제 서비스처럼 그 메시지는 알리지 않는다.
        const messages = options.dotMessages;
        pushDotLive(socket, true);
        setTimeout(() => messages.push({ id: `interim-${messages.length}`, createdAt: "2026-11-14T02:00:01Z", author: "Mika", fromDot: true, text: "Let me check." }), 500);
        setTimeout(() => {
          messages.push({ id: `reply-${messages.length}`, createdAt: "2026-11-14T02:00:03Z", author: "Mika", fromDot: true, text: "On it." });
          pushDotLive(socket, false);
        }, 4_000);
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { sent: true } }));
        return;
      }
      if (options.hosts && hasId && message.method === "akra/gateway/info") {
        const recorded = (table.get(message.method)?.result ?? {}) as object;
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { ...recorded, host, hosts: hostIds.map((id) => ({ id })) } }));
        return;
      }
      if (options.kernel && message.method.startsWith("akra/kernel/")) {
        const accepted = answerKernel(options.kernel, socket, message);
        if (message.method === "akra/kernel/attach") {
          kernelEventsSent = false;
          if (accepted) kernelClients.add(socket);
          else kernelClients.delete(socket);
        }
        if (message.method === "akra/kernel/detach") kernelClients.delete(socket);
        if (message.method === "akra/kernel/extensions/attach" && accepted) extensionClients.add(socket);
        if (message.method === "akra/kernel/extensions/detach") extensionClients.delete(socket);
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
      if (options.issue && message.method.startsWith("akra/issue/") && message.method !== "akra/issue/handoffCopy") {
        const params = (message as { params?: Record<string, unknown> }).params ?? {};
        const answerIssue = options.issue;
        const method = message.method;
        void Promise.resolve()
          .then(() => answerIssue(method, params))
          .then((result) => {
            if (socket.readyState === socket.OPEN) socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result }));
          })
          .catch((error: unknown) => {
            if (socket.readyState === socket.OPEN) socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, error: { code: -32000, message: error instanceof Error ? error.message : String(error) } }));
          });
        return;
      }
      const gitAnswer = {
        "akra/git/graph": options.gitGraph,
        "akra/git/refs": options.gitRefs,
        "akra/git/commit": options.gitCommit,
        "akra/git/plan": options.gitPlan,
        "akra/git/execute": options.gitExecute,
        "akra/git/operations": options.gitOperations,
        "akra/git/undo": options.gitUndo,
        "akra/issue/handoffCopy": options.issueHandoffCopy,
        "skills/list": options.skills,
      }[message.method];
      if (gitAnswer) {
        const params = (message as { params?: Record<string, unknown> }).params ?? {};
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: gitAnswer(params) }));
        return;
      }
      if (options.mcpHub && message.method.startsWith("akra/mcp/")) {
        answerMcpHub(options.mcpHub, socket, message);
        return;
      }
      const appServerReply = hasId ? options.appServer?.(message.method, message.params ?? {}) : undefined;
      if (appServerReply) {
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, ...appServerReply }));
        return;
      }
      if (hasId && options.readThread && (message.method === "thread/resume" || message.method === "thread/read")) {
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: options.readThread(message.params ?? {}) }));
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
    setChatsRateLimited(limited) {
      chats.rateLimited = limited;
    },
    switchChatAccount(account) {
      chats.switchAccount(account);
    },
    setDotLiveState(connected, working) {
      dotLiveConnected = connected;
      for (const socket of watchedRooms.keys()) pushDotLive(socket, working);
    },
    answeredRequests,
    disconnectClients() {
      for (const client of sockets.clients) client.terminate();
    },
    sendExtensionChange(kernel, change = {}) {
      for (const socket of new Set([...kernelClients, ...extensionClients])) {
        if (socket.readyState === socket.OPEN) socket.send(JSON.stringify({ jsonrpc: "2.0", method: "akra/kernel/extensions/changed", params: { ...change, kernel } }));
      }
    },
    sendProviderChange(change) {
      for (const socket of localClients) if (socket.readyState === socket.OPEN) {
        socket.send(JSON.stringify({ jsonrpc: "2.0", method: "akra/providerCredentials/changed", params: change }));
      }
    },
    sendKernelEvent(event) {
      for (const socket of kernelClients) {
        if (socket.readyState === socket.OPEN) {
          socket.send(JSON.stringify({ jsonrpc: "2.0", method: "akra/kernel/event", params: { event } }));
        }
      }
    },
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
