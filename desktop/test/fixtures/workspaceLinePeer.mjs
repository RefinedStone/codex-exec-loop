import { spawn } from "node:child_process";
import { EventEmitter, once } from "node:events";
import { createInterface } from "node:readline";

export function workspaceLinePeer(command, args, { cwd, env, prefix = "" }) {
  const child = spawn(command, args, { cwd, env, windowsHide: true, stdio: ["pipe", "pipe", "pipe"] });
  const events = new EventEmitter();
  let sequence = 0;
  let failure;
  let stderr = "";
  child.stderr.on("data", bytes => { stderr = (stderr + bytes).slice(-4096); });
  const fail = error => { failure = error; events.emit("failure", error); };
  child.on("error", fail);
  child.stdin.on("error", fail);
  const ended = new Promise(resolve => child.once("close", (code, signal) => {
    fail(new Error(`Fixture exited ${code}/${signal}\n${stderr}`));
    resolve(code);
  }));
  createInterface({ input: child.stdout }).on("line", line => {
    const at = line.indexOf(prefix);
    if (at < 0) return;
    try {
      const frame = JSON.parse(line.slice(at + prefix.length));
      events.emit(`reply:${frame.id}`, frame);
      if (frame.method && frame.id === undefined) events.emit("notification", frame);
    } catch (error) {
      fail(error);
    }
  });
  return {
    async request(method, params) {
      if (failure) throw failure;
      const id = ++sequence;
      const event = `reply:${id}`;
      const abort = new AbortController();
      const onFailure = error => abort.abort(error);
      events.on("failure", onFailure);
      const deadline = setTimeout(() => abort.abort(new Error(`Fixture timed out: ${method}\n${stderr}`)), 30_000);
      try {
        const reply = once(events, event, { signal: abort.signal });
        child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
        const [frame] = await reply;
        return frame;
      } catch (error) {
        throw abort.signal.reason ?? error;
      } finally {
        clearTimeout(deadline);
        events.off("failure", onFailure);
      }
    },
    notify(method, params) {
      child.stdin.write(JSON.stringify({ jsonrpc: "2.0", method, params }) + "\n");
    },
    onNotification(listener) { events.on("notification", listener); },
    async close() {
      child.stdin.end();
      const deadline = setTimeout(() => {
        if (process.platform === "win32" && child.pid !== undefined) {
          spawn("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
            env: process.env, windowsHide: true, stdio: "ignore", timeout: 5_000,
          });
        } else child.kill();
      }, 5_000);
      const code = await ended;
      clearTimeout(deadline);
      if (code !== 0) throw failure;
    },
  };
}
