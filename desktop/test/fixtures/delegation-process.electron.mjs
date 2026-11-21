import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdtempSync } from "node:fs";
import { rm } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { app } from "electron";
import { WebSocketServer } from "ws";

import { startDelegatedChatProcess } from "../../src/main/delegatedChatProcess.ts";

const temp = mkdtempSync(join(tmpdir(), "akra-delegation-process-"));
app.setPath("userData", temp);
const endpoint = process.platform === "win32" ? `\\\\.\\pipe\\akra-delegation-worker-${process.pid}` : join(temp, "gateway.sock");
const server = createServer();
const gateway = new WebSocketServer({ server });
let stage = "Electron startup";
const timeout = setTimeout(() => {
  console.error(`Delegation utility process fixture timed out during ${stage}`);
  app.exit(1);
}, 10_000);

void app.whenReady().then(async () => {
  server.listen(endpoint);
  await once(server, "listening");
  stage = "Gateway registration";
  const registered = new Promise((resolve) => gateway.once("connection", (socket) => {
    socket.once("message", (data) => resolve({ socket, registration: JSON.parse(data.toString()) }));
  }));
  const calls = [];
  const entry = fileURLToPath(new URL("../../out/main/delegatedChatGateway.js", import.meta.url));
  const close = startDelegatedChatProcess(entry, endpoint, async (...args) => {
    calls.push(args);
    return { state: "ready" };
  });
  try {
    const { socket, registration } = await registered;
    assert.equal(registration.method, "akra/delegationDriver/register");
    stage = "Gateway reply";
    const reply = once(socket, "message");
    socket.send(JSON.stringify({ jsonrpc: "2.0", id: "status", method: "akra/delegationDriver/status" }));
    assert.deepEqual(JSON.parse((await reply)[0].toString()), {
      jsonrpc: "2.0", id: "status", result: { state: "ready" },
    });
    assert.deepEqual(calls, [["status", "", undefined]]);
    stage = "Gateway unregister and worker exit";
    const unregister = once(socket, "message");
    await close();
    assert.equal(JSON.parse((await unregister)[0].toString()).method, "akra/delegationDriver/unregister");
  } finally {
    await close();
  }
  console.log("Delegation utility process fixture passed");
}).catch((error) => {
  console.error(error);
  process.exitCode = 1;
}).finally(async () => {
  clearTimeout(timeout);
  gateway.clients.forEach((socket) => socket.terminate());
  gateway.close();
  server.close();
  await rm(temp, { recursive: true, force: true });
  app.exit(process.exitCode ?? 0);
});
