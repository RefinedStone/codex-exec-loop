#!/usr/bin/env node
// Optional real-Cargo regression. It uses an isolated tiny crate, no registry or app model calls.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { prepareCacheInputs, sealCacheInputs } from "./ci-cache-inputs.mjs";

if (process.argv.length > 3) throw new Error("usage: check_ci_cache_inputs.mjs [output.json]");
const output = resolve(process.argv[2] || "artifacts/ci-performance/cache-inputs-cargo-proof.json");
const root = mkdtempSync(join(tmpdir(), "akra-cache-inputs-cargo-"));
const repository = join(root, "repo"), targetDirectory = join(root, "target");
const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^GIT_/i.test(key)));
Object.assign(env, { CARGO_TARGET_DIR: targetDirectory, CARGO_INCREMENTAL: "0", CARGO_TERM_COLOR: "never",
  AKRA_CACHE_FIXTURE_VALUE: "base", EXPECTED_ANSWER: "42", EXPECTED_PAYLOAD: "first", EXPECTED_BUILD_VALUE: "base" });
const toolchain = execFileSync("rustc", ["--version", "--verbose"], { encoding: "utf8", env }).trim();
const input = { repository, targetDirectory, boundary: { toolchain, features: "default", profile: "test" },
  sourceRevision: "a".repeat(40) };
const records = [];
function git(args) {
  execFileSync("git", ["-C", repository, ...args], { encoding: "utf8", env });
}
function run(label, prepare = true) {
  const cacheInputs = prepare ? prepareCacheInputs(input) : null;
  const clock = performance.now();
  const args = ["test", "--locked", "--offline", "-v"];
  const result = spawnSync("cargo", args, { cwd: repository, env, encoding: "utf8", maxBuffer: 4 * 1024 * 1024 });
  const lines = (result.stderr || "").split(/\r?\n/).filter((line) => /^\s*(Dirty|Fresh|Compiling|Checking|Finished)\b/.test(line));
  const record = { label, seconds: (performance.now() - clock) / 1000, exitCode: result.status,
    command: ["cargo", ...args], cacheInputs, cargo: lines,
    tests: (result.stdout || "").split(/\r?\n/).filter((line) => /^test result:/.test(line)) };
  records.push(record);
  assert.equal(result.status, 0, label + "\n" + result.stdout + "\n" + result.stderr);
  if (prepare) {
    record.seal = sealCacheInputs(input);
    assert.equal(record.seal.status, "sealed", JSON.stringify(record.seal));
  }
  console.log(JSON.stringify(record));
  return record;
}
const dirty = (record) => record.cargo.some((line) => /^\s*Dirty\b/.test(line));
const fresh = (record) => record.cargo.some((line) => /^\s*Fresh\b/.test(line))
  && !record.cargo.some((line) => /^\s*(Compiling|Checking)\b/.test(line));

try {
  mkdirSync(join(repository, "src"), { recursive: true });
  writeFileSync(join(repository, "Cargo.toml"),
    '[package]\nname = "akra-cache-inputs-proof"\nversion = "0.1.0"\nedition = "2024"\n');
  writeFileSync(join(repository, "build.rs"), [
    "fn main() {",
    '    println!("cargo::rerun-if-env-changed=AKRA_CACHE_FIXTURE_VALUE");',
    '    let value = std::env::var("AKRA_CACHE_FIXTURE_VALUE").unwrap();',
    '    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("generated.rs");',
    '    std::fs::write(output, format!("pub const BUILD_VALUE: &str = {:?};", value)).unwrap();',
    "}", "",
  ].join("\n"));
  const source = join(repository, "src/lib.rs");
  const payload = join(repository, "src/payload.txt");
  writeFileSync(payload, "first");
  writeFileSync(source, [
    'include!(concat!(env!("OUT_DIR"), "/generated.rs"));',
    'pub const PAYLOAD: &str = include_str!("payload.txt");',
    "pub fn answer() -> u32 { 42 }",
    "#[test]",
    "fn built_values_follow_inputs() {",
    '    assert_eq!(answer(), std::env::var("EXPECTED_ANSWER").unwrap().parse::<u32>().unwrap());',
    '    assert_eq!(PAYLOAD, std::env::var("EXPECTED_PAYLOAD").unwrap());',
    '    assert_eq!(BUILD_VALUE, std::env::var("EXPECTED_BUILD_VALUE").unwrap());',
    "}", "",
  ].join("\n"));
  git(["init", "--quiet"]);
  execFileSync("cargo", ["generate-lockfile", "--offline"], { cwd: repository, env, encoding: "utf8" });
  git(["add", "--", "Cargo.toml", "Cargo.lock", "build.rs", "src"]);
  run("cold");
  assert.ok(fresh(run("unchanged")));
  await new Promise((resolve) => setTimeout(resolve, 30));
  writeFileSync(source, readFileSync(source));
  assert.ok(dirty(run("same-bytes-new-checkout-mtime-without-restore", false)));
  await new Promise((resolve) => setTimeout(resolve, 30));
  writeFileSync(source, readFileSync(source));
  assert.ok(fresh(run("same-bytes-new-checkout-mtime-with-restore")));
  const archive = "target-cache.tar";
  execFileSync("tar", ["--format=pax", "-cf", archive, "target"], { cwd: root, env });
  rmSync(targetDirectory, { recursive: true, force: true });
  execFileSync("tar", ["-xf", archive], { cwd: root, env });
  for (const path of [source, payload, join(repository, "Cargo.toml"), join(repository, "Cargo.lock"), join(repository, "build.rs")]) {
    writeFileSync(path, readFileSync(path));
  }
  assert.ok(fresh(run("restored-target-archive-identical-checkout")));
  await new Promise((resolve) => setTimeout(resolve, 30));
  writeFileSync(source, readFileSync(source, "utf8").replace("-> u32 { 42 }", "-> u32 { 43 }"));
  env.EXPECTED_ANSWER = "43";
  assert.ok(dirty(run("changed-rust-content")));
  await new Promise((resolve) => setTimeout(resolve, 30));
  writeFileSync(payload, "other");
  env.EXPECTED_PAYLOAD = "other";
  assert.ok(dirty(run("changed-include-str-input")));
  env.AKRA_CACHE_FIXTURE_VALUE = "next";
  env.EXPECTED_BUILD_VALUE = "next";
  assert.ok(dirty(run("changed-build-script-environment")));
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, JSON.stringify({ schemaVersion: 1, recordedAt: new Date().toISOString(),
    toolchain, os: process.platform, architecture: process.arch, command: process.argv.slice(1),
    scope: "isolated-cache-correctness-fixture; not an Akra performance measurement", records }, null, 2) + "\n");
  console.log("Cargo cache correctness proof: " + output);
} finally { rmSync(root, { recursive: true, force: true }); }
