#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { createValidationPlan, explainPlan, verifyValidationPlanHash } from "./ci-impact-plan.mjs";
import { capturePlanInputs, collectPlanInput, hostConfiguration } from "./ci-plan-inputs.mjs";
import { runStreamingCommand } from "./ci-process-progress.mjs";

const PLATFORM = { win32: "windows", linux: "linux", darwin: "macos" };
function step(label, command, args, condition = {}) {
  return { label, command, args, ...condition };
}
function configurationForExecution(plan, platform, architecture) {
  const os = PLATFORM[platform];
  if (!os || plan.configuration.platforms.length !== 1 || plan.configuration.platforms[0] !== os) {
    throw new Error("local execution requires a plan for the current OS only");
  }
  if (plan.configuration.targetTriples.length > 1) throw new Error("local execution accepts one native target triple at a time");
  const target = plan.configuration.targetTriples[0];
  if (target && ((platform === "win32" && !target.includes("windows"))
      || (platform === "linux" && !target.includes("linux"))
      || (platform === "darwin" && !target.includes("apple-darwin")))) {
    throw new Error("the requested target cannot run native tests on this OS");
  }
  const targetArchitectures = { x64: ["x86_64"], arm64: ["aarch64"], ia32: ["i586", "i686"], arm: ["armv7"] };
  if (target && !targetArchitectures[architecture]?.includes(target.split("-")[0])) {
    throw new Error("the requested target architecture differs from this native host");
  }
  return { os, target, profile: plan.configuration.profile, features: plan.configuration.features };
}

/** Structured argv only. No shell command is assembled or evaluated. */
export function commandsForPlan(plan, platform = process.platform, { forceFull = false, architecture = process.arch } = {}) {
  if (!verifyValidationPlanHash(plan)) throw new Error("validation plan hash is invalid");
  const config = configurationForExecution(plan, platform, architecture);
  const commands = [step("diff-check", "git", ["diff", "--check"])];
  if (plan.mode === "full" || forceFull) {
    if (config.profile !== "test") throw new Error("the existing full gate uses its standard test profile; regenerate the plan with --profile test");
    if (config.target && !(platform === "win32" && config.target === "x86_64-pc-windows-msvc")) {
      throw new Error("the existing full gate cannot honor this explicit target; use its standard host configuration");
    }
    commands.push(platform === "win32"
      ? step("full-native-gate", "powershell", ["-NoProfile", "-File", "scripts/check_native_pr.ps1", "-Mode", "Full"])
      : step("full-native-gate", "bash", ["scripts/check_native_pr.sh"]));
    return commands;
  }
  if (plan.docsOnly) return commands;
  const rustPackages = new Map([...plan.compilePackages, ...plan.testPackages].map((pkg) => [pkg.packageKey, pkg]));
  if (rustPackages.size) {
    commands.push(step("crate-boundaries", process.execPath, ["scripts/check_crate_boundaries.mjs"]));
    commands.push(step("rust-format", "cargo", ["fmt", "--all", "--", "--check"]));
  }
  for (const pkg of plan.testPackages) {
    if (pkg.coverage !== "full" || pkg.targets.some((target) => !["lib", "bin", "test"].includes(target.kind))) {
      throw new Error("unsupported selective target/coverage; request the explicit full gate");
    }
    if (pkg.targets.some((target) => target.requiredFeatures.length > 0) && config.features.includes("default")) {
      throw new Error("feature-gated targets require the explicit full gate until default feature coverage is registered");
    }
    const contracts = plan.contractTests.filter((contract) => contract.packageKey === pkg.packageKey && contract.platforms.includes(config.os));
    for (const contract of contracts) {
      if (contract.target.kind !== "lib" || contract.isolation !== "process-per-filter"
          || !["all", "default", "default-and-all"].includes(contract.features)) {
        throw new Error("unsupported contract target/features; request the explicit full gate");
      }
      if (contract.features !== "default-and-all" && config.features.some((mode) => mode !== contract.features)) {
        throw new Error("mixed feature modes for a scoped contract require the explicit full gate");
      }
      if (contract.features === "all" && !config.features.includes("all")) throw new Error("the selected contracts require the all feature mode");
      if (contract.features === "default" && !config.features.includes("default")) throw new Error("the selected contracts require the default feature mode");
    }
    for (const features of config.features) {
      const filters = [...new Set(contracts.filter((contract) => contract.features === features || contract.features === "default-and-all").map((contract) => contract.filter))].sort();
      const args = [
        "scripts/run_native_rust_contracts.mjs", "--include-members", "--package", pkg.name,
        "--profile", config.profile, "--timings", ...(features === "all" ? ["--all-features"] : []),
        ...(config.target ? ["--target", config.target] : []), "--", ...filters,
      ];
      commands.push(step("tests-" + pkg.name + "-" + features, process.execPath, args, { package: pkg.name, features, profile: config.profile, target: config.target ?? null }));
    }
  }
  for (const pkg of [...rustPackages.values()].sort((a, b) => a.packageKey < b.packageKey ? -1 : a.packageKey > b.packageKey ? 1 : 0)) {
    commands.push(step("clippy-" + pkg.name, "cargo", [
      "clippy", "--locked", "--package", pkg.name, "--all-targets", "--all-features",
      "--profile", config.profile, ...(config.target ? ["--target", config.target] : []), "--", "-D", "warnings",
    ], { package: pkg.name, features: "all", profile: config.profile, target: config.target ?? null }));
  }
  if (plan.nodeRequired) {
    commands.push(platform === "win32"
      ? step("node-surfaces", "powershell", ["-NoProfile", "-File", "scripts/check_native_pr.ps1", "-Mode", "Admin"])
      : step("node-surfaces", "bash", ["scripts/check_node_surfaces.sh"]));
  }
  return commands;
}

