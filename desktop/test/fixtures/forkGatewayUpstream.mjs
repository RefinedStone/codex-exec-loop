// Scripted Codex protocol peer. The production Gateway owns the client-facing transport.
import assert from "node:assert/strict";
import { existsSync, readFileSync, watch, writeFileSync, appendFileSync } from "node:fs";
import { createServer } from "node:http";
import { createConnection } from "node:net";
import { dirname, join } from "node:path";
import { WebSocketServer } from "ws";

const args = process.argv.slice(2);
const socketRoot = socket => {
  const root = dirname(dirname(dirname(socket)));
  assert.match(root, /^\/dev\/shm\/akra-fork-[a-f0-9-]{36}$/);
  assert.equal(socket, join(root, "akra/run/codex-app-server.sock"));
  return root;
};
if (args[0] === "--version") {
  console.log("codex-cli 0.171.0");
} else if (args.includes("--listen")) {
  const address = args[args.indexOf("--listen") + 1];
  assert(address.startsWith("unix://"));
  const root = socketRoot(address.slice("unix://".length));
  const server = createServer();
  startUpstream(root, new WebSocketServer({ server }));
  server.listen(join(root, "akra/run/codex-app-server.sock"));
} else {
  assert.deepEqual(args.slice(0, 3), ["app-server", "proxy", "--sock"]);
  socketRoot(args[3]);
  const socket = createConnection(args[3]);
  socket.once("error", error => { console.error(error); process.exitCode = 1; process.stdin.destroy(); });
  socket.once("close", () => process.stdin.destroy());
  process.stdin.pipe(socket).pipe(process.stdout);
}

function startUpstream(root, webSockets) {
  const { template, source, deleted, wrong } = JSON.parse(readFileSync(join(root, "threads.json"), "utf8"));
  const turn = { id: "fixture-finished", items: [], status: "completed", error: null, itemsView: "full" };
  const marker = name => writeFileSync(join(root, name), "", { flag: "wx" });
  let forks = 0;
  function until(name) {
    return new Promise((resolve, reject) => {
      const path = join(root, name);
      const close = () => { clearTimeout(deadline); watcher.close(); };
      const ready = () => { if (existsSync(path)) { close(); resolve(); } };
      const watcher = watch(root, ready);
      const deadline = setTimeout(() => { close(); reject(new Error(`No fixture control ${name}`)); }, 15_000);
      ready();
    });
  }
  async function dispatch({ id, method, params = {} }, socket) {
    const output = frame => socket.send(JSON.stringify({ jsonrpc: "2.0", ...frame }));
    const notify = (method, params) => output({ method, params });
    appendFileSync(join(root, "requests.jsonl"), JSON.stringify({ method, params }) + "\n");
    if (id === undefined) return;
    const result = result => output({ id, result });
    const error = (code, message) => output({ id, error: { code, message } });
    switch (method) {
      case "initialize":
        return result({ userAgent: "codex-cli/0.171.0 fixture", codexHome: root, platformFamily: "unix", platformOs: "linux" });
      case "thread/list":
        return result({ data: [source], nextCursor: null });
      case "thread/resume":
        if (params.threadId === source.id) return error(-32603, `thread ${source.id} already has an active writer`);
        return error(-32602, "Not a fixture source");
      case "thread/read":
        if (params.threadId === source.id) return result({ thread: source });
        return error(-32602, "Not a fixture source");
      case "thread/turns/list":
        return result({ data: [turn], nextCursor: null, backwardsCursor: null });
      case "thread/fork":
        assert.equal(params.threadId, source.id);
        assert.equal(params.lastTurnId, turn.id);
        assert.equal(params.ephemeral, false);
        if (++forks === 1) {
          notify("thread/started", { thread: deleted });
          await until("release-delete");
          notify("thread/deleted", { threadId: deleted.id });
          await until("release-reply");
          return result({ ...template, thread: deleted, cwd: deleted.cwd });
        }
        assert.equal(forks, 2);
        return result({ ...template, thread: wrong, cwd: wrong.cwd });
      case "model/list":
        return result({ data: [], nextCursor: null });
      case "account/read":
        return result({ account: null, requiresOpenaiAuth: false });
      case "thread/unsubscribe":
        return result({ status: "unsubscribed" });
      default:
        if (method.startsWith("turn/")) marker("unexpected-turn");
        return error(-32601, `No fixture response for ${method}`);
    }
  }
  webSockets.on("connection", socket => socket.on("message", bytes => {
    void dispatch(JSON.parse(bytes), socket).catch(error => {
      console.error(error);
      process.exitCode = 1;
      socket.close();
    });
  }));
}
