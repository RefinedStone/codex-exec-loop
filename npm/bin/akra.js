#!/usr/bin/env node

import { spawn } from "node:child_process";
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
  try {
    child.kill("SIGKILL");
  } catch {
    // The bounded wrapper exit below handles a missing child exit event.
  }
  forcedExitTimer = setTimeout(() => {
    process.exit(exitCodeForSignal("SIGKILL"));
  }, 1000);
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
