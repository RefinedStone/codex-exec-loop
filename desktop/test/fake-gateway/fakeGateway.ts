// 가짜 Gateway: Gateway 녹화 파일(JSONL, P1 §4.6)을 읽어 요청 method 에 맞는 응답을 재생한다.
// unix 소켓이나 Windows named pipe 에서 WebSocket 을 받는다. 턴을 쓰지 않고 화면과 접속을 시험한다 (D-13).
import { readFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { WebSocketServer } from "ws";

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
  readonly receivedMethods: string[];
  close(): Promise<void>;
}

/**
 * 녹화의 이어지는 gateway 메시지이다. 클라이언트가 요청(method)을 보내거나 서버 요청에 답(response id)하면,
 * 녹화에서 그 다음에 왔던 알림과 서버 요청을 같은 순서로 보낸다.
 */
interface ReplaySegment {
  trigger: string;
  followUps: string[];
  used: boolean;
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
      current = key ? { trigger: key, followUps: [], used: false } : null;
      if (current) {
        segments.push(current);
      }
    } else if (current && message.method) {
      current.followUps.push(record.text);
    }
  }
  return segments;
}

/** 녹화를 여러 개 주면 응답 표는 앞의 녹화가 우선하고, 이어지는 메시지는 순서대로 이어 붙인다. */
export async function startFakeGateway(endpoint: string, recordingPaths: string | string[]): Promise<FakeGateway> {
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
  const server: Server = createServer();
  const sockets = new WebSocketServer({ server });
  sockets.on("connection", (socket) => {
    const replayFollowUps = (message: JsonRpcEnvelope) => {
      const key = triggerKey(message);
      const segment = segments.find((candidate) => !candidate.used && candidate.trigger === key);
      if (!segment) {
        return;
      }
      segment.used = true;
      for (const text of segment.followUps) {
        socket.send(text);
      }
    };
    socket.on("message", (data) => {
      const message = JSON.parse(data.toString()) as JsonRpcEnvelope;
      if (!message.method) {
        // 서버 요청(승인 등)에 대한 클라이언트 답이다. 녹화에서 그 뒤에 온 알림을 이어서 보낸다.
        replayFollowUps(message);
        return;
      }
      receivedMethods.push(message.method);
      if (message.id !== undefined && message.id !== null) {
        const recorded = table.get(message.method) ?? {
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
