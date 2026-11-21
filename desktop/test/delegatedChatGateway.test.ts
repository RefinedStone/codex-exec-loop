import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { expect, test, vi } from "vitest";
import { WebSocketServer, type WebSocket } from "ws";

import { connectDelegatedChat } from "../src/delegation/gateway";
import type { ChatDispatchOptions } from "../src/delegation/messages";

test("the dedicated local socket registers, handles only chat callbacks, and unregisters", async () => {
  const temp = await mkdtemp(join(tmpdir(), "akra-chat-gateway-"));
  const endpoint = process.platform === "win32" ? `\\\\.\\pipe\\akra-chat-test-${process.pid}-${Date.now()}` : join(temp, "gateway.sock");
  const server = createServer();
  const gateway = new WebSocketServer({ server });
  server.listen(endpoint);
  await once(server, "listening");
  const dispatch = vi.fn().mockResolvedValue({ state: "ready" });
  const established = new Promise<WebSocket>((resolve) => gateway.once("connection", (ws) => {
    ws.once("message", (message) => {
      expect(JSON.parse(message.toString())).toMatchObject({ method: "akra/delegationDriver/register", params: {} });
      resolve(ws);
    });
  }));
  const connection = connectDelegatedChat({ endpoint }, dispatch);
  try {
    const ws = await established;
    const reply = once(ws, "message");
    ws.send(JSON.stringify({ jsonrpc: "2.0", id: "fixture-1", method: "akra/delegationDriver/status", params: {} }));
    expect(JSON.parse((await reply)[0].toString())).toEqual({ jsonrpc: "2.0", id: "fixture-1", result: { state: "ready" } });
    const denied = once(ws, "message");
    ws.send(JSON.stringify({ jsonrpc: "2.0", id: "fixture-2", method: "akra/delegationDriver/evaluate", params: {} }));
    expect(JSON.parse((await denied)[0].toString())).toMatchObject({ id: "fixture-2", error: { code: -32602 } });
    expect(dispatch).toHaveBeenCalledExactlyOnceWith("status", "", undefined, undefined);
    const chatUrl = "https://chatgpt.com/c/12345678-1234-1234-1234-123456789abc";
    const options = { session: { mode: "continue", chatUrl }, expectedAccountId: "fixture-account", maxResumes: 1 } satisfies ChatDispatchOptions;
    const submit = (id: string, extra: object) => {
      const response = once(ws, "message");
      ws.send(JSON.stringify({
        jsonrpc: "2.0", id, method: "akra/delegationDriver/submit",
        params: { jobId: "attempt-A", instruction: "Fixture", ...extra },
      }));
      return response.then(([wire]) => JSON.parse(wire.toString()));
    };
    expect(await submit("unsafe", { ...options, session: { mode: "continue", chatUrl: "file:///etc/passwd" } }))
      .toMatchObject({ error: { code: -32602 } });
    expect(await submit("unbounded", { ...options, maxResumes: 3 })).toMatchObject({ error: { code: -32602 } });
    expect(await submit("selected", options)).toMatchObject({ result: { state: "ready" } });
    expect(dispatch).toHaveBeenCalledTimes(2);
    expect(dispatch).toHaveBeenLastCalledWith("submit", "attempt-A", "Fixture", options);
    const unregister = once(ws, "message");
    connection.close();
    expect(JSON.parse((await unregister)[0].toString())).toMatchObject({ method: "akra/delegationDriver/unregister", params: {} });
  } finally {
    connection.close();
    gateway.clients.forEach((ws) => ws.terminate());
    gateway.close();
    server.close();
    await rm(temp, { recursive: true, force: true });
  }
});
