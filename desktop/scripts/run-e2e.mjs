// 각 shard는 독립 runner에서 실행하고, shard 안에서는 화면 시험을 순서대로 실행한다.
import { spawn, spawnSync } from "node:child_process";
import { closeSync, mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync, writeSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { defaultE2eSpecs, MANUAL_E2E_SPECS, parseE2eShard, partitionE2eSpecs } from "./e2e-specs.mjs";

const defaultDesktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export function e2ePlan(desktopRoot, env = process.env, argv = []) {
  let shardValue = env.AKRA_E2E_SHARD ?? "1/1";
  let timingsPath = env.AKRA_E2E_TIMINGS;
  let outputDir = env.AKRA_E2E_OUTPUT_DIR ?? join(desktopRoot, "test-results", "e2e");
  let list = false;
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--list") list = true;
    else if (["--shard", "--timings", "--output-dir"].includes(arg)) {
      const value = argv[++index];
      if (!value || value.startsWith("--")) throw new Error(`e2e: missing value for ${arg}`);
      if (arg === "--shard") shardValue = value;
      if (arg === "--timings") timingsPath = value;
      if (arg === "--output-dir") outputDir = value;
    } else throw new Error(`e2e: unknown argument ${arg}`);
  }
  const allSpecs = defaultE2eSpecs(readdirSync(join(desktopRoot, "test", "e2e")));
  if (!allSpecs.length) throw new Error("e2e: no specs found");
  const shard = parseE2eShard(shardValue);
  const timingData = timingsPath ? JSON.parse(readFileSync(resolve(desktopRoot, timingsPath), "utf8")) : {};
  const durationsMs = timingData.durationsMs ?? timingData;
  if (!durationsMs || typeof durationsMs !== "object" || Array.isArray(durationsMs)) {
    throw new Error("e2e: timings must be a duration map or an E2E report");
  }
  const shards = partitionE2eSpecs(allSpecs, shard.total, durationsMs);
  return { shard, allSpecs, selectedSpecs: shards[shard.index - 1], manualSpecs: Object.fromEntries(MANUAL_E2E_SPECS),
    timingSource: timingsPath ?? null, outputDir: resolve(desktopRoot, outputDir), list };
}

// Playwright starts Electron in a separate POSIX session. The spec's process group
// alone therefore does not contain every owned process. Keep birth identities while
// the spec is alive; never use a process-name match or blindly signal a saved PID.
function posixProcessSnapshot() {
  const result = new Map();
  if (process.platform === "linux") {
    for (const entry of readdirSync("/proc")) {
      if (!/^\d+$/.test(entry)) continue;
      try {
        const stat = readFileSync("/proc/" + entry + "/stat", "utf8");
        const fields = stat.slice(stat.lastIndexOf(")") + 2).trim().split(/\s+/);
        const pid = Number(entry);
        result.set(pid, { pid, ppid: Number(fields[1]), pgid: Number(fields[2]),
          birth: fields[19], state: fields[0] });
      } catch (error) {
        if (!["ENOENT", "ESRCH", "EACCES"].includes(error.code)) throw error;
      }
    }
  } else {
    const listed = spawnSync("/bin/ps", ["-A", "-o", "pid=,ppid=,pgid=,lstart="], {
      encoding: "utf8", timeout: 1000, maxBuffer: 4 * 1024 * 1024, env: { ...process.env, LC_ALL: "C" },
    });
    if (listed.error || listed.status !== 0) throw listed.error ?? new Error("could not inspect owned E2E processes");
    for (const line of listed.stdout.split("\n")) {
      const match = /^\s*(\d+)\s+(\d+)\s+(\d+)\s+(.+?)\s*$/.exec(line);
      if (match) {
        const pid = Number(match[1]);
        result.set(pid, { pid, ppid: Number(match[2]), pgid: Number(match[3]), birth: match[4] });
      }
    }
  }
  return result;
}

