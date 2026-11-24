import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { defaultE2eSpecs, MANUAL_E2E_SPECS, parseE2eShard, partitionE2eSpecs } from "./e2e-specs.mjs";
import { e2ePlan, runE2e } from "./run-e2e.mjs";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
function fixture(t, files) {
  const root = mkdtempSync(join(tmpdir(), "akra-e2e-runner-test-"));
  mkdirSync(join(root, "test", "e2e"), { recursive: true });
  for (const [name, source] of Object.entries(files)) writeFileSync(join(root, "test", "e2e", name), source);
  t.after(() => rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 }));
  return root;
}

test("the two shards cover the discovered default suite exactly once, preserving manual exclusions", () => {
  const files = readdirSync(join(desktopRoot, "test", "e2e"));
  const specs = defaultE2eSpecs(files);
  const shards = [1, 2].map((index) => e2ePlan(desktopRoot, {}, ["--shard", `${index}/2`]).selectedSpecs);
  assert.deepEqual(shards.flat().sort(), specs);
  assert.equal(new Set(shards.flat()).size, specs.length);
  assert.deepEqual(files.filter((name) => name.endsWith(".mjs") && !specs.includes(name)).sort(), [...MANUAL_E2E_SPECS.keys()].sort());
  assert.deepEqual(partitionE2eSpecs([...specs].reverse(), 2), shards);
});

test("timing weights balance deterministically and invalid or empty shards fail closed", (t) => {
  const shards = partitionE2eSpecs(["a.mjs", "b.mjs", "c.mjs", "d.mjs"], 2,
    { "a.mjs": 8, "b.mjs": 7, "c.mjs": 2, "d.mjs": 1 });
  assert.deepEqual(shards, [["a.mjs", "d.mjs"], ["b.mjs", "c.mjs"]]);
  for (const value of ["0/2", "3/2", "1/0", "1", "a/b", "1/9007199254740992"]) assert.throws(() => parseE2eShard(value));
  assert.throws(() => partitionE2eSpecs(["a.mjs"], 2));
  assert.throws(() => partitionE2eSpecs(["a.mjs", "a.mjs"], 1));
  assert.throws(() => partitionE2eSpecs(["a.mjs"], 1, { "a.mjs": -1 }));
  const empty = fixture(t, { "ignored.txt": "" });
  assert.throws(() => e2ePlan(empty, {}), /no specs/);
  assert.throws(() => e2ePlan(desktopRoot, {}, ["--shard"]), /missing value/);
  assert.throws(() => e2ePlan(desktopRoot, {}, ["--unknown"]), /unknown argument/);
});

const passingSource = `
  import { existsSync, writeFileSync } from "node:fs";
  import { join } from "node:path";
  import { tmpdir } from "node:os";
  const data = { temp: tmpdir(), userData: process.env.AKRA_DESKTOP_USER_DATA,
    state: process.env.AKRA_HOME, artifacts: process.env.AKRA_E2E_ARTIFACT_DIR };
  if (!Object.values(data).every(existsSync)) throw new Error("isolation directory missing");
  writeFileSync(join(data.artifacts, "isolation.json"), JSON.stringify(data));
  console.log("fixture output retained");
`;
test("concurrent runs isolate profile, DB state, temp and artifacts and retain timed logs", async (t) => {
  const root = fixture(t, { "a.mjs": passingSource, "b.mjs": passingSource });
  const results = await Promise.all([1, 2].map(() => runE2e({ desktopRoot: root, env: process.env, argv: ["--shard", "1/1"], nodeArgs: [] })));
  const paths = [];
  for (const result of results) {
    assert.equal(result.exitCode, 0);
    const report = JSON.parse(readFileSync(result.reportPath, "utf8"));
    assert.equal(report.status, "passed");
    assert.equal(report.results.length, 2);
    assert.ok(report.durationMs >= 0);
    for (const spec of report.results) {
      assert.equal(spec.status, "passed");
      assert.ok(spec.durationMs >= 0);
      assert.match(readFileSync(join(report.outputDir, spec.log), "utf8"), /fixture output retained/);
      const isolated = JSON.parse(readFileSync(join(report.outputDir, spec.spec.slice(0, -4), "isolation.json"), "utf8"));
      paths.push(isolated.temp, isolated.userData, isolated.state, isolated.artifacts);
      assert.equal(existsSync(isolated.temp), false);
      assert.equal(existsSync(isolated.userData), false);
      assert.equal(existsSync(isolated.state), false);
      assert.equal(existsSync(isolated.artifacts), true);
    }
  }
  assert.equal(new Set(paths).size, paths.length);
});

