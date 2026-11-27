import assert from "node:assert/strict";
import {
  existsSync, linkSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync,
  statSync, symlinkSync, utimesSync, writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { prepareCacheInputs, sealCacheInputs, trackedRegularPaths } from "./ci-cache-inputs.mjs";

const revision = "a".repeat(40);
const oldTime = 1_700_000_000_000;
const boundary = { compiler: "rustc fixture", target: "host", features: "default", profile: "test" };
const mtime = (file) => Math.floor(statSync(file).mtimeMs);
function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "akra-cache-inputs-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const repository = join(root, "repo"), targetDirectory = join(root, "target");
  mkdirSync(join(repository, "src"), { recursive: true });
  const source = join(repository, "src/lib.rs");
  writeFileSync(source, "pub fn answer() -> u32 { 42 }\n");
  utimesSync(source, oldTime / 1000, oldTime / 1000);
  const input = { repository, targetDirectory, boundary, sourceRevision: revision, paths: ["src/lib.rs"] };
  return { root, repository, targetDirectory, source, input, snapshot: join(targetDirectory, ".akra-ci-source-inputs-v1.json") };
}

function seed(input) {
  prepareCacheInputs(input);
  assert.equal(sealCacheInputs(input).status, "sealed");
}

test("cache miss snapshots pre-build input without changing source or requiring target artifacts", (t) => {
  const f = fixture(t);
  const before = statSync(f.source);
  const report = prepareCacheInputs(f.input);
  assert.equal(report.status, "missing_snapshot");
  assert.equal(report.restoredFiles, 0);
  assert.equal(report.newFiles, 1);
  assert.equal(mtime(f.source), before.mtimeMs);
  const saved = JSON.parse(readFileSync(f.snapshot, "utf8"));
  assert.equal(saved.inputs[0].mtimeMs, oldTime);
  assert.equal(saved.inputs[0].size, readFileSync(f.source).length);
  assert.deepEqual(report.skipped, []);
});

test("matching bytes reuse source timestamps across source SHAs without changing output bytes", (t) => {
  const f = fixture(t);
  seed(f.input);
  const output = join(f.targetDirectory, "compiled.rlib");
  writeFileSync(output, "compiled-contents");
  const outputBefore = statSync(output).mtimeMs;
  const original = readFileSync(f.source);
  writeFileSync(f.source, original);
  assert.ok(mtime(f.source) > oldTime);
  const report = prepareCacheInputs({ ...f.input, sourceRevision: "b".repeat(40) });
  assert.equal(report.status, "matched_snapshot");
  assert.equal(report.previousRevision, revision);
  assert.equal(report.restoredFiles, 1);
  assert.equal(mtime(f.source), oldTime);
  assert.deepEqual(readFileSync(f.source), original);
  assert.equal(readFileSync(output, "utf8"), "compiled-contents");
  assert.equal(statSync(output).mtimeMs, outputBefore);
  assert.equal(JSON.parse(readFileSync(f.snapshot, "utf8")).sourceRevision, "b".repeat(40));
});

test("changed input and new input retain fresh checkout mtimes", (t) => {
  const f = fixture(t);
  seed(f.input);
  writeFileSync(f.source, "pub fn answer() -> u32 { 43 }\n"); // Equal length still changes the digest.
  const added = join(f.repository, "src/added.rs");
  writeFileSync(added, "pub fn new_api() {}\n");
  const modifiedTime = mtime(f.source), addedTime = mtime(added);
  const report = prepareCacheInputs({ ...f.input, paths: ["src/lib.rs", "src/added.rs"] });
  assert.equal(report.changedFiles, 1);
  assert.equal(report.newFiles, 1);
  assert.equal(report.restoredFiles, 0);
  assert.equal(mtime(f.source), modifiedTime);
  assert.equal(mtime(added), addedTime);
});

test("removed and renamed paths are not recreated or matched by contents alone", (t) => {
  const f = fixture(t);
  seed(f.input);
  const renamed = join(f.repository, "src/moved.rs");
  writeFileSync(renamed, readFileSync(f.source));
  rmSync(f.source);
  const time = mtime(renamed);
  const report = prepareCacheInputs({ ...f.input, paths: ["src/moved.rs"] });
  assert.equal(report.newFiles, 1);
  assert.equal(report.restoredFiles, 0);
  assert.equal(existsSync(f.source), false);
  assert.equal(mtime(renamed), time);
});

test("source path, feature, profile and compiler boundaries fail closed", (t) => {
  const f = fixture(t);
  for (const changed of [
    { compiler: "rustc other" }, { features: "all" }, { profile: "debugging" }, { target: "explicit-other" },
  ]) {
    seed(f.input);
    writeFileSync(f.source, readFileSync(f.source));
    const time = mtime(f.source);
    const report = prepareCacheInputs({ ...f.input, boundary: { ...boundary, ...changed } });
    assert.equal(report.status, "build_boundary_changed");
    assert.equal(report.restoredFiles, 0);
    assert.equal(mtime(f.source), time);
  }
  seed(f.input);
  const otherRepo = join(f.root, "other-repo");
  mkdirSync(join(otherRepo, "src"), { recursive: true });
  const otherSource = join(otherRepo, "src/lib.rs");
  writeFileSync(otherSource, readFileSync(f.source));
  const time = mtime(otherSource);
  const report = prepareCacheInputs({ ...f.input, repository: otherRepo });
  assert.equal(report.status, "repository_path_changed");
  assert.equal(mtime(otherSource), time);
});

