import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { createValidationPlan } from "./ci-impact-plan.mjs";
import { assertCleanSource, commandsForCIRow, createCIExecutionPlan, readCIResultArtifacts, runSelectedRow,
  validateCIResults, verifyCIExecutionPlan } from "./ci-workflow-plan.mjs";

function fixture({ contracts = [], mutate = () => {}, configuration } = {}) {
  const root = "/fixture/repository";
  const packages = ["akra-ade", "leaf"].map((name, index) => ({
    id: "opaque package [" + index + "]", name, version: "1.0.0", source: null, features: {}, dependencies: [],
    manifest_path: root + (index ? "/crates/leaf/Cargo.toml" : "/Cargo.toml"),
    targets: [{ name: name.replaceAll("-", "_"), kind: ["lib"], test: true, doctest: true, "required-features": [] },
      ...(index ? [{ name: "external", kind: ["test"], test: true, doctest: false, "required-features": [] }] : [])],
  }));
  const snapshot = { sha: "b".repeat(40), metadata: { version: 1, workspace_root: root, packages,
    workspace_members: packages.map((pkg) => pkg.id), resolve: { nodes: packages.map((pkg) => ({ id: pkg.id, features: [], deps: [] })) } },
    ownership: { schemaVersion: 1, revision: 1, rootPackage: "akra-ade", fullInputs: ["Cargo.toml", "Cargo.lock"], contracts,
      packages: packages.map((pkg, index) => ({ name: pkg.name, manifest: index ? "crates/leaf/Cargo.toml" : "Cargo.toml",
        state: "active", consumerCoverage: "full", sourcePrefixes: [index ? "crates/leaf/" : "src/"], externalInputs: [], workspaceDependencies: [] })) } };
  mutate(snapshot);
  return createValidationPlan({ schemaVersion: 1, head: snapshot, base: { ...structuredClone(snapshot), sha: "a".repeat(40) },
    changes: [{ status: "M", path: "crates/leaf/src/lib.rs" }], configuration });
}
const contract = (extra = {}) => ({ id: "native-check", owner: "leaf", target: { kind: "lib", name: "leaf" },
  filter: "native::tests::", platforms: ["linux", "windows", "macos"], features: "all", isolation: "process-per-filter", ...extra });
function proof(plan, matrix, row) {
  const { runner, contractFilters, ...conditions } = row;
  return { schemaVersion: 1, ...conditions, sourceSha: plan.sourceSha, planHash: plan.planHash,
    matrixHash: matrix.matrixHash, status: "success", exitCode: 0 };
}
function evidence(plan = fixture()) {
  const matrix = createCIExecutionPlan(plan);
  return { plan, matrix, results: matrix.rows.map((row) => proof(plan, matrix, row)) };
}

test("one common plan expands complete library, integration and doctest coverage on every host and feature", () => {
  const { plan, matrix, results } = evidence(fixture({ contracts: [contract()] }));
  assert.equal(matrix.mode, "selective");
  assert.equal(matrix.rows.length, 18);
  assert.equal(new Set(matrix.rows.map((row) => row.rowId)).size, 18);
  assert.deepEqual([...new Set(matrix.rows.map((row) => row.target.kind))].sort(), ["doc", "lib", "test"]);
  assert.deepEqual([...new Set(matrix.rows.map((row) => row.os))].sort(), ["linux", "macos", "windows"]);
  assert.equal(matrix.rows.filter((row) => row.contractFilters.length).length, 3);
  assert.deepEqual(validateCIResults(plan, matrix, results), { accepted: true, rows: 18, sourceSha: plan.sourceSha, planHash: plan.planHash });
  assert.deepEqual(createCIExecutionPlan(plan), matrix);
});

test("post-merge and explicit full execution cannot pass with an empty selected matrix", () => {
  const plan = fixture();
  const matrix = createCIExecutionPlan(plan, { forceFull: true });
  assert.equal(matrix.mode, "full");
  assert.deepEqual(matrix.rows, []);
  assert.throws(() => validateCIResults(plan, matrix, []), /empty or full/);
  const rootPlan = fixture({ mutate: (snapshot) => {
    snapshot.metadata.packages[0].dependencies.push({ name: "leaf", path: "/fixture/repository/crates/leaf", kind: null, optional: false, features: [] });
    snapshot.metadata.resolve.nodes[0].deps.push({ pkg: snapshot.metadata.packages[1].id, name: "leaf", dep_kinds: [{ kind: null, target: null }] });
    snapshot.ownership.packages[0].workspaceDependencies.push("leaf");
  } });
  assert.equal(rootPlan.mode, "full");
  assert.equal(createCIExecutionPlan(rootPlan).mode, "full");
});

