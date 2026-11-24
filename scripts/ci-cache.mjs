#!/usr/bin/env node
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { appendFileSync, lstatSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const digest = (value) => createHash("sha256").update(JSON.stringify(value)).digest("hex").slice(0, 24);
const component = (name, value) => {
  if (typeof value !== "string" || !value || /[\r\n]/.test(value)) throw new Error(`missing or invalid ${name}`);
  return value.replace(/[^a-zA-Z0-9._-]/g, "-");
};
export function maySaveCache(eventName, ref) {
  return eventName === "push" && ["refs/heads/main", "refs/heads/prerelease"].includes(ref);
}
export function createCachePolicy(input) {
  const { os, architecture, lane, features, profile, target, compiler, manifestHash, lockHash,
    revision, targetDirectory, cargoHome, buildEnvironment = {}, eventName, ref } = input;
  if (!/^[0-9a-f]{40,64}$/i.test(revision ?? "")) throw new Error("cache revision must be a full source SHA");
  if (!compiler || !manifestHash || !lockHash) throw new Error("compiler, manifest hash and lock hash are required");
  const labels = [component("os", os), component("architecture", architecture), component("lane", lane),
    component("target", target), component("features", features), component("profile", profile)];
  const boundary = { os, architecture, lane, target, features, profile, compiler, manifestHash, lockHash,
    targetDirectory: resolve(targetDirectory), buildEnvironmentHash: digest(buildEnvironment) };
  const targetPrefix = `akra-target-v2-${labels.join("-")}-${digest(boundary)}-`;
  const registryPrefix = `akra-registry-v2-${component("os", os)}-${component("architecture", architecture)}-`;
  const registryPaths = ["registry/index", "registry/cache", "git/db"].map((path) => join(cargoHome, path));
  return { schemaVersion: 1, boundary, sourceRevision: revision, saveAllowed: maySaveCache(eventName, ref),
    registry: { key: registryPrefix + digest(lockHash), restorePrefix: registryPrefix, paths: registryPaths },
    target: { key: targetPrefix + revision, restorePrefix: targetPrefix, paths: [resolve(targetDirectory)] } };
}

/** 논리 파일 크기만 계측한다. symlink/junction을 따라 다른 디렉터리를 순회하지 않는다. */
export function directorySize(paths) {
  const result = { bytes: 0, files: 0, skippedLinks: 0, missingPaths: 0, errors: [] };
  const visit = (path) => {
    let stat;
    try { stat = lstatSync(path); }
    catch (error) {
      if (error.code === "ENOENT") result.missingPaths += 1;
      else result.errors.push({ path, code: error.code });
      return;
    }
    if (stat.isSymbolicLink()) { result.skippedLinks += 1; return; }
    if (stat.isFile()) { result.bytes += stat.size; result.files += 1; return; }
    if (!stat.isDirectory()) return;
    let children;
    try { children = readdirSync(path); }
    catch (error) { result.errors.push({ path, code: error.code }); return; }
    for (const child of children) visit(join(path, child));
  };
  paths.forEach(visit);
  return result;
}
export function cacheObservation(policy, phase, env = process.env) {
  if (!["restored", "complete"].includes(phase)) throw new Error("cache phase must be restored or complete");
  const started = performance.now();
  const caches = {};
  for (const type of ["registry", "target"]) {
    const prefix = `AKRA_CI_CACHE_${type.toUpperCase()}_`;
    const primaryKey = env[prefix + "PRIMARY"] || policy[type].key;
    const matchedKey = env[prefix + "MATCHED"] || null;
    const outcome = env[prefix + "OUTCOME"] || "unknown";
    caches[type] = {
      primaryKey, matchedKey, restorePrefix: policy[type].restorePrefix, restoreOutcome: outcome,
      hit: outcome !== "success" ? "unavailable" : !matchedKey ? "miss" : matchedKey === primaryKey ? "exact" : "fallback",
      exactHitOutput: env[prefix + "HIT"] === "true", localSize: directorySize(policy[type].paths),
      saveOutcome: phase === "complete" ? env[prefix + "SAVE"] || "skipped" : null,
    };
  }
  return { schemaVersion: 1, phase, capturedAt: new Date().toISOString(), sourceRevision: policy.sourceRevision,
    saveAllowed: policy.saveAllowed, boundary: policy.boundary, caches,
    sizeScanMilliseconds: Math.round(performance.now() - started) };
}

/** Clear only restored diagnostics so a new artifact cannot claim old Cargo timings. */
export function clearRestoredTimings(targetDirectory) {
  let children;
  try {
    if (lstatSync(targetDirectory).isSymbolicLink()) return [];
    children = readdirSync(targetDirectory, { withFileTypes: true });
  }
  catch (error) { if (error.code === "ENOENT") return []; throw error; }
  const candidates = [join(targetDirectory, "cargo-timings"),
    ...children.filter((entry) => entry.isDirectory() && !entry.isSymbolicLink())
      .map((entry) => join(targetDirectory, entry.name, "cargo-timings"))];
  const removed = [];
  for (const path of candidates) {
    try { lstatSync(path); }
    catch (error) { if (error.code === "ENOENT") continue; throw error; }
    rmSync(path, { recursive: true, force: true });
    removed.push(path);
  }
  return removed;
}

function outputs(policy, outputPath) {
  if (!outputPath) return;
  const values = {
    "registry-key": policy.registry.key, "registry-prefix": policy.registry.restorePrefix,
    "target-key": policy.target.key, "target-prefix": policy.target.restorePrefix,
    "target-directory": policy.target.paths[0], "save-allowed": String(policy.saveAllowed),
  };
  for (const [key, value] of Object.entries(values)) {
    if (/[\r\n]/.test(value)) throw new Error("multiline cache output rejected");
    appendFileSync(outputPath, `${key}=${value}\n`);
  }
  appendFileSync(outputPath, `registry-paths<<AKRA_CACHE_PATHS\n${policy.registry.paths.join("\n")}\nAKRA_CACHE_PATHS\n`);
}
function plan(env) {
  const buildEnvironment = Object.fromEntries(Object.entries(env).filter(([key]) =>
    ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTDOCFLAGS", "CARGO_ENCODED_RUSTDOCFLAGS", "CARGO_INCREMENTAL", "CARGO_BUILD_TARGET"].includes(key)
    || /^CARGO_PROFILE_/.test(key) || /^CARGO_TARGET_.*_(RUSTFLAGS|LINKER)$/.test(key)).sort(([a], [b]) => a.localeCompare(b)));
  const compiler = execFileSync("rustc", ["--version", "--verbose"], { encoding: "utf8" }).trim();
  const host = /^host: (.+)$/m.exec(compiler)?.[1];
  if (!host) throw new Error("rustc did not report its host target");
  const configuredTarget = env.AKRA_CI_TARGET || env.CARGO_BUILD_TARGET;
  return createCachePolicy({
    os: env.RUNNER_OS, architecture: env.RUNNER_ARCH, lane: env.AKRA_CI_CACHE_LANE,
    features: env.AKRA_CI_FEATURES, profile: env.AKRA_CI_PROFILE,
    target: configuredTarget && configuredTarget !== "host" ? "explicit-" + configuredTarget : "implicit-" + host,
    compiler, manifestHash: env.AKRA_CI_CACHE_MANIFEST_HASH, lockHash: env.AKRA_CI_CACHE_LOCK_HASH,
    revision: env.GITHUB_SHA, eventName: env.GITHUB_EVENT_NAME, ref: env.GITHUB_REF,
    targetDirectory: env.CARGO_TARGET_DIR || "target", cargoHome: env.CARGO_HOME || join(homedir(), ".cargo"), buildEnvironment,
  });
}
export function main(args, env = process.env) {
  const directory = env.AKRA_CI_MEASUREMENTS || "artifacts/ci-performance";
  const policyPath = join(directory, "cache-policy.json");
  mkdirSync(directory, { recursive: true });
  if (args[0] === "plan") {
    const policy = plan(env);
    writeFileSync(policyPath, JSON.stringify(policy, null, 2) + "\n");
    outputs(policy, env.GITHUB_OUTPUT);
    console.log(JSON.stringify(policy, null, 2));
  } else if (args[0] === "clear-timings") {
    const policy = JSON.parse(readFileSync(policyPath, "utf8"));
    console.log(JSON.stringify({ removedDiagnostics: clearRestoredTimings(policy.target.paths[0]) }));
  } else if (args[0] === "record" && args.length === 2) {
    const policy = JSON.parse(readFileSync(policyPath, "utf8"));
    const record = cacheObservation(policy, args[1], env);
    appendFileSync(join(directory, "cache-snapshots.jsonl"), JSON.stringify(record) + "\n");
    const lines = ["## Rust cache: " + args[1], "", "| Cache | Restore | Local bytes | Files | Save |", "|---|---|---:|---:|---|",
      ...Object.entries(record.caches).map(([name, cache]) => `| ${name} | ${cache.hit} | ${cache.localSize.bytes} | ${cache.localSize.files} | ${cache.saveOutcome ?? "pending"} |`),
      "", "Bytes are uncompressed local files, not archive/network size. Cache action step duration comes from the Actions jobs API.", ""];
    const markdown = lines.join("\n");
    appendFileSync(join(directory, "cache-summary.md"), markdown);
    if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY, markdown);
    console.log(markdown);
  } else throw new Error("usage: ci-cache.mjs plan | clear-timings | record <restored|complete>");
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main(process.argv.slice(2));