function trackPosixProcesses(child) {
  const owned = new Map();
  let failure;
  const same = (left, right) => left && right && left.pid === right.pid
    && left.birth === right.birth && left.pgid === right.pgid;
  function observe() {
    const snapshot = posixProcessSnapshot();
    const live = new Set();
    const root = snapshot.get(child.pid);
    if (!owned.has(child.pid) && child.exitCode === null && child.signalCode === null && root) {
      owned.set(child.pid, root);
    }
    for (const [pid, identity] of owned) {
      if (same(identity, snapshot.get(pid))) live.add(pid);
    }
    // Previously seen descendants may already have been reparented after the spec exits.
    // Linux birth ticks keep ownership without trusting a recycled PID.
    for (let changed = true; changed;) {
      changed = false;
      for (const [pid, identity] of snapshot) {
        if (live.has(identity.ppid) && !live.has(pid)) {
          owned.set(pid, identity); live.add(pid); changed = true;
        }
      }
    }
    return { snapshot, live };
  }
  function inspect() {
    try { return observe(); }
    catch (error) { failure = String(error); return { snapshot: new Map(), live: new Set() }; }
  }
  inspect();
  const monitor = setInterval(inspect, 1000);
  monitor.unref();
  function stop(signal, includeRoot = true) {
    const { snapshot, live } = inspect();
    for (const pid of [...live].filter(pid => includeRoot || pid !== child.pid).sort((a, b) => (a === child.pid) - (b === child.pid))) {
      if (pid === child.pid && (child.exitCode !== null || child.signalCode !== null)) continue;
      const identity = owned.get(pid);
      const current = snapshot.get(pid);
      if (!same(identity, current) || current.state === "Z") continue;
      if (process.platform !== "linux" && pid !== child.pid) {
        // ps start dates have second precision. Require a current chain to the
        // still-owned ChildProcess; an orphan's old PID is not enough on Darwin.
        let ancestor = current;
        const visited = new Set();
        while (ancestor && ancestor.pid !== child.pid && !visited.has(ancestor.pid)) {
          visited.add(ancestor.pid); ancestor = snapshot.get(ancestor.ppid);
        }
        if (!ancestor || child.exitCode !== null || child.signalCode !== null) {
          failure = "could not safely identify a reparented E2E process";
          continue;
        }
      }
      const fresh = posixProcessSnapshot().get(pid);
      if (!same(identity, fresh)) continue;
      try { process.kill(pid, signal); }
      catch (error) { if (error.code !== "ESRCH") failure = String(error); }
    }
  }
  return {
    stop,
    async finish() {
      clearInterval(monitor);
      const deadline = performance.now() + 2000;
      for (;;) {
        try { stop("SIGKILL", false); }
        catch (error) { failure = String(error); break; }
        const { snapshot, live } = inspect();
        const remaining = [...live].filter(pid => pid !== child.pid && snapshot.get(pid)?.state !== "Z");
        if (!remaining.length) break;
        if (performance.now() >= deadline) { failure ??= "owned E2E processes did not stop within 2 seconds"; break; }
        await new Promise(resolveWait => setTimeout(resolveWait, 50));
      }
      return failure;
    },
  };
}

function stopWindowsChildTree(child) {
  if (child.pid && child.exitCode === null && child.signalCode === null) {
    const result = spawnSync("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
      windowsHide: true, encoding: "utf8", timeout: 5000,
    });
    if (result.error) throw result.error;
    if (result.status !== 0 && child.exitCode === null) child.kill();
  }
}

async function executeSpec({ desktopRoot, spec, env, signal, logPath, nodeArgs }) {
  const started = performance.now();
  const log = openSync(logPath, "w");
  let spawnError;
  let cleanupError;
  let killTimer;
  let rootKillTimer;
  const child = spawn(process.execPath, [...nodeArgs, join(desktopRoot, "test", "e2e", spec)], {
    cwd: desktopRoot, env, stdio: ["ignore", "pipe", "pipe"], detached: process.platform !== "win32", windowsHide: true,
  });
  const tracker = process.platform === "win32" ? null : trackPosixProcesses(child);
  const stop = (kind, includeRoot = true) => {
    try { if (tracker) tracker.stop(kind, includeRoot); else stopWindowsChildTree(child); }
    catch (error) { cleanupError = String(error); }
  };
  const scheduleKill = () => {
    if (!killTimer) killTimer = setTimeout(() => {
      stop("SIGKILL", false);
      // Let the spec reap its terminated descendants before the final fallback.
      rootKillTimer = setTimeout(() => stop("SIGKILL"), 100);
    }, 2000);
  };
  const abort = () => { stop("SIGTERM"); scheduleKill(); };
  signal?.addEventListener("abort", abort, { once: true });
  if (signal?.aborted) abort();
  const tee = (stream) => (chunk) => { writeSync(log, chunk); stream.write(chunk); };
  child.stdout.on("data", tee(process.stdout));
  child.stderr.on("data", tee(process.stderr));
  child.once("error", (error) => { spawnError = String(error); });
  child.once("exit", () => {
    // 관찰한 소유 신원을 통해 별도 session의 Electron도 정리한다.
    if (process.platform !== "win32") { stop("SIGTERM"); scheduleKill(); }
  });
  const outcome = await new Promise((resolveResult) => child.once("close", (code, childSignal) => resolveResult({ code, signal: childSignal })));
  clearTimeout(killTimer);
  clearTimeout(rootKillTimer);
  if (tracker) {
    const trackedError = await tracker.finish();
    cleanupError ??= trackedError;
  }
  signal?.removeEventListener("abort", abort);
  closeSync(log);
  return { durationMs: Math.round(performance.now() - started), exitCode: outcome.code, signal: outcome.signal,
    status: signal?.aborted ? "cancelled" : outcome.code === 0 && !spawnError && !cleanupError ? "passed" : "failed",
    ...(spawnError ? { error: spawnError } : {}), ...(cleanupError ? { cleanupError } : {}) };
}

