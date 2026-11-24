#!/usr/bin/env node

import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, realpathSync, rmSync, chmodSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const test = "adapter::inbound::gateway::tests::human_confirmation::native_process::native_helper_process_confirms_replays_and_revokes_on_eof";

export function confirmationCargoCommands(argv) {
  const booleanOptions = new Set(["--all-features", "--no-default-features", "--timings"]);
  const valueOptions = new Set(["--features", "--target", "--profile", "--target-dir"]);
  const seen = new Set();
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index];
    if (seen.has(option)) throw new Error(`Duplicate Cargo option: ${option}`);
    seen.add(option);
    if (booleanOptions.has(option)) continue;
    if (!valueOptions.has(option)) throw new Error(`Unsupported Cargo option: ${option}`);
    const value = argv[++index];
    if (!value || value.startsWith("-")) throw new Error(`${option} requires a value.`);
  }
  if (seen.has("--all-features") && seen.has("--features")) {
    throw new Error("Use either --all-features or --features.");
  }
  // Keep Linux default-feature validation while allowing macOS to reuse its all-feature artifacts.
  // Both commands inherit CARGO_TARGET_DIR/CARGO_BUILD_TARGET and use exactly the same overrides.
  return {
    build: ["build", "--locked", "--bin", "akra-gateway", "--message-format=json", ...argv],
    test: ["test", "--locked", "--lib", ...argv, test, "--", "--ignored", "--exact"],
  };
}

function main() {
  if (process.platform === "win32") {
    throw new Error("The native confirmation process check requires a Unix host.");
  }
  const commands = confirmationCargoCommands(process.argv.slice(2));

  // Cargo reports the actual executable even with CARGO_TARGET_DIR or a target override.
  const build = execFileSync("cargo", commands.build, {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
    maxBuffer: 64 * 1024 * 1024,
  });
  const executable = build.split(/\r?\n/).filter(Boolean).map((line) => JSON.parse(line))
    .findLast((message) => message.reason === "compiler-artifact"
      && message.target?.name === "akra-gateway" && message.executable)?.executable;
  if (!executable) {
    throw new Error("Cargo did not report the built akra-gateway executable.");
  }

  // macOS Unix sockets have a short path limit; keep this outside long checkout/temp paths.
  const home = mkdtempSync(join(realpathSync("/tmp"), "akra-hc-"));
  try {
    chmodSync(home, 0o700);
    const result = spawnSync("cargo", commands.test, {
      cwd: root,
      env: { ...process.env, AKRA_HOME: home, AKRA_CONFIRMATION_HELPER_TEST_BIN: executable },
      encoding: "utf8",
      stdio: ["ignore", "pipe", "inherit"],
      maxBuffer: 8 * 1024 * 1024,
    });
    process.stdout.write(result.stdout ?? "");
    if (result.error) throw result.error;
    if (result.status !== 0) {
      throw new Error(`Native confirmation process check failed (${result.signal ?? result.status}).`);
    }
    if (!/^test result: ok\. 1 passed; 0 failed;/m.test(result.stdout)) {
      throw new Error("The native confirmation process check must run exactly one passing test.");
    }
  } finally {
    rmSync(home, { recursive: true, force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main();
}
