// #2625: real Codex, two writers, synthetic persisted history, no model turns.
// Fixture shape follows openai/codex rust-v0.171.0 app-server/tests/common/rollout.rs.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { createInterface } from "node:readline";

const entrypoint = process.env.AKRA_CODEX_ENTRY;
assert(entrypoint, "Set AKRA_CODEX_ENTRY to the installed Codex JavaScript entrypoint.");
const root = await mkdtemp(join(tmpdir(), "akra-native-fork-"));
const home = join(root, "codex-home");
const cwd = join(root, "workspace");
await mkdir(cwd);
const sourceId = randomUUID();
const sourcePath = join(home, "sessions", "2025", "01", "05", `rollout-2025-01-05T12-00-00-${sourceId}.jsonl`);
await mkdir(dirname(sourcePath), { recursive: true });
const timestamp = "2025-01-05T12:00:00Z";
const line = (type, payload) => JSON.stringify({ timestamp, type, payload });
const history = [
  line("session_meta", { id: sourceId, session_id: sourceId, timestamp, cwd, originator: "codex",
    cli_version: "0.171.0", source: "cli", model_provider: "openai", base_instructions: { text: "Synthetic verification fixture." } }),
];
for (const turnId of ["fixture-turn-1", "fixture-turn-2"]) {
  history.push(
    line("event_msg", { type: "task_started", turn_id: turnId, model_context_window: null }),
    line("response_item", { type: "message", role: "user", content: [{ type: "input_text", text: `Question ${turnId}` }] }),
    line("event_msg", { type: "user_message", message: `Question ${turnId}`, images: [], local_images: [] }),
    line("response_item", { type: "message", role: "assistant", content: [{ type: "output_text", text: `Answer ${turnId}` }] }),
    line("event_msg", { type: "agent_message", message: `Answer ${turnId}`, phase: "final_answer" }),
    line("event_msg", { type: "task_complete", turn_id: turnId, last_agent_message: `Answer ${turnId}` }),
  );
}
await writeFile(sourcePath, history.join("\n") + "\n");
await writeFile(join(cwd, "sentinel.txt"), "Keep the original worktree.\n");
const allowed = new Set(["initialize", "thread/resume", "thread/read", "thread/turns/list", "thread/fork", "thread/archive", "thread/unarchive"]);
const peers = [];
const calls = [];

function connect() {
  const child = spawn(process.execPath, [entrypoint, "app-server"], {
    env: { ...process.env, CODEX_HOME: home }, cwd, windowsHide: true, stdio: ["pipe", "pipe", "pipe"],
  });
  const pending = new Map();
  let sequence = 0;
  let diagnostics = "";
  child.stderr.on("data", bytes => { diagnostics = (diagnostics + bytes).slice(-4096); });
  const exited = new Promise(resolve => child.once("close", resolve));
  const lines = createInterface({ input: child.stdout });
  lines.on("line", text => {
    const frame = JSON.parse(text);
    const waiting = pending.get(frame.id);
    if (waiting) {
      pending.delete(frame.id);
      clearTimeout(waiting.timer);
      waiting.resolve(frame);
    }
  });
  const request = (method, params) => {
    assert(allowed.has(method), "The fixture forbids " + method);
    calls.push({ method, params });
    const id = ++sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error(`Timed out: ${method}\n${diagnostics}`));
      }, 30_000);
      pending.set(id, { resolve, timer });
      child.stdin.write(JSON.stringify({ id, method, params }) + "\n");
    });
  };
  const peer = {
    child, exited, request,
    async initialize() {
      const result = await request("initialize", { clientInfo: { name: "akra_native_fork_fixture", version: "1" }, capabilities: { experimentalApi: true } });
      assert.equal(result.error, undefined, JSON.stringify(result));
      child.stdin.write(JSON.stringify({ method: "initialized", params: {} }) + "\n");
    },
    async close() { child.stdin.end(); await exited; },
  };
  peers.push(peer);
  return peer;
}

try {
  const owner = connect();
  await owner.initialize();
  const resumed = await owner.request("thread/resume", { threadId: sourceId, cwd, approvalPolicy: "never", sandbox: "read-only" });
  assert.equal(resumed.error, undefined, JSON.stringify(resumed));
  assert.equal(resumed.result.thread.id, sourceId);

  const client = connect();
  await client.initialize();
  const denied = await client.request("thread/resume", { threadId: sourceId });
  assert.match(denied.error?.message ?? "", /already has an active writer/);
  const readable = await client.request("thread/read", { threadId: sourceId, includeTurns: true });
  assert.equal(readable.error, undefined, JSON.stringify(readable));
  assert.deepEqual(readable.result.thread.turns.map(turn => turn.id), ["fixture-turn-1", "fixture-turn-2"]);
  const beforeFork = await readFile(sourcePath, "utf8");
  const response = await client.request("thread/fork", { threadId: sourceId, lastTurnId: "fixture-turn-2", excludeTurns: true, ephemeral: false });
  assert.equal(response.error, undefined, JSON.stringify(response));
  const fork = response.result.thread;
  assert.notEqual(fork.id, sourceId);
  assert.equal(fork.forkedFromId, sourceId);
  assert.equal(fork.cwd, cwd);
  assert.deepEqual(fork.turns, []);
  const turns = await client.request("thread/turns/list", { threadId: fork.id, sortDirection: "asc", itemsView: "full", limit: 20 });
  assert.equal(turns.error, undefined, JSON.stringify(turns));
  assert.deepEqual(turns.result.data.map(turn => turn.id), ["fixture-turn-1", "fixture-turn-2"]);
  assert.equal(await readFile(sourcePath, "utf8"), beforeFork);
  assert.equal(await readFile(join(cwd, "sentinel.txt"), "utf8"), "Keep the original worktree.\n");
  console.log("PASS independent persistent fork preserves the active source writer, stable history, lineage, and cwd");
  await client.close();
  await owner.close();

  const reopened = connect();
  await reopened.initialize();
  const loaded = await reopened.request("thread/read", { threadId: fork.id, includeTurns: true });
  assert.equal(loaded.error, undefined, JSON.stringify(loaded));
  assert.equal(loaded.result.thread.forkedFromId, sourceId);
  assert.deepEqual(loaded.result.thread.turns.map(turn => turn.id), ["fixture-turn-1", "fixture-turn-2"]);
  const archived = await reopened.request("thread/archive", { threadId: fork.id });
  assert.equal(archived.error, undefined, JSON.stringify(archived));
  const restored = await reopened.request("thread/unarchive", { threadId: fork.id });
  assert.equal(restored.error, undefined, JSON.stringify(restored));
  assert.equal(restored.result.thread.id, fork.id);
  assert.equal(restored.result.thread.forkedFromId, sourceId);
  assert.equal(calls.filter(call => call.method === "thread/fork").length, 1);
  console.log("PASS fork reload, archive, and unarchive retain identity without retry or model turns");
  await reopened.close();
} finally {
  for (const peer of peers) {
    if (peer.child.exitCode !== null) continue;
    if (process.platform === "win32") {
      const stop = spawn("taskkill.exe", ["/PID", String(peer.child.pid), "/T", "/F"], { env: process.env, windowsHide: true, stdio: "ignore" });
      await new Promise(resolve => stop.once("close", resolve));
    } else peer.child.kill();
    await peer.exited;
  }
  await rm(root, { recursive: true, force: true, maxRetries: 5 });
}
