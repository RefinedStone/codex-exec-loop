#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { appendFileSync, mkdirSync, readFileSync } from "node:fs";
import { performance } from "node:perf_hooks";
import { dirname, posix, resolve, win32 } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { runStreamingCommand } from "./ci-process-progress.mjs";
import {
  exactTestExecutable, parseTestInventory, planContractFilters, remainingMemberTests, resolveContractTargets,
} from "./ci-contract-targets.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const booleanOptions = new Set(["--all-features", "--no-default-features", "--timings"]);
const valueOptions = new Set(["--features", "--target", "--profile", "--target-dir"]);
const selectors = new Map([
  ["--package", "package"], ["--target-kind", "targetKind"], ["--target-name", "targetName"],
  ["--contract-id", "contractId"],
]);

export function parseContractArguments(argv) {
  const separator = argv.indexOf("--");
  if (separator < 0) throw new Error("Separate Cargo options and required test filters with --.");
  const input = argv.slice(0, separator);
  const filters = argv.slice(separator + 1);
  const options = [];
  const selection = {};
  let includeMembers = false;
  const seen = new Set();
  for (let index = 0; index < input.length; index += 1) {
    const option = input[index];
    if (seen.has(option)) throw new Error("Duplicate option: " + option);
    seen.add(option);
    if (option === "--include-members") { includeMembers = true; continue; }
    if (booleanOptions.has(option)) { options.push(option); continue; }
    if (!valueOptions.has(option) && !selectors.has(option)) throw new Error("Unsupported Cargo option: " + option);
    const value = input[++index];
    if (!value || value.startsWith("-")) throw new Error(option + " requires a value.");
    if (selectors.has(option)) selection[selectors.get(option)] = value;
    else options.push(option, value);
  }
  if (seen.has("--all-features") && seen.has("--features")) throw new Error("Use either --all-features or --features.");
  if ((!filters.length && !selection.contractId)
    || filters.some((filter) => !filter.trim() || filter.startsWith("-"))) {
    throw new Error("At least one nonempty test filter is required; libtest flags are not filters.");
  }
  if (new Set(filters).size !== filters.length) throw new Error("Duplicate test filters are not allowed.");
  if ((selection.targetKind || selection.targetName) && !selection.package) {
    throw new Error("Target selectors require an explicit package.");
  }
  if (selection.contractId && (selection.package || filters.length)) {
    throw new Error("A registry contract ID supplies its own package, target and filter.");
  }
  if (includeMembers && Object.keys(selection).length) throw new Error("--include-members cannot narrow the package selector.");
  return {options, filters, ...(Object.keys(selection).length ? {selection} : {}), ...(includeMembers ? {includeMembers:true} : {})};
}

export const libraryTestExecutable = exactTestExecutable;

export function testRuntimeEnvironment(executable, env = process.env, platform = process.platform) {
  const key = platform === "win32"
    ? Object.keys(env).find((name) => name.toUpperCase() === "PATH") ?? "PATH"
    : platform === "darwin" ? "DYLD_FALLBACK_LIBRARY_PATH" : "LD_LIBRARY_PATH";
  const paths = platform === "win32" ? win32 : posix;
  const directories = [paths.dirname(executable), paths.resolve(paths.dirname(executable), "..")];
  const fallback = platform === "darwin" && !env[key]
    ? [env.HOME && paths.resolve(env.HOME, "lib"), "/usr/local/lib", "/usr/lib"].filter(Boolean)
    : [];
  return { ...env, [key]: [...directories, env[key], ...fallback].filter(Boolean).join(paths.delimiter) };
}

