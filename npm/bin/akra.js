#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { constants as osConstants } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { detectPackageManager, resolveBinaryPath } from "../lib/runtime.js";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const require = createRequire(import.meta.url);

const packageRoot = path.join(__dirname, "..");
let binaryPath;
try {
  ({ binaryPath } = resolveBinaryPath({
    packageRoot,
    resolvePackageJson: (specifier) => require.resolve(specifier),
  }));
} catch (error) {
  reportError(error);
  process.exit(1);
}

const env = { ...process.env };
const packageManagerEnvVar =
  detectPackageManager() === "bun"
    ? "AKRA_MANAGED_BY_BUN"
    : "AKRA_MANAGED_BY_NPM";
env[packageManagerEnvVar] = "1";

const child = spawn(binaryPath, process.argv.slice(2), {
  stdio: "inherit",
  env,
});

const shutdownGraceMs = parseShutdownGraceMs(
  process.env.AKRA_NPM_SHUTDOWN_GRACE_MS,
);
let childExited = false;
let shutdownSignal = null;
let shutdownTimer = null;
let forcedExitTimer = null;
let shutdownDescendants = emptyDescendantSnapshot();

const maxProcessTableBytes = 4 * 1024 * 1024;
const maxTrackedDescendants = 4096;
const maxLinuxProcessEntries = 32768;

child.on("error", (error) => {
  childExited = true;
  clearShutdownTimers();
  reportError(error);
  process.exit(1);
});

const forwardSignal = (signal) => {
  if (childExited) {
    return;
  }

  if (shutdownSignal !== null) {
    forceTerminateChild();
    return;
  }

  shutdownSignal = signal;

  try {
    if (process.platform !== "win32" && Number.isInteger(child.pid)) {
      const descendants = collectUnixDescendants(child.pid);
      rememberShutdownDescendants(descendants);
      signalUnixDescendants(descendants, signal);
    }
    if (!child.kill(signal)) {
      forceTerminateChild();
      return;
    }
  } catch {
    forceTerminateChild();
    return;
  }

  shutdownTimer = setTimeout(forceTerminateChild, shutdownGraceMs);
};

for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => forwardSignal(signal));
}

child.on("exit", (code, signal) => {
  if (shutdownSignal !== null && process.platform !== "win32") {
    signalUnixDescendants(
      revalidateDescendantSnapshot(shutdownDescendants),
      "SIGKILL",
    );
  }
  childExited = true;
  clearShutdownTimers();
  if (signal) {
    process.exit(exitCodeForSignal(signal));
    return;
  }

  process.exit(code ?? 1);
});

function forceTerminateChild() {
  if (childExited || forcedExitTimer !== null) {
    return;
  }

  if (shutdownTimer !== null) {
    clearTimeout(shutdownTimer);
    shutdownTimer = null;
  }
  if (process.platform !== "win32") {
    forceTerminateUnixDescendants();
  }
  killChild("SIGKILL");
  forcedExitTimer = setTimeout(() => {
    process.exit(exitCodeForSignal("SIGKILL"));
  }, 1000);
}

function forceTerminateUnixDescendants() {
  if (!Number.isInteger(child.pid)) {
    return;
  }

  // Freeze the native leader before taking the ancestry snapshot so it cannot
  // start another app-server between discovery and forced termination.
  killChild("SIGSTOP");
  const first = collectUnixDescendants(child.pid);
  rememberShutdownDescendants(first);
  signalUnixDescendants(first, "SIGSTOP");
  const second = collectUnixDescendants(child.pid);
  rememberShutdownDescendants(second);
  const descendants = revalidateDescendantSnapshot(
    mergeDescendantSnapshots(shutdownDescendants, second),
  );
  signalUnixDescendants(descendants, "SIGKILL");
}

function emptyDescendantSnapshot() {
  return { processes: [], unsafeProcessGroups: new Set() };
}

function rememberShutdownDescendants(snapshot) {
  shutdownDescendants = mergeDescendantSnapshots(
    shutdownDescendants,
    snapshot,
  );
}

