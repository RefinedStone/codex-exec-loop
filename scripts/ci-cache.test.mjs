import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { cacheObservation, createCachePolicy, directorySize, maySaveCache } from "./ci-cache.mjs";

const input = {
  os: "Linux", architecture: "X64", lane: "rust-tests", features: "default", profile: "dev/test",
  target: "implicit-x86_64-unknown-linux-gnu", compiler: "rustc 1.95.0 fixture",
  manifestHash: "fixture-manifest", lockHash: "fixture-lock", revision: "a".repeat(40),
  targetDirectory: "target", cargoHome: join(tmpdir(), "akra-cache-fixture-home"),
  eventName: "push", ref: "refs/heads/prerelease", buildEnvironment: { CARGO_INCREMENTAL: "0" },
};

test("source snapshots refresh only inside one complete target cache boundary", () => {
  const original = createCachePolicy(input);
  const next = createCachePolicy({ ...input, revision: "b".repeat(40) });
  assert.notEqual(original.target.key, next.target.key);
  assert.equal(original.target.restorePrefix, next.target.restorePrefix);
  assert.ok(next.target.key.startsWith(original.target.restorePrefix));
  for (const [field, value] of Object.entries({
    os: "macOS", architecture: "ARM64", lane: "rust-lint", features: "all", profile: "debugging",
    target: "explicit-x86_64-unknown-linux-gnu", compiler: "rustc other",
    manifestHash: "changed-manifest", lockHash: "changed-lock", targetDirectory: "another-target", buildEnvironment: { RUSTFLAGS: "-C opt-level=1" },
  })) {
    const changed = createCachePolicy({ ...input, [field]: value });
    assert.notEqual(changed.target.restorePrefix, original.target.restorePrefix, field);
    assert.equal(changed.target.key.startsWith(original.target.restorePrefix), false, field);
  }
  const lint = createCachePolicy({ ...input, lane: "rust-lint", features: "all" });
  assert.equal(lint.registry.key, original.registry.key);
  const dependencyChange = createCachePolicy({ ...input, lockHash: "new-lock" });
  assert.notEqual(dependencyChange.registry.key, original.registry.key);
  assert.equal(dependencyChange.registry.restorePrefix, original.registry.restorePrefix);
  assert.throws(() => createCachePolicy({ ...input, revision: "short" }));
  assert.equal(JSON.stringify(original).includes("-C opt-level"), false);
});

test("PRs, manual runs and arbitrary branch pushes never write target snapshots", () => {
  for (const ref of ["refs/heads/main", "refs/heads/prerelease"]) assert.equal(maySaveCache("push", ref), true);
  for (const event of ["pull_request", "pull_request_target", "workflow_dispatch"]) {
    assert.equal(maySaveCache(event, "refs/heads/prerelease"), false);
  }
  assert.equal(maySaveCache("push", "refs/heads/codex/feature"), false);
  assert.equal(createCachePolicy({ ...input, eventName: "pull_request" }).saveAllowed, false);
});

test("local byte metrics count files without following junctions or deleting active caches", (t) => {
  const root = mkdtempSync(join(tmpdir(), "akra-cache-size-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, "target"));
  mkdirSync(join(root, "other"));
  writeFileSync(join(root, "target", "a.bin"), Buffer.alloc(7));
  writeFileSync(join(root, "other", "outside.bin"), Buffer.alloc(100));
  symlinkSync(join(root, "other"), join(root, "target", "external"), process.platform === "win32" ? "junction" : "dir");
  const size = directorySize([join(root, "target"), join(root, "missing")]);
  assert.equal(size.bytes, 7);
  assert.equal(size.files, 1);
  assert.equal(size.skippedLinks, 1);
  assert.equal(size.missingPaths, 1);
  assert.deepEqual(size.errors, []);
  assert.equal(readFileSync(join(root, "target", "a.bin")).length, 7);
});

test("restore observations distinguish exact, fallback, missing and unavailable caches", (t) => {
  const root = mkdtempSync(join(tmpdir(), "akra-cache-observation-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const policy = createCachePolicy({ ...input, targetDirectory: join(root, "target"), cargoHome: join(root, "cargo") });
  const exact = cacheObservation(policy, "restored", {
    AKRA_CI_CACHE_TARGET_OUTCOME: "success", AKRA_CI_CACHE_TARGET_MATCHED: policy.target.key, AKRA_CI_CACHE_TARGET_HIT: "true",
    AKRA_CI_CACHE_REGISTRY_OUTCOME: "success",
  });
  assert.equal(exact.caches.target.hit, "exact");
  assert.equal(exact.caches.target.primaryKey, policy.target.key);
  assert.equal(exact.caches.registry.hit, "miss");
  assert.equal(exact.caches.target.localSize.bytes, 0);
  const fallback = cacheObservation(policy, "complete", {
    AKRA_CI_CACHE_TARGET_OUTCOME: "success", AKRA_CI_CACHE_TARGET_MATCHED: policy.target.restorePrefix + "b".repeat(40),
    AKRA_CI_CACHE_TARGET_HIT: "false", AKRA_CI_CACHE_TARGET_SAVE: "skipped", AKRA_CI_CACHE_REGISTRY_OUTCOME: "failure",
  });
  assert.equal(fallback.caches.target.hit, "fallback");
  assert.equal(fallback.caches.target.exactHitOutput, false);
  assert.equal(fallback.caches.target.saveOutcome, "skipped");
  assert.equal(fallback.caches.registry.hit, "unavailable");
  assert.throws(() => cacheObservation(policy, "invalid"));
});