test("unsupported targets and required features fail closed instead of losing rows", () => {
  for (const mutate of [
    (snapshot) => { snapshot.metadata.packages[1].targets[0]["required-features"] = ["native"]; },
    (snapshot) => { snapshot.metadata.packages[1].targets[1].kind = ["bench"]; },
  ]) {
    const matrix = createCIExecutionPlan(fixture({ mutate }));
    assert.equal(matrix.mode, "full");
    assert.deepEqual(matrix.rows, []);
  }
  assert.equal(createCIExecutionPlan(fixture({ configuration: { targetTriples: ["custom-target"] } })).mode, "full");
});

test("every required contract must have its exact supported feature, isolation and target row", () => {
  for (const candidate of [contract({ features: "some-feature" }), contract({ isolation: "shared-process" })]) {
    const matrix = createCIExecutionPlan(fixture({ contracts: [candidate] }));
    assert.equal(matrix.mode, "full");
    assert.ok(matrix.fallbackReasons.some((reason) => reason.startsWith("unsupported-contract-condition")));
  }
  const matrix = createCIExecutionPlan(fixture({ contracts: [contract()], configuration: { features: ["default"] } }));
  assert.equal(matrix.mode, "full");
  assert.ok(matrix.fallbackReasons.some((reason) => reason.startsWith("required-contract-not-covered")));
});

test("compile-only binaries and examples cannot disappear from a successful selected plan", () => {
  for (const kind of ["bin", "example"]) {
    const plan = fixture({ mutate: (snapshot) => { snapshot.metadata.packages[1].targets.push({
      name: "worker", kind: [kind], test: false, doctest: false, "required-features": [],
    }); } });
    assert.equal(plan.mode, "selective");
    const matrix = createCIExecutionPlan(plan);
    assert.equal(matrix.mode, "full");
    assert.ok(matrix.fallbackReasons.some((reason) => reason.startsWith("compile-target-not-covered:")));
  }
});

test("missing, duplicate and unexpected rows are rejected even when every provided result succeeded", () => {
  const { plan, matrix, results } = evidence();
  assert.throws(() => validateCIResults(plan, matrix, results.slice(1)), /Missing/);
  assert.throws(() => validateCIResults(plan, matrix, [...results, results[0]]), /duplicate/);
  assert.throws(() => validateCIResults(plan, matrix, [{ ...results[0], rowId: "0".repeat(24) }, ...results.slice(1)]), /Unexpected/);
  assert.throws(() => validateCIResults(plan, matrix, undefined), /missing/);
});

