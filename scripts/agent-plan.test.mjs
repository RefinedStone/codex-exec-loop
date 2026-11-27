import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { assertCurrentPlan, commandsForPlan, executeMeasured, parseOptions, runValidationPlan } from "./agent-plan.mjs";
import { createValidationPlan } from "./ci-impact-plan.mjs";
import { capturePlanInputs, collectPlanInput, hostConfiguration } from "./ci-plan-inputs.mjs";
import {
  createPlanFixture, fixtureGit, fixtureMetadata, fixtureToolchain, writeFixtureFile,
} from "./fixtures/ci-plan-fixture.mjs";

const scriptRoot = dirname(fileURLToPath(import.meta.url));
const agentScript = join(scriptRoot, "agent-plan.mjs");
function request(fixture, options = {}) {
  const state = capturePlanInputs({ cwd: fixture.cwd, environment: fixture.environment, toolchain: fixtureToolchain });
  const snapshot = { sha: state.sourceSha, metadata: fixtureMetadata(fixture.cwd), ownership: structuredClone(fixture.ownership) };
  return {
    schemaVersion: 1, base: structuredClone(snapshot), head: snapshot,
    changes: [{ status: "M", path: "crates/leaf/src/lib.rs" }],
    localChangeId: state.localChangeId, configuration: hostConfiguration(), ...options,
  };
}
function contract(overrides = {}) {
  return {
    id: "leaf-contract", owner: "leaf", target: { name: "leaf", kind: "lib" },
    filter: "tests::works", platforms: ["linux", "macos", "windows"],
    features: "default-and-all", isolation: "process-per-filter", ...overrides,
  };
}
function executionServices(fixture, input, overrides = {}) {
  return {
    cwd: fixture.cwd, environment: fixture.environment,
    collect: () => structuredClone(input),
    capture: () => capturePlanInputs({ cwd: fixture.cwd, environment: fixture.environment, toolchain: fixtureToolchain }),
    execute: async () => 0, ...overrides,
  };
}

test("selective argv runs complete member targets once per feature mode with registered contract filters", () => {
  const fixture = createPlanFixture({ contracts: [contract()] });
  try {
    const plan = createValidationPlan(request(fixture));
    const ciPlan = createValidationPlan(request(fixture, { configuration: {
      ...hostConfiguration(), platforms: ["linux", "macos", "windows"],
    } }));
    assert.deepEqual(plan.compilePackages, ciPlan.compilePackages);
    assert.deepEqual(plan.testPackages, ciPlan.testPackages);
    const commands = commandsForPlan(plan);
    const tests = commands.filter((step) => step.label.startsWith("tests-"));
    assert.equal(plan.mode, "selective");
    assert.deepEqual(tests.map((step) => step.features), ["all", "default"]);
    for (const step of tests) {
      assert.equal(step.command, process.execPath);
      assert.deepEqual(step.args.slice(0, 5), [
        "scripts/run_native_rust_contracts.mjs", "--include-members", "--package", "leaf", "--profile",
      ]);
      assert.deepEqual(step.args.slice(step.args.indexOf("--") + 1), ["tests::works"]);
      assert.equal(step.args.includes("--all-features"), step.features === "all");
    }
    assert.equal(commands.some((step) => step.command === "cargo" && step.args[0] === "test"), false);
    assert.deepEqual(commands.filter((step) => step.label.startsWith("clippy-")).map((step) => step.package), ["leaf"]);
    assert.equal(commands.some((step) => step.args.includes("other") || step.args.includes("akra-ade")), false);
    assert.ok(commands.some((step) => step.args[0] === "scripts/check_crate_boundaries.mjs"));
    assert.ok(commands.some((step) => step.command === "cargo" && step.args[0] === "fmt"));
  } finally { fixture.cleanup(); }
});

test("Windows and POSIX select the same member commands and preserve explicit target/profile", () => {
  const fixture = createPlanFixture();
  try {
    for (const [platform, triple] of [
      ["linux", "x86_64-unknown-linux-gnu"], ["win32", "x86_64-pc-windows-msvc"], ["darwin", "aarch64-apple-darwin"],
    ]) {
      const configuration = { ...hostConfiguration(platform), profile: "dev", features: ["all"], targetTriples: [triple] };
      const commands = commandsForPlan(createValidationPlan(request(fixture, { configuration })), platform, {
        architecture: triple.startsWith("aarch64") ? "arm64" : "x64",
      });
      const step = commands.find((command) => command.label === "tests-leaf-all");
      assert.deepEqual(step.args.slice(step.args.indexOf("--target"), step.args.indexOf("--target") + 2), ["--target", triple]);
      assert.deepEqual(step.args.slice(step.args.indexOf("--profile"), step.args.indexOf("--profile") + 2), ["--profile", "dev"]);
      assert.equal(commands.filter((command) => command.label.startsWith("tests-")).length, 1);
    }
  } finally { fixture.cleanup(); }
});

