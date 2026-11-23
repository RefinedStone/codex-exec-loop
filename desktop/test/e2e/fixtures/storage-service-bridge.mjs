import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

const BRIDGE = "composition::storage_extension::tests::desktop_bridge::storage_desktop_fixture_bridge";
const CLI = "adapter::outbound::extension_gateway_client::tests::storage_desktop_fixture::storage_desktop_fixture_cli";
const PREFIX = "AKRA_STORAGE_FIXTURE ";
const CLI_PREFIX = "AKRA_STORAGE_CLI_RESULT ";
const options = name => ["--ignored", "--exact", name, "--nocapture", "--test-threads=1"];

function executable() {
  const value = process.env.AKRA_STORAGE_FIXTURE_BIN;
  if (!value) throw new Error("AKRA_STORAGE_FIXTURE_BIN must point to the compiled Rust library test executable; no native process is built automatically");
  return value;
}
export async function startStorageServiceBridge() {
  const child = spawn(executable(), options(BRIDGE), {
    env: { ...process.env, AKRA_STORAGE_FIXTURE_SERVER: "1" }, stdio: ["pipe", "pipe", "pipe"],
  });
  let stderr = "", nextId = 0, stopped = false, readyResolve, readyReject;
  const pending = new Map(), trace = [];
  const ready = new Promise((resolve, reject) => { readyResolve = resolve; readyReject = reject; });
  const ended = new Promise(resolve => child.once("close", (code, signal) => resolve({ code, signal })));
  const fail = error => { readyReject(error); for (const item of pending.values()) item.reject(error); pending.clear(); };
  child.stderr.on("data", data => { stderr = (stderr + data).slice(-16000); });
  child.once("error", fail);
  child.once("exit", (code, signal) => {
    if (!stopped) fail(new Error("storage fixture exited: " + code + "/" + signal + "\n" + stderr));
  });
  createInterface({ input: child.stdout }).on("line", line => {
    const start = line.indexOf(PREFIX);
    if (start < 0) return;
    try {
      const value = JSON.parse(line.slice(start + PREFIX.length));
      if (value.ready) readyResolve(value);
      else {
        const item = pending.get(value.id);
        if (item) { pending.delete(value.id); item.resolve(value); }
      }
    } catch (error) { fail(error); }
  });
  const timeout = setTimeout(() => fail(new Error("storage fixture did not become ready\n" + stderr)), 30_000);
  let info;
  try { info = await ready; } catch (error) { stopped = true; child.kill(); throw error; } finally { clearTimeout(timeout); }
  const rpc = async (method, params, caller = "desktop") => {
    if (stopped || child.exitCode !== null) throw new Error("storage fixture is stopped");
    const id = ++nextId;
    const reply = await new Promise((resolve, reject) => {
      const deadline = setTimeout(() => { pending.delete(id); reject(new Error("storage fixture request timed out: " + method)); }, 20_000);
      pending.set(id, { resolve: value => { clearTimeout(deadline); resolve(value); }, reject: error => { clearTimeout(deadline); reject(error); } });
      child.stdin.write(JSON.stringify({ id, method, params, caller }) + "\n");
    });
    trace.push({ method, params: structuredClone(params), reply: structuredClone(reply) });
    return reply;
  };
  return {
    info, trace, rpc,
    forward(method, params) {
      const reply = { result: null };
      reply.replyAfter = rpc(method, params, params.action?.startsWith("shared_") ? "cli" : "desktop").then(value => {
        if (value.error) { delete reply.result; reply.error = value.error; } else reply.result = value.result;
      }).catch(error => { delete reply.result; reply.error = { code: -32000, message: String(error) }; });
      return reply;
    },
    async close() {
      if (stopped) return;
      stopped = true;
      if (child.exitCode === null) child.stdin.end(JSON.stringify({ method: "fixture/shutdown" }) + "\n");
      const deadline = setTimeout(() => child.kill(), 10_000);
      const exit = await ended; clearTimeout(deadline);
      if (exit.code !== 0) throw new Error("storage fixture cleanup failed: " + JSON.stringify(exit) + "\n" + stderr);
    },
  };
}
export async function storageCliQuery(endpoint, threadId, action, input) {
  const child = spawn(executable(), options(CLI), {
    env: { ...process.env, AKRA_STORAGE_FIXTURE_ENDPOINT: endpoint,
      AKRA_STORAGE_FIXTURE_CALL: JSON.stringify({ thread_id: threadId, action, input }) },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stdout = "", stderr = "";
  child.stdout.on("data", value => { stdout += value; });
  child.stderr.on("data", value => { stderr = (stderr + value).slice(-16000); });
  const timer = setTimeout(() => child.kill(), 30_000);
  const code = await new Promise((resolve, reject) => { child.once("close", resolve); child.once("error", reject); }).finally(() => clearTimeout(timer));
  if (code !== 0) throw new Error("real CLI adapter failed: " + code + "\n" + stdout + stderr);
  const line = stdout.split(/\r?\n/).find(value => value.includes(CLI_PREFIX));
  if (!line) throw new Error("CLI adapter returned no JSON result\n" + stdout);
  return JSON.parse(line.slice(line.indexOf(CLI_PREFIX) + CLI_PREFIX.length));
}
