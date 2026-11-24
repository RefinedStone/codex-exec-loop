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

function stopChildTree(child, signal) {
  if (!child.pid) return;
  if (process.platform === "win32") {
    // 실행 중인 이 시험의 자식 트리만 정리한다.
    if (child.exitCode === null && child.signalCode === null) {
      const result = spawnSync("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], { windowsHide: true, encoding: "utf8", timeout: 5000 });
      if (result.error) throw result.error;
      if (result.status !== 0 && child.exitCode === null) child.kill();
    }
  } else {
    try { process.kill(-child.pid, signal); }
    catch (error) { if (error.code !== "ESRCH") throw error; }
  }
}

async function executeSpec({ desktopRoot, spec, env, signal, logPath, nodeArgs }) {
  const started = performance.now();
  const log = openSync(logPath, "w");
  let spawnError;
  let cleanupError;
  let killTimer;
  const child = spawn(process.execPath, [...nodeArgs, join(desktopRoot, "test", "e2e", spec)], {
    cwd: desktopRoot, env, stdio: ["ignore", "pipe", "pipe"], detached: process.platform !== "win32", windowsHide: true,
  });
  const stop = (kind) => {
    try { stopChildTree(child, kind); }
    catch (error) { cleanupError = String(error); }
  };
  const scheduleKill = () => {
    if (!killTimer) killTimer = setTimeout(() => stop("SIGKILL"), 2000);
  };
  const abort = () => { stop("SIGTERM"); scheduleKill(); };
  signal?.addEventListener("abort", abort, { once: true });
  if (signal?.aborted) abort();
  const tee = (stream) => (chunk) => { writeSync(log, chunk); stream.write(chunk); };
  child.stdout.on("data", tee(process.stdout));
  child.stderr.on("data", tee(process.stderr));
  child.once("error", (error) => { spawnError = String(error); });
  child.once("exit", () => {
    // 정상 종료 뒤 남은 Electron 하위 프로세스도 이 시험의 process group에서만 정리한다.
    if (process.platform !== "win32") { stop("SIGTERM"); scheduleKill(); }
  });
  const outcome = await new Promise((resolveResult) => child.once("close", (code, childSignal) => resolveResult({ code, signal: childSignal })));
  if (process.platform !== "win32") stop("SIGKILL");
  clearTimeout(killTimer);
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