test("full fallback delegates to the existing broad gate and docs/Node have their own scope", () => {
  const fixture = createPlanFixture();
  try {
    for (const platform of ["linux", "win32"]) {
      const input = request(fixture, { configuration: hostConfiguration(platform), changes: [{ status: "M", path: "Cargo.lock" }] });
      const commands = commandsForPlan(createValidationPlan(input), platform);
      assert.deepEqual(commands.map((step) => step.label), ["diff-check", "full-native-gate"]);
      assert.deepEqual(commands[1].args, platform === "win32"
        ? ["-NoProfile", "-File", "scripts/check_native_pr.ps1", "-Mode", "Full"]
        : ["scripts/check_native_pr.sh"]);
    }
    const docs = createValidationPlan(request(fixture, { changes: [{ status: "M", path: "docs/reference/example.md" }] }));
    assert.deepEqual(commandsForPlan(docs).map((step) => step.label), ["diff-check"]);
    assert.equal(commandsForPlan(docs, process.platform, { forceFull: true })[1].label, "full-native-gate");
    const node = createValidationPlan(request(fixture, { changes: [{ status: "M", path: "assets/admin/app.js" }] }));
    assert.deepEqual(commandsForPlan(node).map((step) => step.label), ["diff-check", "node-surfaces"]);
  } finally { fixture.cleanup(); }
});

test("unsupported OS, targets, feature coverage and isolation reject execution rather than drop checks", () => {
  const fixture = createPlanFixture({ contracts: [contract({ features: "all" })] });
  try {
    const input = request(fixture);
    assert.throws(() => commandsForPlan(createValidationPlan(input)), /mixed feature modes/);
    input.configuration.features = ["all"];
    assert.ok(commandsForPlan(createValidationPlan(input)).some((step) => step.label === "tests-leaf-all"));
    input.head.ownership.contracts[0].isolation = "shared-process";
    assert.throws(() => commandsForPlan(createValidationPlan(input)), /unsupported contract/);
    input.head.ownership.contracts[0].isolation = "process-per-filter";
    input.configuration.platforms = ["linux", "windows"];
    assert.throws(() => commandsForPlan(createValidationPlan(input)), /current OS only/);
    input.configuration = { ...hostConfiguration(), targetTriples: ["unsupported-target"] };
    assert.throws(() => commandsForPlan(createValidationPlan(input)), /cannot run native tests/);
    input.configuration = { ...hostConfiguration("linux"), targetTriples: ["aarch64-unknown-linux-gnu"] };
    assert.throws(() => commandsForPlan(createValidationPlan(input), "linux", { architecture: "x64" }), /architecture differs/);
    input.configuration = { ...hostConfiguration(), profile: "dev" };
    input.changes = [{ status: "M", path: "Cargo.toml" }];
    assert.throws(() => commandsForPlan(createValidationPlan(input)), /standard test profile/);
  } finally { fixture.cleanup(); }
});

test("a modified or stale saved plan cannot run even its first command", async () => {
  const fixture = createPlanFixture();
  try {
    const input = request(fixture), plan = createValidationPlan(input);
    const altered = structuredClone(plan);
    altered.sourceSha = "b".repeat(40);
    assert.throws(() => assertCurrentPlan(altered, plan), /integrity/);
    for (const mutate of [
      (next) => { next.localChangeId = "different-input-bytes"; },
      (next) => { next.head.sha = "b".repeat(40); },
      (next) => { next.configuration.features = ["all"]; },
      (next) => { next.head.ownership.revision++; },
      (next) => { next.head.metadata.packages[1].targets[0].doctest = false; },
    ]) {
      const next = structuredClone(input);
      mutate(next);
      let executed = 0;
      await assert.rejects(runValidationPlan(plan, executionServices(fixture, next, {
        execute: async () => { executed++; return 0; },
      })), /stale validation plan/);
      assert.equal(executed, 0);
    }
  } finally { fixture.cleanup(); }
});

