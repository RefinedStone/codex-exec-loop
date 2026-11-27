import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { isolatedEnvironment, smokeLauncher, smokeUpgrade } from "../scripts/smoke-native-release.mjs";

test("release smoke clears old install paths, auth, and profile overrides", () => {
  const env = isolatedEnvironment("/fixture", "linux", { HOME: "/old", AKRA_HOME: "/old/.akra",
    GH_TOKEN: "secret", OPENAI_API_KEY: "secret", PATH: "/old/bin" });
  assert.equal(env.HOME, path.join("/fixture", "profile"));
  assert.equal(env.AKRA_HOME, path.join(env.HOME, ".akra"));
  assert.equal(env.PATH, "/usr/bin:/bin");
  assert.equal(env.GH_TOKEN, undefined);
  assert.equal(env.OPENAI_API_KEY, undefined);
});

test("release launcher smoke verifies each subprocess, including persisted configuration", () => {
  const root = mkdtempSync(path.join(tmpdir(), "akra-smoke-unit-"));
  try {
    const bundle = path.join(root, "installed fixture");
    mkdirSync(bundle);
    const calls = [];
    const fake = (command, args, options) => {
      calls.push({ command, args, options });
      return fixtureProcess(command, args, options);
    };
    smokeLauncher(bundle, root, "linux", fake);
    assert.equal(calls.length, 5);
    assert.ok(calls.every(c => c.command === path.join(bundle, "akra")
      && c.options.cwd === path.join(root, "outside checkout")
      && c.options.env.AKRA_HOME.startsWith(root)));
    assert.throws(() => smokeLauncher(bundle, root, "linux", () => ({ status: 1, stdout: "" })),
      /release launcher failed/);
    assert.throws(() => smokeLauncher(bundle, root, "linux", () => ({ status: 0, stdout: "wrong" })),
      /unexpected release launcher output/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});


function fixtureProcess(_command, args, options) {
  const file = path.join(options.env.AKRA_HOME, "config.toml");
  let stdout;
  if (args.includes("--help")) stdout = "Usage: akra";
  else if (args.includes("path")) stdout = file;
  else if (args.includes("set")) {
    mkdirSync(options.env.AKRA_HOME, { recursive: true });
    writeFileSync(file, args.at(-1));
    stdout = "parallel.evidence_mode = " + args.at(-1);
  } else {
    const value = existsSync(file) ? readFileSync(file, "utf8") : "unset";
    stdout = "parallel.evidence_mode = " + value + " (global)";
  }
  return { status: 0, stdout };
}

test("upgrade reads the previous installation's non-default setting before candidate writes", () => {
  const root = mkdtempSync(path.join(tmpdir(), "akra-upgrade-unit-"));
  try {
    const old = path.join(root, "old"), candidate = path.join(root, "new");
    const calls = [];
    smokeUpgrade(old, candidate, root, "linux", (command, args, options) => {
      calls.push({ command, args, home: options.env.AKRA_HOME });
      return fixtureProcess(command, args, options);
    });
    assert.equal(new Set(calls.map(call => call.home)).size, 1);
    assert.deepEqual(calls.filter(call => call.command === path.join(candidate, "akra")).map(call => call.args[1]),
      [undefined, "path", "get", "set", "get"]);
    assert.throws(() => smokeUpgrade(old, old, root, "linux", fixtureProcess), /separate installed bundles/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("upgrade fails if the candidate loses the previous setting, before writing any replacement", () => {
  const root = mkdtempSync(path.join(tmpdir(), "akra-upgrade-loss-"));
  try {
    const old = path.join(root, "old"), candidate = path.join(root, "new");
    let candidateWrites = 0;
    assert.throws(() => smokeUpgrade(old, candidate, root, "linux", (command, args, options) => {
      if (command === path.join(candidate, "akra")) {
        if (args.includes("set")) candidateWrites++;
        if (args.includes("get")) return { status: 0, stdout: "parallel.evidence_mode = unset (global)" };
      }
      return fixtureProcess(command, args, options);
    }), /unexpected release launcher output/);
    assert.equal(candidateWrites, 0);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
