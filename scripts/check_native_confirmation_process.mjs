#!/usr/bin/env node

import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, realpathSync, rmSync, chmodSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const test = "adapter::inbound::gateway::tests::human_confirmation::native_process::native_helper_process_confirms_replays_and_revokes_on_eof";

if (process.platform === "win32") {
  throw new Error("The native confirmation process check requires a Unix host.");
}

// Cargo reports the actual executable even with CARGO_TARGET_DIR or a target override.
const build = execFileSync("cargo", ["build", "--locked", "--bin", "akra-gateway", "--message-format=json"], {
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
  const result = spawnSync("cargo", ["test", "--locked", "--lib", test, "--", "--ignored", "--exact"], {
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