test("real source edits after planning are rejected by immediate recollection", async () => {
  const fixture = createPlanFixture();
  try {
    writeFixtureFile(fixture.cwd, "crates/leaf/src/lib.rs", "// planned edit\n");
    const options = { cwd: fixture.cwd, baseRef: "prerelease", environment: fixture.environment };
    const plan = createValidationPlan(collectPlanInput(options, fixture.services));
    writeFixtureFile(fixture.cwd, "crates/leaf/src/lib.rs", "// later edit\n");
    let executed = 0;
    await assert.rejects(runValidationPlan(plan, {
      ...options, collect: (current) => collectPlanInput(current, fixture.services),
      execute: async () => { executed++; return 0; },
    }), /stale validation plan/);
    assert.equal(executed, 0);
  } finally { fixture.cleanup(); }
});

test("execution uses the repository root, keeps target/incremental settings and stops on failure", async () => {
  const fixture = createPlanFixture();
  try {
    fixture.environment.CARGO_TARGET_DIR = join(fixture.cwd, "target/user-owned");
    fixture.environment.CARGO_INCREMENTAL = "1";
    const input = request(fixture), plan = createValidationPlan(input), seen = [];
    await assert.rejects(runValidationPlan(plan, executionServices(fixture, input, {
      cwd: join(fixture.cwd, "crates/leaf"),
      execute: async (command, options) => {
        seen.push(command.label);
        assert.equal(resolve(options.cwd), resolve(fixture.cwd));
        assert.equal(options.environment.CARGO_TARGET_DIR, fixture.environment.CARGO_TARGET_DIR);
        assert.equal(options.environment.CARGO_INCREMENTAL, "1");
        return seen.length === 2 ? 7 : 0;
      },
    })), /failed \(7\): crate-boundaries/);
    assert.deepEqual(seen, ["diff-check", "crate-boundaries"]);
  } finally { fixture.cleanup(); }
});

test("source changes during successful commands prevent current success evidence", async () => {
  const fixture = createPlanFixture();
  try {
    const input = request(fixture), plan = createValidationPlan(input);
    await assert.rejects(runValidationPlan(plan, executionServices(fixture, input, {
      execute: async () => { writeFixtureFile(fixture.cwd, "crates/leaf/src/lib.rs", "// changed during checks\n"); return 0; },
    })), /inputs changed during execution/);
  } finally { fixture.cleanup(); }
});

test("real Node subprocess execution reuses the existing performance JSONL format", async () => {
  const fixture = createPlanFixture();
  try {
    copyFileSync(join(scriptRoot, "ci-performance.mjs"), join(fixture.cwd, "scripts/ci-performance.mjs"));
    fixtureGit(fixture.cwd, ["add", "scripts/ci-performance.mjs"]);
    fixtureGit(fixture.cwd, ["commit", "-qm", "fixture meter"]);
    writeFixtureFile(fixture.cwd, "docs/example.md", "A fixture document.\n");
    const input = request(fixture, { changes: [{ status: "A", path: "docs/example.md" }] });
    const plan = createValidationPlan(input);
    const services = executionServices(fixture, input);
    delete services.execute;
    const result = await runValidationPlan(plan, services);
    assert.equal(result.mode, "selective");
    const records = readFileSync(join(fixture.cwd, "artifacts/ci-performance/steps.jsonl"), "utf8").trim().split("\n").map(JSON.parse);
    assert.equal(records.length, 1);
    assert.equal(records[0].schema, 1);
    assert.equal(records[0].exitCode, 0);
    assert.equal(records[0].command, "git");
    assert.ok(records[0].seconds >= 0);
    assert.ok(records[0].label.endsWith("-diff-check"));
  } finally { fixture.cleanup(); }
});

