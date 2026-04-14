#!/usr/bin/env node

import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { detectPackageManager, resolveBinaryPath } from "../lib/runtime.js";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const require = createRequire(import.meta.url);

const packageRoot = path.join(__dirname, "..");
const { binaryPath } = resolveBinaryPath({
  packageRoot,
  resolvePackageJson: (specifier) => require.resolve(specifier),
});

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

child.on("error", (error) => {
  console.error(error);
  process.exit(1);
});

const forwardSignal = (signal) => {
  if (child.killed) {
    return;
  }

  try {
    child.kill(signal);
  } catch {
    // Ignore signal forwarding races during shutdown.
  }
};

for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => forwardSignal(signal));
}

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }

  process.exit(code ?? 1);
});