export async function runE2e({ desktopRoot = defaultDesktopRoot, env = process.env, argv = [], signal, nodeArgs = process.execArgv } = {}) {
  const plan = e2ePlan(desktopRoot, env, argv);
  if (plan.list) { console.log(JSON.stringify(plan, null, 2)); return { exitCode: 0, plan }; }
  mkdirSync(plan.outputDir, { recursive: true });
  const outputDir = mkdtempSync(join(plan.outputDir, `shard-${plan.shard.index}-of-${plan.shard.total}-`));
  // Darwin의 긴 기본 TMPDIR는 Unix 소켓 경로 한도를 넘기므로 짧은 OS 임시 경로를 쓴다.
  const temporaryRoot = mkdtempSync(join(process.platform === "darwin" ? "/tmp" : tmpdir(), "ake-"));
  const reportPath = join(outputDir, "report.json");
  const started = performance.now();
  const report = { schemaVersion: 1, ...plan, outputDir, startedAt: new Date().toISOString(), status: "running",
    results: plan.selectedSpecs.map((spec) => ({ spec, status: "not_run" })), durationsMs: {} };
  const save = () => {
    report.durationMs = Math.round(performance.now() - started);
    writeFileSync(reportPath + ".tmp", JSON.stringify(report, null, 2) + "\n");
    renameSync(reportPath + ".tmp", reportPath);
  };
  save();
  console.log(`e2e: shard ${plan.shard.index}/${plan.shard.total}, ${plan.selectedSpecs.length}/${plan.allSpecs.length} specs; report ${reportPath}`);
  try {
    for (const result of report.results) {
      if (signal?.aborted) break;
      const specRoot = mkdtempSync(join(temporaryRoot, "s-"));
      const artifacts = join(outputDir, result.spec.slice(0, -4));
      for (const directory of ["tmp", "user-data", "state", "config", "cache"]) mkdirSync(join(specRoot, directory));
      mkdirSync(artifacts);
      const childEnv = { ...env, TMPDIR: join(specRoot, "tmp"), TMP: join(specRoot, "tmp"), TEMP: join(specRoot, "tmp"),
        AKRA_DESKTOP_USER_DATA: join(specRoot, "user-data"), AKRA_HOME: join(specRoot, "state"),
        XDG_CONFIG_HOME: join(specRoot, "config"), XDG_CACHE_HOME: join(specRoot, "cache"),
        AKRA_E2E_ARTIFACT_DIR: artifacts };
      console.log(`e2e: ${result.spec}`);
      result.status = "running";
      result.log = join(result.spec.slice(0, -4), "output.log");
      save();
      Object.assign(result, await executeSpec({ desktopRoot, spec: result.spec, env: childEnv, signal,
        logPath: join(artifacts, "output.log"), nodeArgs }));
      report.durationsMs[result.spec] = Math.max(result.durationMs, 1);
      save();
      rmSync(specRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
      if (result.status !== "passed") break;
    }
  } catch (error) {
    report.error = String(error);
  } finally {
    try { rmSync(temporaryRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 }); }
    catch (error) { report.cleanupError = String(error); }
    report.status = signal?.aborted ? "cancelled"
      : !report.error && !report.cleanupError && report.results.every((result) => result.status === "passed") ? "passed" : "failed";
    report.completedAt = new Date().toISOString();
    save();
  }
  const exitCode = report.status === "passed" ? 0 : report.status === "cancelled" ? 130
    : report.results.find((result) => Number.isInteger(result.exitCode) && result.exitCode > 0)?.exitCode ?? 1;
  console.log(`e2e: ${report.status}; report ${reportPath}`);
  return { exitCode, report, reportPath };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const controller = new AbortController();
  let receivedSignal;
  const interrupt = (signal) => { receivedSignal = signal; controller.abort(); };
  const onTerm = () => interrupt("SIGTERM");
  const onInt = () => interrupt("SIGINT");
  process.on("SIGTERM", onTerm);
  process.on("SIGINT", onInt);
  try {
    const result = await runE2e({ argv: process.argv.slice(2), signal: controller.signal });
    process.exitCode = receivedSignal === "SIGTERM" ? 143 : result.exitCode;
  } catch (error) {
    console.error(String(error));
    process.exitCode = 1;
  } finally {
    process.removeListener("SIGTERM", onTerm);
    process.removeListener("SIGINT", onInt);
  }
}