test("the shared process supervisor preserves cancellation and failure instead of reporting success", async () => {
  const command = { label: "fixture", command: process.execPath, args: ["--version"], features: "all", profile: "test" };
  for (const [result, expected] of [
    [{ status: 0, aborted: false, signal: null }, 0],
    [{ status: 7, aborted: false, signal: null }, 7],
    [{ status: 0, aborted: true, signal: "SIGINT" }, 130],
    [{ status: 0, aborted: true, signal: "SIGTERM" }, 143],
    [{ status: null, aborted: false, signal: "SIGKILL" }, 1],
  ]) {
    assert.equal(await executeMeasured(command, {
      cwd: scriptRoot, planHash: "a".repeat(64),
      environment: { CARGO_TARGET_DIR: "preserved target", CARGO_INCREMENTAL: "1" },
      stream: async (executable, args, options) => {
        assert.equal(executable, process.execPath);
        assert.equal(args[1], "run");
        assert.deepEqual(args.slice(-2), [process.execPath, "--version"]);
        assert.equal(options.env.CARGO_TARGET_DIR, "preserved target");
        assert.equal(options.env.CARGO_INCREMENTAL, "1");
        return result;
      },
    }), expected);
  }
  await assert.rejects(executeMeasured(command, {
    cwd: scriptRoot, planHash: "a".repeat(64),
    stream: async () => { throw new Error("Process tree cleanup failed: fixture"); },
  }), /cleanup failed/);
});

test("saved JSON is read-only, accepts UTF-8 BOM and round-trips without Cargo", () => {
  const fixture = createPlanFixture();
  try {
    const plan = createValidationPlan(request(fixture));
    const path = join(fixture.cwd, "artifacts/saved plan.json");
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, "\uFEFF" + JSON.stringify(plan));
    const output = execFileSync(process.execPath, [agentScript, "--plan", path, "--json"], { encoding: "utf8", cwd: fixture.cwd });
    assert.deepEqual(JSON.parse(output), plan);
    const explain = execFileSync(process.execPath, [agentScript, "--plan", path, "--explain"], { encoding: "utf8", cwd: fixture.cwd });
    assert.match(explain, /Required local argv/);
    assert.match(explain, /--include-members/);
    assert.equal(fixtureGit(fixture.cwd, ["status", "--porcelain"]), "");
  } finally { fixture.cleanup(); }
});

test("CLI actions and saved configurations cannot be silently mixed", () => {
  assert.equal(parseOptions([]).action, "explain");
  assert.equal(parseOptions(["--json", "--features", "default"]).configuration.features[0], "default");
  assert.equal(parseOptions(["--run", "--online"]).offline, false);
  assert.equal(parseOptions(["--run", "--full"]).forceFull, true);
  for (const args of [
    ["--run", "--json"], ["--run", "--run"], ["--features", "unknown"],
    ["--base"], ["--unknown"], ["--full"], ["--plan", "saved.json", "--base", "elsewhere"],
    ["--plan", "saved.json", "--features", "all"], ["--plan", "saved.json", "--profile", "dev"],
    ["--plan", "saved.json", "--target", "x86_64-pc-windows-msvc"],
  ]) assert.throws(() => parseOptions(args), undefined, JSON.stringify(args));
});

test("POSIX changed wrappers forward literal argv and environment to the planner", (context) => {
  if (spawnSync("bash", ["--version"], { stdio: "ignore" }).error) { context.skip("bash unavailable"); return; }
  const fixture = createPlanFixture();
  try {
    writeFixtureFile(fixture.cwd, "scripts/agent-plan.mjs",
      'console.log(JSON.stringify({args:process.argv.slice(2),target:process.env.CARGO_TARGET_DIR,incremental:process.env.CARGO_INCREMENTAL}));\n');
    const env = { ...fixture.environment, CARGO_TARGET_DIR: "target/preserved directory", CARGO_INCREMENTAL: "1" };
    for (const file of ["check_native_pr.sh", "check_windows_portable.sh"]) {
      copyFileSync(join(scriptRoot, file), join(fixture.cwd, "scripts", file));
      const args = ["--base", "branch $(literal) value"];
      const output = execFileSync("bash", [join(fixture.cwd, "scripts", file), "--changed", ...args], { encoding: "utf8", env });
      const recorded = JSON.parse(output.trim());
      assert.deepEqual(recorded.args, ["--run", ...(file === "check_windows_portable.sh" ? ["--target", "x86_64-pc-windows-msvc"] : []), ...args]);
      assert.equal(recorded.target, env.CARGO_TARGET_DIR);
      assert.equal(recorded.incremental, "1");
      const invalid = spawnSync("bash", [join(fixture.cwd, "scripts", file), "--full", "--base", "value"], { encoding: "utf8", env });
      assert.equal(invalid.status, 2);
    }
  } finally { fixture.cleanup(); }
});
