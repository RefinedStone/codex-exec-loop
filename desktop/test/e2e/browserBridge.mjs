import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, access, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";

const executable = process.env.AKRA_GATEWAY_BIN;
assert.ok(executable, "AKRA_GATEWAY_BIN must point to the built native gateway");
const home = await mkdtemp(join(tmpdir(), "akra-browser-bridge-"));
const processes = [];
const deadline = setTimeout(() => {
  for (const child of processes) if (child.exitCode === null) child.kill();
  console.error("Native browser bridge test timed out");
  process.exitCode = 1;
}, 90_000);
function peer(command, args, env) {
  const child = spawn(command, args, { env: { ...process.env, ...env }, stdio: "pipe", windowsHide: true });
  processes.push(child);
  const messages = [];
  const listeners = new Set();
  let errorText = "";
  child.stderr.on("data", (chunk) => { errorText += chunk; });
  const lines = createInterface({ input: child.stdout });
  lines.on("line", (line) => {
    const frame = JSON.parse(line);
    messages.push(frame);
    for (const listener of listeners) listener(frame);
  });
  const exited = new Promise((resolveExit) => child.once("close", (code) => resolveExit(code)));
  const next = (predicate) => {
    const existing = messages.find(predicate);
    if (existing) return Promise.resolve(existing);
    return new Promise((resolveFrame, reject) => {
      const timer = setTimeout(() => { listeners.delete(listener); reject(new Error(`Frame timeout: ${errorText}`)); }, 30_000);
      const listener = (frame) => {
        if (!predicate(frame)) return;
        clearTimeout(timer);
        listeners.delete(listener);
        resolveFrame(frame);
      };
      listeners.add(listener);
    });
  };
  return { child, exited, next, send: (frame) => child.stdin.write(`${JSON.stringify(frame)}\n`) };
}

try {
  const broker = peer(executable, ["desktop-browser-bridge"], { AKRA_HOME: home });
  await broker.next((frame) => frame.type === "ready");
  broker.send({ id: "prepare", command: "prepare" });
  const { result: prepared } = await broker.next((frame) => frame.id === "prepare");
  assert.ok(prepared.sessionFile);
  const helper = peer(process.execPath, [resolve("out/main/browserMcp.js")], { AKRA_BROWSER_SESSION_FILE: prepared.sessionFile });
  helper.send({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "bridge-test", version: "1" } } });
  assert.ok((await helper.next((frame) => frame.id === 1)).result);
  helper.send({ jsonrpc: "2.0", id: 2, method: "tools/list" });
  const tools = (await helper.next((frame) => frame.id === 2)).result.tools;
  assert.equal(tools.length, 16);
  if (process.env.AKRA_CODEX_ENTRY) {
    broker.send({ id: "codex-prepare", command: "prepare" });
    const session = (await broker.next((frame) => frame.id === "codex-prepare")).result;
    const codex = peer(process.execPath, [process.env.AKRA_CODEX_ENTRY, "app-server"], { CODEX_HOME: home });
    codex.send({ id: 1, method: "initialize", params: { clientInfo: { name: "browser_fixture", version: "1" }, capabilities: { experimentalApi: true } } });
    assert.ok((await codex.next((frame) => frame.id === 1)).result);
    codex.send({ method: "initialized", params: {} });
    codex.send({ id: 2, method: "thread/start", params: {
      cwd: process.cwd(), ephemeral: true,
      config: {
        "mcp_servers.akra_browser.command": process.execPath,
        "mcp_servers.akra_browser.args": [resolve("out/main/browserMcp.js")],
        "mcp_servers.akra_browser.env.AKRA_BROWSER_SESSION_FILE": session.sessionFile,
        "mcp_servers.akra_browser.env.ELECTRON_RUN_AS_NODE": "1",
      },
    } });
    const started = await codex.next((frame) => frame.id === 2);
    assert.ok(started.result?.thread?.id, JSON.stringify(started));
    broker.send({ id: "codex-bind", command: "bind", registrationId: session.registrationId, threadId: started.result.thread.id });
    assert.deepEqual((await broker.next((frame) => frame.id === "codex-bind")).result, {});
    codex.send({ id: 3, method: "mcpServerStatus/list", params: { threadId: started.result.thread.id, detail: "toolsAndAuthOnly" } });
    const status = await codex.next((frame) => frame.id === 3);
    const browser = status.result?.data.find((entry) => entry.name === "akra_browser");
    assert.ok(browser, JSON.stringify(status));
    assert.equal(Object.keys(browser.tools).length, 16);
    console.log("PASS: actual Codex thread/start and thread-scoped browser tool discovery without a model turn", browser.runtimeStatus);
    broker.send({ id: "codex-revoke", command: "revoke", registrationId: session.registrationId });
    await broker.next((frame) => frame.id === "codex-revoke");
    codex.child.stdin.end();
    assert.equal(await codex.exited, 0);
  }
  broker.send({ id: "bind", command: "bind", registrationId: prepared.registrationId, threadId: "fixture-thread" });
  assert.deepEqual((await broker.next((frame) => frame.id === "bind")).result, {});
  helper.send({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "browser_status", arguments: {}, _meta: { threadId: "foreign-thread" } } });
  assert.ok((await helper.next((frame) => frame.id === 3)).error);
  helper.send({ jsonrpc: "2.0", id: 4, method: "tools/call", params: { name: "browser_status", arguments: {}, _meta: { threadId: "fixture-thread" } } });
  const call = await broker.next((frame) => frame.type === "call");
  assert.equal(call.threadId, "fixture-thread");
  assert.equal(call.tool, "browser_status");
  assert.equal("token" in call, false);
  const result = { content: [{ type: "text", text: "verified native round trip" }] };
  broker.send({ command: "answer", requestId: call.requestId, result });
  assert.deepEqual((await helper.next((frame) => frame.id === 4)).result, result);
  broker.send({ id: "revoke", command: "revoke", registrationId: prepared.registrationId });
  assert.deepEqual((await broker.next((frame) => frame.id === "revoke")).result, {});
  assert.equal(await helper.exited, 0);
  await assert.rejects(access(prepared.sessionFile), { code: "ENOENT" });
  broker.child.stdin.end();
  assert.equal(await broker.exited, 0);
  console.log("PASS: native broker + bundled MCP, 16 schemas, thread rejection, round trip, revoke, helper exit, sidecar cleanup");
} finally {
  clearTimeout(deadline);
  for (const child of processes) if (child.exitCode === null) child.kill();
  await rm(home, { recursive: true, force: true });
}