export function assertCurrentPlan(saved, current) {
  if (!verifyValidationPlanHash(saved) || !verifyValidationPlanHash(current)) throw new Error("validation plan integrity check failed");
  if (saved.planHash !== current.planHash) {
    throw new Error("stale validation plan: source, local contents, configuration, ownership or graph changed; generate a new plan");
  }
}
export async function executeMeasured(command, {
  cwd, environment = process.env, planHash, stream = runStreamingCommand,
}) {
  const args = [
    resolve(cwd, "scripts/ci-performance.mjs"), "run",
    "local-" + planHash.slice(0, 12) + "-" + command.label, "--", command.command, ...command.args,
  ];
  const env = {
    ...environment, AKRA_CI_PROFILE: command.profile ?? "test",
    AKRA_CI_FEATURES: command.features ?? "full-or-non-rust",
    AKRA_CI_TARGET: command.target ?? environment.CARGO_BUILD_TARGET ?? "host",
  };
  // The shared supervisor owns the wrapper's POSIX group or Windows descendant tree.
  const result = await stream(process.execPath, args, { cwd, env });
  if (result.aborted || result.signal) {
    return result.signal === "SIGINT" ? 130 : result.signal === "SIGTERM" ? 143 : 1;
  }
  return result.status ?? 1;
}

/** Recollects immediately before commands, then rejects changed input contents after validation. */
export async function runValidationPlan(plan, {
  cwd = process.cwd(), offline = true, forceFull = false,
  collect = collectPlanInput, capture = capturePlanInputs, execute = executeMeasured,
  platform = process.platform, environment = process.env,
} = {}) {
  if (!verifyValidationPlanHash(plan)) throw new Error("validation plan hash is invalid");
  const request = collect({
    cwd, ...(plan.baseSha ? { baseSha: plan.baseSha } : { baseRef: "__unavailable_validation_base__" }),
    headSha: plan.sourceSha, configuration: plan.configuration, offline, environment,
  });
  const current = createValidationPlan(request), executionCwd = request.head.metadata.workspace_root;
  assertCurrentPlan(plan, current);
  const commands = commandsForPlan(current, platform, { forceFull });
  for (const command of commands) {
    console.log("argv: " + JSON.stringify([command.command, ...command.args]));
    const exitCode = await execute(command, { cwd: executionCwd, environment, planHash: current.planHash });
    if (exitCode !== 0) throw new Error("validation command failed (" + exitCode + "): " + command.label);
  }
  const after = capture({ cwd: executionCwd, environment });
  if (after.sourceSha !== current.sourceSha || after.localChangeId !== current.localChangeId) {
    throw new Error("validation inputs changed during execution; this run is not current success evidence");
  }
  return { mode: forceFull ? "full" : current.mode, planHash: current.planHash, commands };
}

export function parseOptions(args) {
  const options = { action: "explain", baseRef: "origin/prerelease", offline: true, forceFull: false };
  const config = hostConfiguration();
  let actionSet = false;
  const seen = new Set();
  for (let index = 0; index < args.length; index++) {
    const arg = args[index];
    if (seen.has(arg)) throw new Error("duplicate argument: " + arg);
    seen.add(arg);
    if (["--json", "--explain", "--run"].includes(arg)) {
      if (actionSet) throw new Error("--json, --explain and --run are separate actions");
      options.action = arg.slice(2);
      actionSet = true;
    } else if (arg === "--online") {
      options.offline = false;
    } else if (arg === "--full") {
      options.forceFull = true;
    } else if (["--base", "--plan", "--features", "--profile", "--target"].includes(arg)) {
      const value = args[++index];
      if (!value || value.startsWith("--")) throw new Error(arg + " requires a value");
      if (arg === "--base") options.baseRef = value;
      if (arg === "--plan") options.planPath = value;
      if (arg === "--features") {
        if (!["default", "all", "both"].includes(value)) throw new Error("--features accepts default, all or both");
        config.features = value === "both" ? ["all", "default"] : [value];
      }
      if (arg === "--profile") config.profile = value;
      if (arg === "--target") config.targetTriples = [value];
    } else {
      throw new Error("unknown argument: " + arg);
    }
  }
  if (options.planPath && ["--base", "--features", "--profile", "--target"].some((flag) => seen.has(flag))) {
    throw new Error("a saved plan already fixes its base and configuration; generate a new plan to change them");
  }
  if (options.forceFull && options.action !== "run") throw new Error("--full is an execution option; use --run --full");
  return { ...options, configuration: config };
}
async function main(args) {
  const options = parseOptions(args), cwd = process.cwd();
  const plan = options.planPath
    ? JSON.parse(readFileSync(options.planPath, "utf8").replace(/^\uFEFF/, ""))
    : createValidationPlan(collectPlanInput({ cwd, baseRef: options.baseRef, configuration: options.configuration, offline: options.offline }));
  if (!verifyValidationPlanHash(plan)) throw new Error("saved validation plan hash is invalid");
  if (options.action === "json") {
    console.log(JSON.stringify(plan, null, 2));
    return;
  }
  console.log(explainPlan(plan));
  if (options.action === "run") {
    const result = await runValidationPlan(plan, { cwd, offline: options.offline, forceFull: options.forceFull });
    console.log(result.mode === "full"
      ? "Full validation passed using the existing native gate."
      : "Selected validation passed. This result covers only the displayed packages, contracts and Node surfaces.");
  } else {
    console.log("\nRequired local argv:");
    for (const command of commandsForPlan(plan)) console.log("  " + JSON.stringify([command.command, ...command.args]));
    console.log("\nRun this plan with node scripts/agent-plan.mjs --run using the same inputs.");
  }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