function collectUnixDescendants(rootPid) {
  const processes = readUnixProcessTable();
  if (processes === null) {
    return { processes: [], unsafeProcessGroups: new Set() };
  }
  const byParent = new Map();
  const byPid = new Map();
  for (const processEntry of processes) {
    byPid.set(processEntry.pid, processEntry);
    const children = byParent.get(processEntry.ppid) ?? [];
    children.push(processEntry);
    byParent.set(processEntry.ppid, children);
  }

  const rootProcess = byPid.get(rootPid);
  if (rootProcess === undefined || rootProcess.ppid !== process.pid) {
    return { processes: [], unsafeProcessGroups: new Set() };
  }
  const unsafeProcessGroups = new Set(
    [byPid.get(process.pid)?.pgid, rootProcess.pgid].filter(
      (pgid) => Number.isInteger(pgid) && pgid > 0,
    ),
  );
  const descendants = [];
  const pending = [...(byParent.get(rootPid) ?? [])].map((entry) => ({
    ...entry,
    depth: 1,
  }));
  const visited = new Set();
  while (pending.length > 0 && descendants.length < maxTrackedDescendants) {
    const entry = pending.shift();
    if (visited.has(entry.pid)) {
      continue;
    }
    visited.add(entry.pid);
    descendants.push(entry);
    for (const childEntry of byParent.get(entry.pid) ?? []) {
      pending.push({ ...childEntry, depth: entry.depth + 1 });
    }
  }
  return { processes: descendants, unsafeProcessGroups };
}

function readUnixProcessTable() {
  if (process.platform === "linux") {
    return readLinuxProcessTable();
  }
  for (const executable of ["/bin/ps", "/usr/bin/ps"]) {
    const result = spawnSync(
      executable,
      [
        "-A",
        "-o",
        "pid=",
        "-o",
        "ppid=",
        "-o",
        "pgid=",
        "-o",
        "lstart=",
      ],
      {
        encoding: "utf8",
        env: { LANG: "C", LC_ALL: "C", PATH: "/usr/bin:/bin" },
        maxBuffer: maxProcessTableBytes,
        timeout: 500,
        windowsHide: true,
      },
    );
    if (result.error || result.status !== 0 || typeof result.stdout !== "string") {
      continue;
    }
    const processes = [];
    let invalid = false;
    for (const line of result.stdout.split("\n")) {
      if (line.trim() === "") {
        continue;
      }
      const fields = line.trim().split(/\s+/);
      if (
        fields.length < 8 ||
        fields.slice(0, 3).some((field) => !/^\d+$/.test(field))
      ) {
        invalid = true;
        break;
      }
      const [pid, ppid, pgid] = fields.slice(0, 3).map(Number);
      if (
        ![pid, ppid, pgid].every(Number.isSafeInteger) ||
        pid <= 0 ||
        ppid < 0 ||
        pgid <= 0
      ) {
        continue;
      }
      processes.push({
        pid,
        ppid,
        pgid,
        identity: `ps:${fields.slice(3).join(" ")}`,
      });
    }
    if (!invalid) {
      return processes;
    }
  }
  return null;
}

function readLinuxProcessTable() {
  const startedAt = Date.now();
  let entries;
  try {
    entries = readdirSync("/proc").filter((entry) => /^\d+$/.test(entry));
  } catch {
    return null;
  }
  if (entries.length > maxLinuxProcessEntries) {
    return null;
  }

  const processes = [];
  for (const entry of entries) {
    if (Date.now() - startedAt > 500) {
      return null;
    }
    let stat;
    try {
      stat = readFileSync(`/proc/${entry}/stat`, "utf8");
    } catch {
      continue;
    }
    const openParen = stat.indexOf("(");
    const closeParen = stat.lastIndexOf(")");
    if (openParen <= 0 || closeParen <= openParen) {
      continue;
    }
    const pidText = stat.slice(0, openParen).trim();
    const fields = stat.slice(closeParen + 1).trim().split(/\s+/);
    if (
      pidText !== entry ||
      fields.length < 20 ||
      !/^\d+$/.test(pidText) ||
      !/^\d+$/.test(fields[1]) ||
      !/^\d+$/.test(fields[2]) ||
      !/^\d+$/.test(fields[19])
    ) {
      continue;
    }
    const pid = Number(pidText);
    const ppid = Number(fields[1]);
    const pgid = Number(fields[2]);
    if (
      ![pid, ppid, pgid].every(Number.isSafeInteger) ||
      pid <= 0 ||
      ppid < 0 ||
      pgid <= 0
    ) {
      continue;
    }
    processes.push({
      pid,
      ppid,
      pgid,
      identity: `linux:${pid}:${fields[19]}`,
    });
  }
  return processes;
}

