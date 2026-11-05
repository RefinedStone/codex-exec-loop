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

export async function startFakeGateway(endpoint: string, recordingPath: string): Promise<FakeGateway> {
  const table = loadReplayTable(recordingPath);
  const receivedMethods: string[] = [];
  const server: Server = createServer();
  const sockets = new WebSocketServer({ server });
  sockets.on("connection", (socket) => {
    socket.on("message", (data) => {
      const message = JSON.parse(data.toString()) as JsonRpcEnvelope;
      if (!message.method) {
        return;
      }
      receivedMethods.push(message.method);
      if (message.id === undefined || message.id === null) {
        return;
      }
      const recorded = table.get(message.method) ?? {
        error: { code: -32601, message: `fake gateway has no recording for ${message.method}` },
      };
      socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, ...recorded }));
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
