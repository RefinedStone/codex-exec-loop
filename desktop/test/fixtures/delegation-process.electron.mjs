import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { mkdtempSync } from "node:fs";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { app } from "electron";
import { WebSocketServer } from "ws";

import { startDelegatedChatProcess } from "../../src/main/delegatedChatProcess.ts";
import { delegatedTunnelStatus } from "../../src/main/delegatedTunnelStatus.ts";

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
  const runtime = join(temp, "runtime", "cokacremote");
  const runtimeEntry = join(runtime, "dist", "src", "stdio-server.js");
  const bytes = "throw new Error('Status must not execute a runtime artifact.');";
  await mkdir(join(runtime, "dist", "src"), { recursive: true });
  await writeFile(runtimeEntry, bytes);
  await writeFile(join(runtime, ".akra-runtime.json"), JSON.stringify({
    schemaVersion: 23, entrySha256: createHash("sha256").update(bytes).digest("hex"),
  }));
  const entry = fileURLToPath(new URL("../../out/main/delegatedChatGateway.js", import.meta.url));
  const close = startDelegatedChatProcess(entry, endpoint, async (method, id, instruction, options, report) => {
    calls.push([method, id, instruction, options]);
    if (method === "submit") return { delivery: "accepted", phase: "working" };
    if (method === "observe") {
      report({ type: "checkpoint", message: "Fixture progress" });
      return { delivery: "observed", phase: "working" };
    }
    if (method === "readResult") {
      report({ type: "result", result: "Fixture answer" });
      return { text: "Fixture answer" };
    }
    return { state: "ready", tunnel: await delegatedTunnelStatus(temp) };
  });
  try {
    const { socket, registration } = await registered;
    assert.equal(registration.method, "akra/delegationDriver/register");
    stage = "Gateway reply";
    const reply = once(socket, "message");
    socket.send(JSON.stringify({ jsonrpc: "2.0", id: "status", method: "akra/delegationDriver/status" }));
    assert.deepEqual(JSON.parse((await reply)[0].toString()), {
      jsonrpc: "2.0", id: "status", result: { state: "ready", tunnel: { install: "ready", connection: "unknown" } },
    });
    assert.deepEqual(calls, [["status", "", undefined, undefined]]);
    await writeFile(runtimeEntry, "modified");
    const secondReply = once(socket, "message");
    socket.send(JSON.stringify({ jsonrpc: "2.0", id: "invalid", method: "akra/delegationDriver/status" }));
    assert.deepEqual(JSON.parse((await secondReply)[0].toString()).result.tunnel, { install: "invalid", connection: "unknown" });
    stage = "private job callbacks";
    const token = "a".repeat(64);
    const seen = [];
    socket.on("message", (data) => {
      const message = JSON.parse(data.toString());
      if (message.method === "akra/kernel/attach") {
        assert.equal(message.params.workspace, temp);
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { kernel: "fixture" } }));
      } else if (message.method === "akra/kernel/command") {
        seen.push(message.params.command);
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { processed: true } }));
      }
    });
    const exchange = async (method, params) => {
      const answer = new Promise((resolve) => {
        const listener = (data) => {
          const message = JSON.parse(data.toString());
          if (message.id !== method || message.method) return;
          socket.off("message", listener);
          resolve(message);
        };
        socket.on("message", listener);
      });
      socket.send(JSON.stringify({ jsonrpc: "2.0", id: method, method: `akra/delegationDriver/${method}`, params }));
      return answer;
    };
    const operation = Promise.all([
      new Promise((resolve) => {
        const listener = (data) => {
          const message = JSON.parse(data.toString());
          if (message.params?.command?.type !== "report_delegation_result") return;
          socket.off("message", listener);
          resolve();
        };
        socket.on("message", listener);
      }),
      (async () => {
        assert.ok((await exchange("submit", {
          jobId: "attempt-2", instruction: "Fixture prompt",
          callback: { jobId: "logical-job", token, workspace: temp },
        })).result);
        assert.ok((await exchange("observe", { jobId: "attempt-2" })).result);
        assert.deepEqual((await exchange("readResult", { jobId: "attempt-2" })).result, { text: "Fixture answer" });
      })(),
    ]);
    await operation;
    assert.ok(!JSON.stringify(calls).includes(token), "Capability must stay in the utility process");
    assert.deepEqual(seen.map(({ token: credential, ...command }) => {
      assert.ok(credential === token, "Expected the job-scoped credential");
      return command;
    }), [
      { type: "checkpoint_delegation", id: "logical-job", message: "Fixture progress" },
      { type: "report_delegation_result", id: "logical-job", result: "Fixture answer" },
    ]);
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