export function createContractProgressWriter({cwd = root, env = process.env, write = (text) => process.stdout.write(text)} = {}) {
  const file = resolve(cwd, env.AKRA_NATIVE_PROGRESS_FILE || ("artifacts/ci-performance/native-contracts-" + process.pid + ".jsonl"));
  return (event) => {
    const line = JSON.stringify({schemaVersion:1, sourceSha:env.GITHUB_SHA ?? null,
      planHash:env.AKRA_CI_PLAN_HASH ?? null, at:new Date().toISOString(), ...event});
    mkdirSync(dirname(file), {recursive:true});
    appendFileSync(file, line + "\n", "utf8");
    write("AKRA_NATIVE_PROGRESS " + line + "\n");
  };
}

function metadataOptions(options) {
  const selected = [];
  for (let index = 0; index < options.length; index += 1) {
    const option = options[index];
    if (["--all-features", "--no-default-features"].includes(option)) selected.push(option);
    else if (valueOptions.has(option)) {
      const value = options[++index];
      if (option === "--features") selected.push(option, value);
      if (option === "--target") selected.push("--filter-platform", value);
    }
  }
  return selected;
}

export async function runNativeContracts(argv, {
  run = spawnSync, cwd = root, env = process.env,
  write = (text) => process.stdout.write(text),
  stream = run === spawnSync ? runStreamingCommand : undefined,
  record = createContractProgressWriter({cwd, env, write}),
  metadata, ownership, platform = process.platform,
} = {}) {
  const parsed = parseContractArguments(argv);
  const { options } = parsed;
  let { filters, selection } = parsed;
  ownership ??= JSON.parse(readFileSync(resolve(cwd, "scripts/ci-crate-ownership.json"), "utf8"));
  const invoke = (command, args, runtimeEnv = env) => {
    const result = run(command, args, {cwd, env:runtimeEnv, encoding:"utf8",
      stdio:["ignore", "pipe", "inherit"], maxBuffer:64 * 1024 * 1024});
    if (result.error) throw result.error;
    if (result.status !== 0 || result.signal) {
      if (result.stdout) write(result.stdout);
      throw new Error(command + " failed (" + (result.signal ?? result.status) + ").");
    }
    return result.stdout ?? "";
  };
  if (selection?.contractId) {
    const matches = (ownership.contracts ?? []).filter((item) => item.id === selection.contractId);
    if (matches.length !== 1) throw new Error("Unknown or ambiguous contract ID.");
    const contract = matches[0];
    const os = {win32:"windows", darwin:"macos", linux:"linux"}[platform];
    if (!contract.platforms.includes(os)) throw new Error("Contract is not defined on this OS.");
    if (contract.features === "all" && !options.includes("--all-features")) {
      throw new Error("Contract requires --all-features.");
    }
    if (contract.isolation !== "process-per-filter" || contract.ignored === true) {
      throw new Error("This runner only executes ordinary process-per-filter contracts; use the exact helper entrypoint for ignored tests.");
    }
    selection = {package:contract.owner, targetKind:contract.target.kind, targetName:contract.target.name};
    filters = [contract.filter];
  }
  metadata ??= JSON.parse(invoke("cargo", ["metadata", "--locked", "--format-version", "1", ...metadataOptions(options)]));
  const requests = resolveContractTargets(metadata, selection, ownership, parsed.includeMembers);
  const buildGroups = new Map();
  for (const request of requests) {
    const key = request.target.kind + ":" + (request.target.kind === "lib" ? "" : request.target.name);
    if (!buildGroups.has(key)) buildGroups.set(key, []);
    buildGroups.get(key).push(request);
  }
  const prepared = [];
  for (const group of buildGroups.values()) {
    const target = group[0].target;
    const targetArgs = target.kind === "lib" ? ["--lib"] : ["--" + target.kind, target.name];
    const packages = group.flatMap((request) => ["--package", request.packageId]);
    const build = invoke("cargo", ["test", "--locked", ...targetArgs, ...packages,
      "--no-run", "--message-format=json", ...options]);
    for (const request of group) {
      const executable = exactTestExecutable(build, request);
      const runtimeEnv = testRuntimeEnvironment(executable, env, platform);
      const inventory = parseTestInventory(invoke(executable, ["--list", "--format", "terse"], runtimeEnv));
      const ignored = parseTestInventory(invoke(executable, ["--list", "--ignored", "--format", "terse"], runtimeEnv));
      if (ignored.some((name) => !inventory.includes(name))) throw new Error("Ignored inventory is not a subset of all tests.");
      prepared.push({...request, executable, runtimeEnv, inventory, ignored});
    }
  }
  const assignments = planContractFilters(prepared, filters, ownership.transfers ?? [], Boolean(selection?.package));
  if (parsed.includeMembers) assignments.push(...remainingMemberTests(prepared, assignments));
  write("Prepared " + prepared.length + " package/target executables for " + filters.length + " native contract filters.\n");
  const optionsValue = (flag, fallback) => {
    const index = options.indexOf(flag); return index < 0 ? fallback : options[index + 1];
  };
  const condition = {os:platform, features:options.includes("--all-features") ? "all" : optionsValue("--features", "default"),
    noDefaultFeatures:options.includes("--no-default-features"), profile:optionsValue("--profile", "test"),
    targetTriple:optionsValue("--target", "host")};
  for (const [index, assignment] of assignments.entries()) {
    const { executable, runtimeEnv, filter, args } = assignment;
    const identity = {contractId:parsed.selection?.contractId ?? filter, filter,
      packageId:assignment.packageId, package:assignment.package,
      target:assignment.target, ...condition};
    write("\n==> Native contract: " + assignment.package + "/" + assignment.target.name + " " + filter + "\n");
    const started = performance.now();
    record({...identity, phase:"started", index:index + 1, total:assignments.length, expectedTests:assignment.names.length});
    try {
      let output;
      if (stream) {
        const result = await stream(executable, args, {cwd, env:runtimeEnv, write});
        if (result.aborted || result.status !== 0 || result.signal) {
          const error = new Error(executable + " failed (" + (result.signal ?? result.status) + ").");
          error.cancelled = Boolean(result.aborted || result.signal);
          throw error;
        }
        output = result.stdout ?? "";
      } else {
        output = invoke(executable, args, runtimeEnv);
        write(output);
      }
      const summary = [...output.matchAll(/^test result: ok\. (\d+) passed; 0 failed;/gm)].at(-1);
      if (!summary || Number(summary[1]) !== assignment.names.length) {
        throw new Error("Native contract did not run the expected non-ignored inventory: " + filter);
      }
      record({...identity, phase:"finished", status:"passed", passed:Number(summary[1]), durationMs:Math.round(performance.now() - started)});
    } catch (error) {
      record({...identity, phase:"finished", status:error.cancelled ? "cancelled" : "failed", durationMs:Math.round(performance.now() - started)});
      throw error;
    }
  }
  if (parsed.includeMembers) {
    for (const pkg of metadata.packages) {
      if (!metadata.workspace_members.includes(pkg.id) || pkg.name === "akra-ade") continue;
      const docTarget = pkg.targets.find((target) => target.kind.includes("lib") && target.doctest === true);
      if (!docTarget) continue;
      const started = performance.now();
      const identity = {contractId:"member-doctest", packageId:pkg.id, package:pkg.name, target:{kind:"doc",name:docTarget.name}, ...condition};
      record({...identity, phase:"started"});
      try {
        write(invoke("cargo", ["test", "--locked", "--package", pkg.id, "--doc", ...options]));
        record({...identity, phase:"finished",status:"passed",durationMs:Math.round(performance.now()-started)});
      } catch (error) {
        record({...identity, phase:"finished",status:"failed",durationMs:Math.round(performance.now()-started)});
        throw error;
      }
    }
  }
  return {preparedTargets:prepared.length, executedGroups:assignments.length,
    executedTests:assignments.reduce((count, assignment) => count + assignment.names.length, 0)};
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  runNativeContracts(process.argv.slice(2)).catch((error) => { console.error(error.message); process.exitCode = 1; });
}