test("corrupt, duplicate and future snapshot metadata never restore source mtimes", (t) => {
  const f = fixture(t);
  const cases = [
    () => "not-json",
    (snapshot) => JSON.stringify({ ...snapshot, schemaVersion: 99 }),
    (snapshot) => JSON.stringify({ ...snapshot, capturedAt: new Date(Date.now() + 60_000).toISOString() }),
    (snapshot) => JSON.stringify({ ...snapshot, inputs: [...snapshot.inputs, snapshot.inputs[0]] }),
    (snapshot) => JSON.stringify({ ...snapshot, inputs: [{ ...snapshot.inputs[0], mtimeMs: -1 }] }),
  ];
  for (const corrupt of cases) {
    seed(f.input);
    const snapshot = JSON.parse(readFileSync(f.snapshot, "utf8"));
    writeFileSync(f.snapshot, corrupt(snapshot));
    writeFileSync(f.source, readFileSync(f.source));
    const time = mtime(f.source);
    const report = prepareCacheInputs(f.input);
    assert.equal(report.status, "invalid_snapshot");
    assert.equal(report.restoredFiles, 0);
    assert.equal(mtime(f.source), time);
  }
});

test("snapshot entries outside the current tracked inventory are inert", (t) => {
  const f = fixture(t);
  seed(f.input);
  const external = join(f.root, "outside.rs");
  writeFileSync(external, readFileSync(f.source));
  const snapshot = JSON.parse(readFileSync(f.snapshot, "utf8"));
  snapshot.inputs.push({ ...snapshot.inputs[0], path: "../outside.rs" });
  writeFileSync(f.snapshot, JSON.stringify(snapshot));
  const time = mtime(external);
  const report = prepareCacheInputs(f.input);
  assert.equal(report.restoredFiles, 0);
  assert.equal(mtime(external), time);
});

test("symlinks, junction ancestors and hardlinks are never backdated", (t) => {
  const f = fixture(t);
  seed(f.input);
  const outside = join(f.root, "outside");
  mkdirSync(outside);
  const external = join(outside, "lib.rs");
  writeFileSync(external, readFileSync(f.source));
  const time = mtime(external);
  rmSync(join(f.repository, "src"), { recursive: true });
  symlinkSync(outside, join(f.repository, "src"), process.platform === "win32" ? "junction" : "dir");
  let report = prepareCacheInputs(f.input);
  assert.equal(report.restoredFiles, 0);
  assert.equal(report.skipped.length, 1);
  assert.equal(mtime(external), time);
  rmSync(join(f.repository, "src"), { recursive: true });
  mkdirSync(join(f.repository, "src"));
  linkSync(external, f.source);
  report = prepareCacheInputs(f.input);
  assert.equal(report.restoredFiles, 0);
  assert.equal(report.skipped[0].reason, "non_regular_input");
  assert.equal(mtime(external), time);
});

test("linked target directories cannot redirect snapshot writes", (t) => {
  const f = fixture(t);
  const outside = join(f.root, "outside");
  mkdirSync(outside);
  symlinkSync(outside, f.targetDirectory, process.platform === "win32" ? "junction" : "dir");
  assert.throws(() => prepareCacheInputs(f.input), /linked_path/);
  assert.equal(existsSync(join(outside, ".akra-ci-source-inputs-v1.json")), false);
});

test("tracked inventory ignores untracked files and handles paths with spaces", (t) => {
  const f = fixture(t);
  const git = (args) => {
    const result = spawnSync("git", ["-C", f.repository, ...args], {
      encoding: "utf8",
      env: Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^GIT_/i.test(key))),
    });
    assert.equal(result.status, 0, result.stderr);
  };
  git(["init", "--quiet"]);
  writeFileSync(join(f.repository, "src/with space.rs"), "pub fn spaced() {}\n");
  writeFileSync(join(f.repository, "src/untracked.rs"), "untracked");
  git(["add", "--", "src/lib.rs", "src/with space.rs"]);
  assert.deepEqual(trackedRegularPaths(f.repository), ["src/lib.rs", "src/with space.rs"]);
  const { paths, ...input } = f.input;
  const report = prepareCacheInputs(input);
  assert.equal(report.trackedFiles, 2);
  assert.equal(report.skipped.length, 0);
});


test("pending input hints are never reused before successful validation seals them", (t) => {
  const f = fixture(t);
  prepareCacheInputs(f.input);
  writeFileSync(f.source, readFileSync(f.source));
  const time = mtime(f.source);
  const report = prepareCacheInputs(f.input);
  assert.equal(report.status, "unsealed_snapshot");
  assert.equal(report.restoredFiles, 0);
  assert.equal(mtime(f.source), time);
});

test("input changes during validation invalidate reuse hints even after original bytes and mtime return", async (t) => {
  const f = fixture(t);
  prepareCacheInputs(f.input);
  const original = readFileSync(f.source);
  await new Promise((resolve) => setTimeout(resolve, 20));
  writeFileSync(f.source, "temporary validation edit");
  writeFileSync(f.source, original);
  utimesSync(f.source, oldTime / 1000, oldTime / 1000);
  const sealed = sealCacheInputs(f.input);
  assert.equal(sealed.status, "inputs_changed_during_validation");
  assert.deepEqual(sealed.changed, ["src/lib.rs"]);
  assert.equal(JSON.parse(readFileSync(f.snapshot, "utf8")).state, "invalidated");
  writeFileSync(f.source, original);
  const time = mtime(f.source);
  assert.equal(prepareCacheInputs(f.input).status, "unsealed_snapshot");
  assert.equal(mtime(f.source), time);
});

test("a different source revision cannot seal a stale pending snapshot", (t) => {
  const f = fixture(t);
  prepareCacheInputs(f.input);
  assert.equal(sealCacheInputs({ ...f.input, sourceRevision: "b".repeat(40) }).status, "source_revision_changed");
  assert.equal(JSON.parse(readFileSync(f.snapshot, "utf8")).state, "pending");
});
