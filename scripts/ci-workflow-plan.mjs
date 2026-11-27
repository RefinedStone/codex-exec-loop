#!/usr/bin/env node
import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import { appendFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { createValidationPlan, explainPlan, verifyValidationPlanHash } from "./ci-impact-plan.mjs";
import { assertCleanPlanCheckout } from "./ci-plan-inputs.mjs";
import { runNativeContracts } from "./run_native_rust_contracts.mjs";

const HOSTS = {
  linux: { runner: "ubuntu-latest", targetTriple: "x86_64-unknown-linux-gnu", platform: "linux", architecture: "x64" },
  windows: { runner: "windows-latest", targetTriple: "x86_64-pc-windows-msvc", platform: "win32", architecture: "x64" },
  macos: { runner: "macos-15", targetTriple: "aarch64-apple-darwin", platform: "darwin", architecture: "arm64" },
};
const coordinates = ["packageKey", "package", "manifest", "target", "os", "features", "profile", "targetTriple", "architecture"];
const stable = (value) => Array.isArray(value) ? value.map(stable) : value && typeof value === "object"
  ? Object.fromEntries(Object.keys(value).sort().map((key) => [key, stable(value[key])])) : value;
const encode = (value) => JSON.stringify(stable(value));
const digest = (value) => createHash("sha256").update(encode(value)).digest("hex");
const unique = (values) => [...new Set(values)].sort();
function requirePlan(plan) {
  if (!verifyValidationPlanHash(plan)) throw new Error("Invalid validation plan hash.");
}
function rowCoordinates(row) { return Object.fromEntries(coordinates.map((key) => [key, row[key]])); }

/** Expand only proven complete package coverage; unsupported cases retain the full gate. */
export function createCIExecutionPlan(plan, { forceFull = false } = {}) {
  requirePlan(plan);
  const reasons = new Set(plan.mode === "full" ? plan.fullReasons : []);
  if (forceFull) reasons.add("event-requires-full-validation");
  if (plan.configuration.targetTriples.length) reasons.add("custom-target-triples-require-full-validation");
  let rows = [];
  if (!reasons.size) {
    for (const pkg of plan.compilePackages) for (const target of pkg.targets) {
      const tested = plan.testPackages.find((candidate) => candidate.packageKey === pkg.packageKey)?.targets
        .some((candidate) => candidate.kind === target.kind && candidate.name === target.name && candidate.test);
      if (!tested) reasons.add("compile-target-not-covered:" + pkg.packageKey + ":" + target.kind + ":" + target.name);
    }
    for (const pkg of plan.testPackages) {
      if (pkg.name === "akra-ade" || pkg.manifest === "Cargo.toml" || pkg.coverage !== "full") {
        reasons.add("root-or-partial-package-requires-full-validation");
      }
      for (const target of pkg.targets) {
        if (target.requiredFeatures.length) reasons.add("feature-gated-target-requires-full-validation");
        if (!["lib", "bin", "test"].includes(target.kind)) reasons.add("unsupported-target-kind:" + target.kind);
        const targets = [];
        if (target.test) targets.push({ kind: target.kind, name: target.name });
        if (target.doctest && target.kind === "lib") targets.push({ kind: "doc", name: target.name });
        if (target.doctest && target.kind !== "lib") reasons.add("unsupported-doctest-target");
        for (const selectedTarget of targets) for (const os of plan.configuration.platforms) for (const features of plan.configuration.features) {
          const host = HOSTS[os];
          if (!host) throw new Error("Unsupported CI host.");
          const coordinate = { packageKey: pkg.packageKey, package: pkg.name, manifest: pkg.manifest,
            target: selectedTarget, os, features, profile: plan.configuration.profile,
            targetTriple: host.targetTriple, architecture: host.architecture };
          const contracts = plan.contractTests.filter((item) => item.packageKey === pkg.packageKey
            && item.target.kind === selectedTarget.kind && item.target.name === selectedTarget.name
            && item.platforms.includes(os) && item.features === features);
          rows.push({ ...coordinate, rowId: digest(coordinate).slice(0, 24), runner: host.runner,
            contractFilters: unique(contracts.map((item) => item.filter)) });
        }
      }
    }
    // A required contract may never disappear due to an unsupported condition or target.
    for (const contract of plan.contractTests) {
      if (contract.isolation !== "process-per-filter" || !["all", "default"].includes(contract.features)) {
        reasons.add("unsupported-contract-condition:" + contract.id);
      }
      for (const os of contract.platforms) {
        if (!rows.some((row) => row.packageKey === contract.packageKey && row.os === os
          && row.features === contract.features && row.target.kind === contract.target.kind
          && row.target.name === contract.target.name && row.contractFilters.includes(contract.filter))) {
          reasons.add("required-contract-not-covered:" + contract.id + ":" + os);
        }
      }
    }
    if (!rows.length) reasons.add("no-selected-rust-targets");
    if (rows.length > 256) reasons.add("matrix-exceeds-actions-limit");
    if (new Set(rows.map((row) => row.rowId)).size !== rows.length) throw new Error("Duplicate CI matrix coordinate.");
  }
  if (reasons.size) rows = [];
  rows.sort((a, b) => a.rowId.localeCompare(b.rowId));
  const result = { schemaVersion: 1, sourceSha: plan.sourceSha, planHash: plan.planHash,
    forceFull, mode: reasons.size ? "full" : "selective", fallbackReasons: [...reasons].sort(), rows };
  return { ...result, matrixHash: digest(result) };
}

export function verifyCIExecutionPlan(plan, matrix) {
  requirePlan(plan);
  if (!matrix || typeof matrix.forceFull !== "boolean"
    || encode(matrix) !== encode(createCIExecutionPlan(plan, { forceFull: matrix.forceFull }))) {
    throw new Error("CI matrix does not match the immutable validation plan.");
  }
  return matrix;
}

export function validateCIResults(plan, matrix, results) {
  verifyCIExecutionPlan(plan, matrix);
  if (matrix.mode !== "selective" || !matrix.rows.length) throw new Error("An empty or full matrix cannot pass the selected gate.");
  if (!Array.isArray(results)) throw new Error("CI result collection is missing.");
  const expected = new Map(matrix.rows.map((row) => [row.rowId, row]));
  const seen = new Set();
  for (const result of results) {
    const row = expected.get(result?.rowId);
    if (!row || seen.has(result.rowId)) throw new Error("Unexpected or duplicate CI result.");
    seen.add(result.rowId);
    if (result.schemaVersion !== 1 || result.sourceSha !== plan.sourceSha
      || result.planHash !== plan.planHash || result.matrixHash !== matrix.matrixHash
      || encode(rowCoordinates(result)) !== encode(rowCoordinates(row))) {
      throw new Error("Stale or mismatched CI result conditions.");
    }
    if (result.status !== "success" || result.exitCode !== 0) throw new Error("Required CI row did not succeed: " + result.rowId);
  }
  if (seen.size !== expected.size) throw new Error("Missing required CI results.");
  return { accepted: true, rows: seen.size, sourceSha: plan.sourceSha, planHash: plan.planHash };
}

/** Keep artifact directories separate so colliding result filenames cannot overwrite one another. */
export function readCIResultArtifacts(directory, runId) {
  if (!/^[1-9][0-9]*$/.test(String(runId ?? ""))) throw new Error("Missing CI run ID.");
  const prefix = "ci-plan-result-" + runId + "-";
  return readdirSync(directory, { withFileTypes: true }).map((artifact) => {
    if (!artifact.isDirectory() || !artifact.name.startsWith(prefix)) throw new Error("Unexpected result artifact directory.");
    const rowId = artifact.name.slice(prefix.length);
    if (!/^[a-f0-9]{24}$/.test(rowId)) throw new Error("Invalid result artifact row ID.");
    const artifactPath = join(directory, artifact.name);
    const entries = readdirSync(artifactPath, { withFileTypes: true });
    if (entries.length !== 1 || !entries[0].isFile() || entries[0].name !== rowId + ".json") {
      throw new Error("Result artifact must contain exactly its named row proof.");
    }
    const result = JSON.parse(readFileSync(join(artifactPath, entries[0].name), "utf8"));
    if (result.rowId !== rowId) throw new Error("Artifact name does not match result row ID.");
    return result;
  });
}

export function commandsForCIRow(row) {
  if (!HOSTS[row.os] || !["all", "default"].includes(row.features)) throw new Error("Unsupported CI row conditions.");
  const options = [...(row.features === "all" ? ["--all-features"] : []), "--profile", row.profile, "--target", row.targetTriple, "--timings"];
  if (row.target.kind === "doc") return [{ command: "cargo", args: ["test", "--locked", "--package", row.package, "--doc", ...options] }];
  if (!["lib", "bin", "test"].includes(row.target.kind)) throw new Error("Unsupported test harness kind.");
  return [{ command: process.execPath, args: ["scripts/run_native_rust_contracts.mjs", "--all-tests",
    "--package", row.package, "--target-kind", row.target.kind, "--target-name", row.target.name,
    ...options, "--", ...row.contractFilters] }];
}

export function assertCleanSource(sourceSha, cwd = process.cwd(), environment = process.env) {
  const state = assertCleanPlanCheckout({ cwd, environment });
  if (state.sourceSha !== sourceSha) throw new Error("CI checkout changed or does not match the planned source SHA.");
}

export async function runSelectedRow(plan, matrix, rowId, {
  cwd = process.cwd(), env = process.env, platform = process.platform, architecture = process.arch,
  checkSource = assertCleanSource, contracts = runNativeContracts, spawn = spawnSync,
} = {}) {
  verifyCIExecutionPlan(plan, matrix);
  if (env.AKRA_CI_PLAN_HASH && env.AKRA_CI_PLAN_HASH !== plan.planHash) throw new Error("Downloaded plan differs from the Scope output hash.");
  const row = matrix.rows.find((candidate) => candidate.rowId === rowId);
  if (matrix.mode !== "selective" || !row) throw new Error("Requested CI row is not planned.");
  if (HOSTS[row.os].platform !== platform || row.architecture !== architecture) throw new Error("CI host does not match planned OS/architecture.");
  checkSource(plan.sourceSha, cwd, env);
  const command = commandsForCIRow(row)[0];
  const startedAt = new Date().toISOString();
  const started = performance.now();
  const record = { schemaVersion: 1, rowId, ...rowCoordinates(row), sourceSha: plan.sourceSha,
    planHash: plan.planHash, matrixHash: matrix.matrixHash, startedAt, command };
  try {
    const runtimeEnv = { ...env, AKRA_CI_PLAN_HASH: plan.planHash };
    let tests;
    if (row.target.kind === "doc") {
      const result = spawn(command.command, command.args, { cwd, env: runtimeEnv, stdio: "inherit" });
      if (result.error || result.signal || result.status !== 0) throw new Error("Doctest failed: " + (result.error?.message ?? result.signal ?? result.status));
    } else {
      tests = await contracts(command.args.slice(1), { cwd, env: runtimeEnv });
    }
    checkSource(plan.sourceSha, cwd, env);
    return { ...record, status: "success", exitCode: 0, durationMs: Math.round(performance.now() - started), tests };
  } catch (error) {
    return { ...record, status: error.cancelled ? "cancelled" : "failed", exitCode: 1,
      durationMs: Math.round(performance.now() - started), error: error.message };
  }
}

function parseArguments(args) {
  const command = args.shift();
  const values = new Map();
  const allowed = { create: ["--directory"], run: ["--directory", "--row", "--result"], verify: ["--directory", "--results"] }[command];
  if (!allowed) throw new Error("usage: ci-workflow-plan.mjs <create|run|verify> [options]");
  while (args.length) {
    const option = args.shift(), value = args.shift();
    if (!allowed.includes(option) || !value || value.startsWith("--") || values.has(option)) throw new Error("Invalid workflow plan arguments.");
    values.set(option, value);
  }
  return { command, values };
}

export async function main(args, env = process.env) {
  const { command, values } = parseArguments([...args]);
  const directory = resolve(values.get("--directory") ?? "artifacts/ci-plan");
  if (command === "create") {
    const { collectPlanInput } = await import("./ci-plan-inputs.mjs");
    const headSha = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
    assertCleanSource(env.GITHUB_SHA || headSha);
    const rawBase = env.AKRA_CI_BASE_SHA;
    const request = await collectPlanInput({ headSha,
      ...(rawBase && !/^0+$/.test(rawBase) ? { baseSha: rawBase } : {}), offline: false, requireCleanCheckout: true,
      configuration: { platforms: ["linux", "macos", "windows"], features: ["default", "all"], profile: "test", targetTriples: [] } });
    const plan = createValidationPlan(request);
    assertCleanSource(plan.sourceSha);
    const matrix = createCIExecutionPlan(plan, { forceFull: env.GITHUB_EVENT_NAME !== "pull_request" || (env.AKRA_CI_REQUESTED_SCOPE || "auto") !== "auto" });
    mkdirSync(directory, { recursive: true });
    writeFileSync(join(directory, "plan.json"), JSON.stringify(plan, null, 2) + "\n");
    writeFileSync(join(directory, "matrix.json"), JSON.stringify(matrix, null, 2) + "\n");
    if (env.GITHUB_OUTPUT) for (const [key, value] of Object.entries({
      "impact-mode": matrix.mode, "impact-plan-hash": plan.planHash, "impact-source-sha": plan.sourceSha,
      "impact-node-required": String(plan.nodeRequired),
      "impact-plan-artifact": "ci-validation-plan-" + (env.GITHUB_RUN_ID ?? "local") + "-" + (env.GITHUB_RUN_ATTEMPT ?? "1"),
      "impact-has-rows": String(matrix.rows.length > 0), "impact-matrix": JSON.stringify({ include: matrix.rows }),
    })) appendFileSync(env.GITHUB_OUTPUT, key + "=" + value + "\n");
    const summary = explainPlan(plan) + "\nCI execution: " + matrix.mode + "; rows: " + matrix.rows.length
      + "\n" + matrix.fallbackReasons.map((reason) => "Full: " + reason).join("\n") + "\n";
    if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY, "## Immutable validation plan\n\n```text\n" + summary + "```\n");
    console.log(summary);
    return { plan, matrix };
  }
  const plan = JSON.parse(readFileSync(join(directory, "plan.json"), "utf8"));
  const matrix = JSON.parse(readFileSync(join(directory, "matrix.json"), "utf8"));
  if (env.AKRA_CI_PLAN_HASH && env.AKRA_CI_PLAN_HASH !== plan.planHash) throw new Error("Downloaded plan differs from the Scope output hash.");
  if (command === "run") {
    const rowId = values.get("--row");
    if (!rowId || !/^[a-f0-9]{24}$/.test(rowId)) throw new Error("A valid --row is required.");
    const result = await runSelectedRow(plan, matrix, rowId, { env });
    const path = resolve(values.get("--result") ?? join("artifacts/ci-plan-results", rowId + ".json"));
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, JSON.stringify(result, null, 2) + "\n");
    if (result.status !== "success") process.exitCode = 1;
    return result;
  }
  assertCleanSource(plan.sourceSha);
  if (env.AKRA_CI_SELECTED_JOB_RESULT !== "success") throw new Error("Selected Rust matrix job did not succeed.");
  const resultsDirectory = resolve(values.get("--results") ?? "artifacts/ci-plan-results");
  const results = readCIResultArtifacts(resultsDirectory, env.GITHUB_RUN_ID);
  const accepted = validateCIResults(plan, matrix, results);
  console.log(JSON.stringify(accepted));
  return accepted;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main(process.argv.slice(2)).catch((error) => { console.error(error.message); process.exitCode = 1; });
}