test("a failed child retains its log and leaves later specs explicitly not_run", async (t) => {
  const root = fixture(t, { "a.mjs": 'console.error("fixture failure"); process.exit(7);', "b.mjs": passingSource });
  const result = await runE2e({ desktopRoot: root, env: process.env, nodeArgs: [] });
  assert.equal(result.exitCode, 7);
  assert.equal(result.report.status, "failed");
  assert.deepEqual(result.report.results.map((item) => item.status), ["failed", "not_run"]);
  assert.match(readFileSync(join(result.report.outputDir, result.report.results[0].log), "utf8"), /fixture failure/);
});

test("cancellation terminates the owned child tree and records unrun specs", async (t) => {
  const root = fixture(t, {
    "a.mjs": `
      import { spawn } from "node:child_process";
      import { writeFileSync } from "node:fs";
      const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "inherit" });
      process.on("SIGTERM", () => {});
      child.once("exit", () => process.exit(0));
      writeFileSync(process.env.E2E_FIXTURE_PID, String(child.pid));
      setInterval(() => {}, 1000);
    `,
    "b.mjs": passingSource,
  });
  const pidPath = join(root, "child.pid");
  const controller = new AbortController();
  const running = runE2e({ desktopRoot: root, env: { ...process.env, E2E_FIXTURE_PID: pidPath }, nodeArgs: [], signal: controller.signal });
  const deadline = Date.now() + 5000;
  while (!existsSync(pidPath) && Date.now() < deadline) await new Promise((resolveWait) => setTimeout(resolveWait, 20));
  controller.abort();
  const result = await running;
  assert.ok(existsSync(pidPath), "fixture descendant started");
  const pid = Number(readFileSync(pidPath, "utf8"));
  assert.equal(result.exitCode, 130);
  assert.equal(result.report.status, "cancelled");
  assert.deepEqual(result.report.results.map((item) => item.status), ["cancelled", "not_run"]);
  assert.throws(() => process.kill(pid, 0), { code: "ESRCH" });
});


test("detached Electron-style descendants stop without touching an unrelated process", { skip: process.platform === "win32" }, async (t) => {
  const childSource = 'const fs = require("node:fs"); process.on("SIGTERM", () => {}); fs.writeFileSync(process.env.E2E_DETACHED_READY, String(process.pid)); setInterval(() => {}, 1000);';
  const specSource = [
    'import { spawn } from "node:child_process";',
    'import { writeFileSync } from "node:fs";',
    'process.on("SIGTERM", () => {});',
    'const child = spawn(process.execPath, ["-e", ' + JSON.stringify(childSource) + '], { detached: true, stdio: "ignore" });',
    'child.once("exit", () => process.exit(0));',
    'writeFileSync(process.env.E2E_FIXTURE_PID, String(child.pid));',
    'setInterval(() => {}, 1000);',
  ].join("\n");
  const root = fixture(t, { "a.mjs": specSource, "b.mjs": passingSource });
  const pidPath = join(root, "detached.pid");
  const readyPath = join(root, "detached.ready");
  const unrelated = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { detached: true, stdio: "ignore" });
  t.after(async () => {
    if (unrelated.exitCode === null && unrelated.signalCode === null) {
      const exited = new Promise(resolveExit => unrelated.once("exit", resolveExit));
      unrelated.kill("SIGKILL");
      await exited;
    }
    if (existsSync(pidPath)) {
      const pid = Number(readFileSync(pidPath, "utf8"));
      try { process.kill(pid, "SIGKILL"); } catch (error) { if (error.code !== "ESRCH") throw error; }
    }
  });
  const controller = new AbortController();
  const running = runE2e({ desktopRoot: root, env: { ...process.env,
    E2E_FIXTURE_PID: pidPath, E2E_DETACHED_READY: readyPath }, nodeArgs: [], signal: controller.signal });
  const deadline = Date.now() + 5000;
  while (!existsSync(readyPath) && Date.now() < deadline) await new Promise(resolveWait => setTimeout(resolveWait, 20));
  controller.abort();
  const result = await running;
  assert.ok(existsSync(readyPath), "detached descendant installed its SIGTERM handler");
  const pid = Number(readFileSync(pidPath, "utf8"));
  assert.equal(result.exitCode, 130);
  assert.equal(result.report.results[0].cleanupError, undefined);
  assert.deepEqual(result.report.results.map(item => item.status), ["cancelled", "not_run"]);
  assert.throws(() => process.kill(pid, 0), { code: "ESRCH" });
  assert.doesNotThrow(() => process.kill(unrelated.pid, 0), "unrelated detached process remains alive");
});
