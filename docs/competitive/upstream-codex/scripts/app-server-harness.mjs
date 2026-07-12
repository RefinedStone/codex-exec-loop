import { spawn } from "node:child_process";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { createInterface } from "node:readline";
import { tmpdir } from "node:os";
import { join } from "node:path";

const DEFAULT_TIMEOUT_MS = 20_000;
const SAFE_ENV_NAMES = ["LANG", "LC_ALL", "PATH", "SSL_CERT_DIR", "SSL_CERT_FILE", "TZ"];

function isolatedEnvironment(codexHome) {
  const env = { CODEX_HOME: codexHome, HOME: codexHome };
  for (const name of SAFE_ENV_NAMES) {
    if (process.env[name] !== undefined) {
      env[name] = process.env[name];
    }
  }
  return env;
}

export function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

export function decodeCanonicalBase64(value, label) {
  if (typeof value !== "string") {
    throw new TypeError(`${label} must be a string`);
  }
  const decoded = Buffer.from(value, "base64");
  if (decoded.toString("base64") !== value) {
    throw new TypeError(`${label} must be canonical standard Base64`);
  }
  return decoded;
}

export async function createIsolatedCodexHome() {
  return mkdtemp(join(tmpdir(), "akra-codex-v01441-"));
}

export async function removeIsolatedCodexHome(path) {
  await rm(path, { recursive: true, force: true });
}

export class AppServerClient {
  static start(binary, codexHome, timeoutMs = DEFAULT_TIMEOUT_MS) {
    const child = spawn(binary, [], {
      env: isolatedEnvironment(codexHome),
      stdio: ["pipe", "pipe", "pipe"],
    });
    return new AppServerClient(child, timeoutMs);
  }

  constructor(child, timeoutMs) {
    this.child = child;
    this.timeoutMs = timeoutMs;
    this.nextId = 1;
    this.notifications = [];
    this.pending = new Map();
    this.stderr = "";
    this.spawnFailed = false;
    this.exit = new Promise((resolve) => {
      let settled = false;
      const settle = (result) => {
        if (!settled) {
          settled = true;
          resolve(result);
        }
      };
      child.once("exit", (code, signal) => settle({ code, signal }));
      child.once("error", (error) => {
        this.spawnFailed = true;
        settle({ code: null, signal: null, error });
      });
    });

    createInterface({ input: child.stdout }).on("line", (line) => this.#receive(line));
    child.stderr.on("data", (chunk) => {
      this.stderr = `${this.stderr}${chunk}`.slice(-16_384);
    });
    child.stdin.on("error", (error) => this.#rejectPending(error));
    child.once("error", (error) => this.#rejectPending(error));
    child.once("exit", (code, signal) => {
      this.#rejectPending(new Error(`app-server exited: code=${code} signal=${signal}`));
    });
  }

  get pid() {
    return this.child.pid;
  }

  async initialize(capabilities = {}) {
    return this.request("initialize", {
      clientInfo: {
        name: "akra_upstream_audit",
        title: "Akra upstream audit",
        version: "0.144.1",
      },
      capabilities,
    });
  }

  notify(method, params = {}) {
    this.#write({ method, params });
  }

  request(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`request timed out: ${method}`));
      }, this.timeoutMs);
      this.pending.set(id, { method, resolve, reject, timer });
      this.#write({ id, method, params });
    });
  }

  async close() {
    if (
      this.spawnFailed ||
      this.child.exitCode !== null ||
      this.child.signalCode !== null
    ) {
      return this.exit;
    }
    this.child.stdin.end();
    const graceful = await Promise.race([this.exit, delay(1_000).then(() => null)]);
    if (graceful) {
      return graceful;
    }
    this.child.kill("SIGTERM");
    const terminated = await Promise.race([this.exit, delay(1_000).then(() => null)]);
    if (terminated) {
      return terminated;
    }
    this.child.kill("SIGKILL");
    return this.exit;
  }

  #write(message) {
    this.child.stdin.write(`${JSON.stringify(message)}\n`);
  }

  #receive(line) {
    let message;
    try {
      message = JSON.parse(line);
    } catch (error) {
      this.#rejectPending(new Error(`invalid JSON from app-server: ${error.message}`));
      return;
    }
    if (message.id === undefined || message.id === null) {
      this.notifications.push(message);
      return;
    }
    const pending = this.pending.get(message.id);
    if (!pending) {
      return;
    }
    clearTimeout(pending.timer);
    this.pending.delete(message.id);
    pending.resolve(message);
  }

  #rejectPending(error) {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
  }
}

export async function sampleLinuxProcessTree(rootPid) {
  const pids = [];
  const pending = [rootPid];
  const visited = new Set();
  while (pending.length > 0) {
    const pid = pending.pop();
    if (!Number.isInteger(pid) || visited.has(pid)) {
      continue;
    }
    visited.add(pid);
    try {
      const taskIds = await readdir(`/proc/${pid}/task`);
      const children = new Set();
      for (const taskId of taskIds) {
        try {
          const body = await readFile(`/proc/${pid}/task/${taskId}/children`, "utf8");
          for (const child of body.trim().split(/\s+/).filter(Boolean).map(Number)) {
            children.add(child);
          }
        } catch (error) {
          if (error.code !== "ENOENT" && error.code !== "ESRCH") {
            throw error;
          }
        }
      }
      pending.push(...children);
      pids.push(pid);
    } catch (error) {
      if (error.code !== "ENOENT" && error.code !== "ESRCH") {
        throw error;
      }
    }
  }

  let rssKib = 0;
  let processCount = 0;
  for (const pid of pids) {
    try {
      const status = await readFile(`/proc/${pid}/status`, "utf8");
      const match = status.match(/^VmRSS:\s+(\d+)\s+kB$/m);
      if (match) {
        rssKib += Number(match[1]);
        processCount += 1;
      }
    } catch (error) {
      if (error.code !== "ENOENT" && error.code !== "ESRCH") {
        throw error;
      }
    }
  }
  return { rssKib, processCount };
}