function mergeDescendantSnapshots(first, second) {
  const merged = new Map();
  for (const entry of first.processes) {
    const previous = merged.get(entry.pid);
    if (previous === undefined || entry.depth > previous.depth) {
      merged.set(entry.pid, entry);
    }
  }
  for (const entry of second.processes) {
    merged.set(entry.pid, entry);
  }
  return {
    processes: [...merged.values()],
    unsafeProcessGroups: new Set([
      ...first.unsafeProcessGroups,
      ...second.unsafeProcessGroups,
    ]),
  };
}

function revalidateDescendantSnapshot(snapshot) {
  const current = readUnixProcessTable();
  if (current === null) {
    return emptyDescendantSnapshot();
  }
  const currentByPid = new Map(current.map((entry) => [entry.pid, entry]));
  return {
    processes: snapshot.processes.filter((entry) => {
      const live = currentByPid.get(entry.pid);
      return (
        live !== undefined &&
        live.pgid === entry.pgid &&
        live.identity === entry.identity
      );
    }),
    unsafeProcessGroups: snapshot.unsafeProcessGroups,
  };
}

function signalUnixDescendants(snapshot, signal) {
  const processGroups = new Map();
  const descendantPids = new Set(snapshot.processes.map((entry) => entry.pid));
  for (const entry of snapshot.processes) {
    if (
      entry.pgid > 1 &&
      descendantPids.has(entry.pgid) &&
      !snapshot.unsafeProcessGroups.has(entry.pgid)
    ) {
      processGroups.set(
        entry.pgid,
        Math.max(processGroups.get(entry.pgid) ?? 0, entry.depth),
      );
    }
  }
  const signaledProcessGroups = new Set();
  for (const [processGroup] of [...processGroups.entries()].sort(
    (left, right) => right[1] - left[1],
  )) {
    if (killPid(-processGroup, signal)) {
      signaledProcessGroups.add(processGroup);
    }
  }
  for (const entry of [...snapshot.processes].sort((left, right) => right.depth - left.depth)) {
    if (!signaledProcessGroups.has(entry.pgid)) {
      killPid(entry.pid, signal);
    }
  }
}

function killChild(signal) {
  try {
    child.kill(signal);
  } catch {
    // The bounded wrapper exit handles an already-terminated child.
  }
}

function killPid(pid, signal) {
  try {
    process.kill(pid, signal);
    return true;
  } catch {
    // ESRCH is expected when a process or group exits during the sweep.
    return false;
  }
}

function clearShutdownTimers() {
  if (shutdownTimer !== null) {
    clearTimeout(shutdownTimer);
    shutdownTimer = null;
  }
  if (forcedExitTimer !== null) {
    clearTimeout(forcedExitTimer);
    forcedExitTimer = null;
  }
}

function parseShutdownGraceMs(rawValue) {
  if (rawValue === undefined) {
    return 5000;
  }
  const value = Number(rawValue);
  return Number.isInteger(value) && value >= 100 && value <= 60000
    ? value
    : 5000;
}

function exitCodeForSignal(signal) {
  const signalNumber = osConstants.signals?.[signal];
  return typeof signalNumber === "number" ? 128 + signalNumber : 1;
}

function reportError(error) {
  const message = error instanceof Error ? error.message : String(error);
  console.error(`akra: ${message}`);
  if (process.env.AKRA_NPM_DEBUG === "1" && error instanceof Error && error.stack) {
    console.error(error.stack);
  }
}
