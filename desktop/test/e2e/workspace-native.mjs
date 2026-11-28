// #2833: real Desktop renderer session, Gateway workspace route, Git/SQLite and Codex app-server.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, mkdir, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, relative } from "node:path";
import { promisify } from "node:util";
import { samePath } from "../../src/renderer/git/gitGraph.ts";
import { DesktopSession } from "../../src/renderer/session/desktopSession.ts";
import { desktopSessionStore, remoteHostStore } from "../../src/renderer/session/desktopStore.ts";
import { syntheticWorkspaceRollout } from "../fixtures/syntheticWorkspaceRollout.mjs";
import { workspaceLinePeer } from "../fixtures/workspaceLinePeer.mjs";

const { AKRA_WORKSPACE_FIXTURE_BIN: executable, AKRA_CODEX_ENTRY: codexEntry } = process.env;
assert(executable && codexEntry, "Set AKRA_WORKSPACE_FIXTURE_BIN and AKRA_CODEX_ENTRY to absolute paths.");
assert(isAbsolute(executable) && isAbsolute(codexEntry));
const root = await realpath(await mkdtemp(join(tmpdir(), "akra-workspace-desktop-")));
const home = join(root, "codex-home");
const cwd = join(root, "repo");
const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^GIT_|^AKRA_|^CODEX_/i.test(key)));
const run = promisify(execFile);
const git = async (directory, args) => (await run("git", args, {
  cwd: directory, env: { ...env, GIT_MASTER: "1", GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: join(root, "empty-git-config") },
  timeout: 10_000, windowsHide: true,
})).stdout.trim();
const rpc = method => `akra/conversationWorkspace/${method}`;
const allow = new Set(["thread/start", "thread/read", "thread/resume", "thread/list", "thread/turns/list",
  "thread/unsubscribe", "model/list", "account/read", "account/rateLimits/read"]);
const store = remoteHostStore("workspace-native-" + crypto.randomUUID());
const calls = [];
const starts = [];
const allocations = [];
let fault = "prepare-reply-lost";
let session;
let native;
let codex;
const upstream = () => workspaceLinePeer("node", [codexEntry, "app-server"], {
  cwd: root, env: { ...env, CODEX_HOME: home },
});

function awaitState(predicate) {
  if (predicate(store.getState())) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const stop = store.subscribe(state => {
      if (!predicate(state)) return;
      clearTimeout(deadline);
      stop();
      resolve();
    });
    const deadline = setTimeout(() => {
      stop();
      const { sessionError, transcriptError, openThreadId } = store.getState();
      reject(new Error(`Desktop state did not settle: ${JSON.stringify({ sessionError, transcriptError, openThreadId })}`));
    }, 30_000);
  });
}

