import test from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { isolatedEnvironment, smokeLauncher } from "../scripts/smoke-native-release.mjs";

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
      mkdirSync(options.env.AKRA_HOME, { recursive: true });
      const stdout = args.includes("--help") ? "Usage: akra"
        : args.includes("path") ? options.env.AKRA_HOME : "parallel.evidence_mode = observe (global)";
      return { status: 0, stdout };
    };
    smokeLauncher(bundle, root, "linux", fake);
    assert.equal(calls.length, 4);
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