test("artifact directories preserve collisions and reject wrong filenames, extra files and other runs", () => {
  const directory = mkdtempSync(join(tmpdir(), "akra-ci-artifacts-"));
  const { plan, matrix, results } = evidence();
  try {
    for (const result of results) {
      const path = join(directory, "ci-plan-result-2-" + result.rowId);
      mkdirSync(path);
      writeFileSync(join(path, result.rowId + ".json"), JSON.stringify(result));
    }
    assert.equal(validateCIResults(plan, matrix, readCIResultArtifacts(directory, "2")).accepted, true);
    assert.throws(() => readCIResultArtifacts(directory, "1"), /Unexpected/);
    const first = join(directory, "ci-plan-result-2-" + results[0].rowId);
    writeFileSync(join(first, "extra.json"), JSON.stringify(results[0]));
    assert.throws(() => readCIResultArtifacts(directory, "2"), /exactly/);
    rmSync(join(first, "extra.json"));
    writeFileSync(join(first, results[0].rowId + ".json"), JSON.stringify(results[1]));
    assert.throws(() => readCIResultArtifacts(directory, "2"), /does not match/);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test("skipped, cancelled, failed, incomplete and nonzero rows never pass", () => {
  const { plan, matrix, results } = evidence();
  for (const status of ["skipped", "cancelled", "failed", "running", undefined]) {
    assert.throws(() => validateCIResults(plan, matrix, [{ ...results[0], status }, ...results.slice(1)]), /did not succeed/);
  }
  assert.throws(() => validateCIResults(plan, matrix, [{ ...results[0], exitCode: 1 }, ...results.slice(1)]), /did not succeed/);
});

test("stale SHA and every package/target/OS/feature/profile/triple coordinate must match", () => {
  const { plan, matrix, results } = evidence();
  for (const [key, value] of Object.entries({ sourceSha: "c".repeat(40), planHash: "0".repeat(64), matrixHash: "f".repeat(64),
    packageKey: "other", package: "other", manifest: "other/Cargo.toml", target: { kind: "test", name: "other" },
    os: "other", features: "other", profile: "release", targetTriple: "other", architecture: "other", schemaVersion: 2 })) {
    assert.throws(() => validateCIResults(plan, matrix, [{ ...results[0], [key]: value }, ...results.slice(1)]), /mismatched/, key);
  }
});

test("altered plans and omitted or modified matrix coordinates cannot change what the gate expects", () => {
  const { plan, matrix } = evidence();
  assert.throws(() => verifyCIExecutionPlan({ ...plan, sourceSha: "c".repeat(40) }, matrix), /hash/);
  for (const altered of [
    { ...matrix, rows: matrix.rows.slice(1) },
    { ...matrix, forceFull: true },
    { ...matrix, rows: [{ ...matrix.rows[0], contractFilters: ["invented"] }, ...matrix.rows.slice(1)] },
    { ...matrix, rows: [{ ...matrix.rows[0], runner: "other" }, ...matrix.rows.slice(1)] },
  ]) assert.throws(() => verifyCIExecutionPlan(plan, altered), /immutable/);
});

test("commands retain exact target, feature, profile and triple; isolated filters use the shared runner", () => {
  const matrix = createCIExecutionPlan(fixture({ contracts: [contract()], configuration: { profile: "custom" } }));
  const row = matrix.rows.find((candidate) => candidate.os === "windows" && candidate.features === "all" && candidate.target.kind === "lib");
  assert.deepEqual(commandsForCIRow(row)[0].args, ["scripts/run_native_rust_contracts.mjs", "--all-tests", "--package", "leaf",
    "--target-kind", "lib", "--target-name", "leaf", "--all-features", "--profile", "custom", "--target", "x86_64-pc-windows-msvc", "--timings", "--", "native::tests::"]);
  const doc = matrix.rows.find((candidate) => candidate.features === "default" && candidate.target.kind === "doc");
  assert.deepEqual(commandsForCIRow(doc)[0], { command: "cargo", args: ["test", "--locked", "--package", "leaf", "--doc", "--profile", "custom", "--target", doc.targetTriple, "--timings"] });
});

test("a row proves its execution conditions and checks the source before and after the command", async () => {
  const { plan, matrix } = evidence();
  const row = matrix.rows.find((candidate) => candidate.os === "linux" && candidate.target.kind === "lib");
  let sourceChecks = 0, invoked = 0;
  const result = await runSelectedRow(plan, matrix, row.rowId, { platform: "linux", architecture: "x64", cwd: "/fixture",
    checkSource: (sha, cwd) => { assert.equal(sha, plan.sourceSha); assert.equal(cwd, "/fixture"); sourceChecks += 1; },
    contracts: async (args, options) => { invoked += 1; assert.equal(options.env.AKRA_CI_PLAN_HASH, plan.planHash);
      assert.deepEqual(args, commandsForCIRow(row)[0].args.slice(1)); return { executedTests: 2 }; } });
  assert.equal(sourceChecks, 2);
  assert.equal(invoked, 1);
  assert.equal(result.status, "success");
  assert.deepEqual(result.tests, { executedTests: 2 });
  assert.equal(result.sourceSha, plan.sourceSha);
  assert.deepEqual(result.target, row.target);
});

test("runtime errors, cancellation, source mutation and incorrect hosts are rejected", async () => {
  const { plan, matrix } = evidence();
  const row = matrix.rows.find((candidate) => candidate.os === "linux" && candidate.target.kind === "lib");
  const options = { platform: "linux", architecture: "x64", checkSource: () => {}, contracts: async () => ({}) };
  for (const cancelled of [false, true]) {
    const result = await runSelectedRow(plan, matrix, row.rowId, { ...options, contracts: async () => { throw Object.assign(new Error("failed"), { cancelled }); } });
    assert.equal(result.status, cancelled ? "cancelled" : "failed");
    assert.equal(result.exitCode, 1);
  }
  let count = 0;
  const stale = await runSelectedRow(plan, matrix, row.rowId, { ...options, checkSource: () => { if (count++) throw new Error("source changed"); } });
  assert.equal(stale.status, "failed");
  await assert.rejects(runSelectedRow(plan, matrix, row.rowId, { ...options, platform: "darwin" }), /host/);
  await assert.rejects(runSelectedRow(plan, matrix, row.rowId, { ...options, architecture: "arm64" }), /host/);
  await assert.rejects(runSelectedRow(plan, matrix, row.rowId, { ...options, env: { AKRA_CI_PLAN_HASH: "wrong" } }), /Scope output hash/);
});

test("doctest exit and signal errors propagate to the result proof", async () => {
  const { plan, matrix } = evidence();
  const row = matrix.rows.find((candidate) => candidate.os === "linux" && candidate.target.kind === "doc");
  for (const execution of [{ status: 1 }, { status: 0, signal: "SIGTERM" }, { error: new Error("spawn") }]) {
    const result = await runSelectedRow(plan, matrix, row.rowId, { platform: "linux", architecture: "x64", checkSource: () => {}, spawn: () => execution });
    assert.equal(result.status, "failed");
    assert.equal(result.exitCode, 1);
  }
});

test("real Git source checks reject stale HEAD, modified tracked content and untracked input", () => {
  const cwd = mkdtempSync(join(tmpdir(), "akra-ci-proof-"));
  const git = (...args) => execFileSync("git", args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  try {
    git("init");
    mkdirSync(join(cwd, "scripts"));
    writeFileSync(join(cwd, "scripts/ci-crate-ownership.json"), JSON.stringify({ packages: [] }));
    writeFileSync(join(cwd, "source.txt"), "original\n");
    git("add", "source.txt", "scripts/ci-crate-ownership.json");
    git("-c", "user.name=CI fixture", "-c", "user.email=ci-fixture@example.invalid", "commit", "-m", "fixture");
    const sha = git("rev-parse", "HEAD");
    assert.doesNotThrow(() => assertCleanSource(sha, cwd));
    assert.throws(() => assertCleanSource("0".repeat(40), cwd), /changed/);
    writeFileSync(join(cwd, "source.txt"), "changed\n");
    assert.throws(() => assertCleanSource(sha, cwd), /clean/);
    writeFileSync(join(cwd, "source.txt"), "original\n");
    writeFileSync(join(cwd, "extra.txt"), "new\n");
    assert.throws(() => assertCleanSource(sha, cwd), /clean/);
  } finally { rmSync(cwd, { recursive: true, force: true }); }
});

test("workflow preserves stable gates and exact immutable plan/result artifact wiring", () => {
  const workflow = readFileSync(new URL("../.github/workflows/native-pr-checks.yml", import.meta.url), "utf8");
  assert.match(workflow, /name: Fast Gate/);
  assert.match(workflow, /name: CI Gate/);
  assert.match(workflow, /name: Post-Merge Gate/);
  assert.match(workflow, /node scripts\/ci-workflow-plan\.mjs create/);
  assert.match(workflow, /node scripts\/ci-workflow-plan\.mjs run --row/);
  assert.match(workflow, /node scripts\/ci-workflow-plan\.mjs verify/);
  assert.match(workflow, /AKRA_CI_SELECTED_JOB_RESULT:.*needs\.selected_rust\.result/);
  assert.match(workflow, /merge-multiple: false/);
  assert.match(workflow, /overwrite: true/);
  assert.match(workflow, /name: \$\{\{ needs\.scope\.outputs\.impact-plan-artifact \}\}/);
  assert.match(workflow, /if-no-files-found: error/);
  assert.match(workflow, /AKRA_CI_SELECTED_RUST_REQUIRED:.*impact-mode/);
  assert.match(workflow, /if: runner\.os != 'Windows'/);
});