try {
  await mkdir(home);
  await mkdir(cwd);
  await git(cwd, ["init", "-b", "main"]);
  await git(cwd, ["-c", "user.name=Workspace Fixture", "-c", "user.email=workspace@example.test",
    "commit", "--allow-empty", "-m", "base"]);
  const baseCommit = await git(cwd, ["rev-parse", "HEAD"]);
  native = workspaceLinePeer(executable, ["--ignored", "--exact",
    "adapter::inbound::gateway::conversation_workspace::tests::desktop_fixture::desktop_workspace_fixture_bridge",
    "--nocapture", "--test-threads=1"], {
    cwd: root, prefix: "AKRA_WORKSPACE_FIXTURE ", env: { ...env, GIT_MASTER: "1", AKRA_WORKSPACE_FIXTURE_SERVER: "1" },
  });
  codex = upstream();
  const received = new Set();
  const connections = new Set();
  const emit = frame => { for (const listener of received) listener(JSON.stringify(frame)); };
  codex.onNotification(emit);
  const networkError = message => ({ error: { code: -32000, message } });
  const dispatch = async frame => {
    const { id, method, params = {} } = frame;
    calls.push({ method, params });
    if (id === undefined) {
      if (method === "initialized") codex.notify(method, params);
      return;
    }
    if (method === "akra/gateway/info") return { result: { gatewayVersion: "fixture", codexVersion: "codex-cli 0.171.0", hosts: [{ id: "local" }] } };
    if (method === "akra/git/graph") return { result: { repository: true, stale: false, head: baseCommit, refs: [], rows: [], worktrees: [], next: null } };
    if ([rpc("get"), rpc("prepare"), rpc("inventory")].includes(method)) {
      const reply = await native.request(method, params);
      if (method === rpc("prepare") && reply.result) {
        allocations.push(reply.result);
        if (fault === "prepare-reply-lost") {
          fault = undefined;
          return networkError("fixture lost committed prepare reply");
        }
      }
      return reply.error ? { error: reply.error } : { result: reply.result };
    }
    if (method !== "initialize" && !allow.has(method)) {
      return { error: { code: -32601, message: `Fixture refuses ${method}` } };
    }
    const reply = await codex.request(method, params);
    if (method === "thread/start" && reply.result) {
      starts.push({ requestedCwd: params.cwd, thread: reply.result.thread });
      if (fault === "start-reply-lost") return networkError("fixture lost accepted thread/start reply");
    }
    if (method === "thread/resume" && fault === "wrong-resume-cwd" && reply.result) {
      return { result: { ...reply.result, thread: { ...reply.result.thread, cwd } } };
    }
    return reply.error ? { error: reply.error } : { result: reply.result };
  };
  session = new DesktopSession({
    send(text) {
      const frame = JSON.parse(text);
      void dispatch(frame).then(reply => {
        if (reply) emit({ jsonrpc: "2.0", id: frame.id, ...reply });
      }, error => emit({ jsonrpc: "2.0", id: frame.id, ...networkError(String(error)) }));
    },
    onMessage(listener) { received.add(listener); return () => received.delete(listener); },
    onConnectionState(listener) { connections.add(listener); return () => connections.delete(listener); },
    ready() {},
  }, desktopSessionStore(store), "fixture", callback => callback());
  const connect = phase => { for (const listener of connections) listener({ phase, endpoint: "native-fixture" }); };
  session.start();
  connect("connected");
  await awaitState(state => state.gateway !== null);

  const lostPrepare = session.startIsolatedThread(cwd);
  await assert.rejects(session.startIsolatedThread(cwd), /진행 중/);
  await assert.rejects(lostPrepare, /lost committed prepare reply/);
  assert.equal(starts.length, 0);
  const first = await session.startIsolatedThread(cwd);
  assert.deepEqual(calls.filter(call => call.method === rpc("get")).map(call => call.params.requestId),
    [allocations[0].requestId, allocations[0].requestId]);
  assert.equal(allocations.length, 1);
  const second = await session.startIsolatedThread(cwd);
  assert.notEqual(first, second);
  assert.equal(allocations.length, 2);
  assert.notEqual(allocations[0].workspaceDir, allocations[1].workspaceDir);
  assert.notEqual(allocations[0].branch, allocations[1].branch);
  for (const [index, id] of [first, second].entries()) {
    const workspace = allocations[index];
    const { thread, requestedCwd } = starts[index];
    const saved = await codex.request("thread/read", { threadId: id, includeTurns: false });
    assert.equal(saved.error, undefined, JSON.stringify(saved));
    assert.equal(thread.id, id);
    assert(samePath(requestedCwd, workspace.workspaceDir));
    assert(samePath(saved.result.thread.cwd, workspace.workspaceDir));
    assert.equal(await git(workspace.workspaceDir, ["branch", "--show-current"]), workspace.branch);
    const folder = store.getState().folders.find(folder => samePath(folder.cwd, workspace.workspaceDir));
    assert.equal(folder?.threads.find(thread => thread.id === id)?.branch, workspace.branch);
    console.log("PASS thread/worktree/branch", JSON.stringify({ id, workspaceDir: workspace.workspaceDir, branch: workspace.branch }));
  }
  console.log("PASS duplicate click and lost prepare reply use one allocation and one recovered request ID");

  await session.openThread(first);
  assert.equal(store.getState().transcript, null);
  assert(store.getState().transcriptError);
  connect("disconnected");
  await codex.close();
  for (const { thread } of starts) {
    assert.equal(typeof thread.path, "string");
    const belowHome = relative(home, thread.path);
    assert(belowHome && !belowHome.startsWith("..") && !isAbsolute(belowHome));
    await mkdir(dirname(thread.path), { recursive: true });
    await writeFile(thread.path, syntheticWorkspaceRollout(thread));
  }
  codex = upstream();
  codex.onNotification(emit);
  const reopened = awaitState(state => state.transcript?.threadId === first
    && samePath(state.transcript.cwd, allocations[0].workspaceDir));
  connect("connected");
  await reopened;
  assert.equal(starts.length, 2);
  console.log("PASS app-server restart and Desktop reconnect reopen the original thread/worktree from synthetic saved history");

  fault = "wrong-resume-cwd";
  await session.reopenThread();
  assert.equal(store.getState().transcript, null);
  await assert.rejects(session.sendMessage("fixture must not transmit"), /격리|작업 공간|scope/);
  fault = undefined;
  await session.reopenThread();
  assert(samePath(store.getState().transcript.cwd, allocations[0].workspaceDir));
  fault = "start-reply-lost";
  await assert.rejects(session.startIsolatedThread(cwd), /lost accepted thread\/start reply/);
  const createCount = calls.filter(call => call.method === "thread/start").length;
  await assert.rejects(session.startIsolatedThread(cwd), /결과가 불명/);
  assert.equal(createCount, 3);
  assert.equal(calls.filter(call => call.method === "thread/start").length, createCount);
  assert.equal(store.getState().openThreadId, first);
  assert(calls.filter(call => call.method === "thread/start").every(call => !samePath(call.params.cwd, cwd)));
  assert(!calls.some(call => call.method.startsWith("turn/")));
  const inventory = await native.request(rpc("inventory"), { cwd });
  assert.equal(inventory.result.items.length, 3);
  assert(inventory.result.items.every(item => item.cleanupAllowed === false));
  console.log("PASS wrong-scope reconnect and lost start hold state without source fallback, replay, cleanup, or a model turn");
} finally {
  session?.stop();
  try {
    await Promise.all([codex?.close(), native?.close()]);
  } finally {
    await rm(root, { recursive: true, force: true, maxRetries: 5 });
  }
}
