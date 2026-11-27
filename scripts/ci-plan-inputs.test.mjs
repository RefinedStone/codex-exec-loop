import assert from "node:assert/strict";
import {
  existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, symlinkSync,
  utimesSync, writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import test from "node:test";
import { createValidationPlan } from "./ci-impact-plan.mjs";
import { assertCleanPlanCheckout, capturePlanInputs, collectPlanInput } from "./ci-plan-inputs.mjs";
import { createPlanFixture, fixtureGit, fixtureMetadata, fixtureToolchain, writeFixtureFile } from "./fixtures/ci-plan-fixture.mjs";

function collect(fixture, options = {}, services = {}) {
  return collectPlanInput({ cwd: fixture.cwd, baseRef: "prerelease", ...options }, { ...fixture.services, ...services });
}
function capture(fixture, environment = fixture.environment) {
  return capturePlanInputs({ cwd: fixture.cwd, environment, toolchain: fixtureToolchain });
}
function installOutputFixture() {
  const fixture = createPlanFixture({ externalInputs: ["assets/"] });
  const registry = JSON.parse(readFileSync(new URL("./ci-crate-ownership.json", import.meta.url), "utf8"));
  const output = registry.validationOutputs.find((entry) => entry.path === "assets/admin/game/node_modules/");
  assert.ok(output, "the actual broad assets/ ownership must declare the npm ci output");
  fixture.ownership.validationOutputs = [structuredClone(output)];
  writeFixtureFile(fixture.cwd, "scripts/ci-crate-ownership.json", JSON.stringify(fixture.ownership));
  writeFixtureFile(fixture.cwd, output.manifest, '{"name":"fixture","version":"1.0.0"}\n');
  writeFixtureFile(fixture.cwd, output.lockfile, '{"name":"fixture","lockfileVersion":3,"packages":{}}\n');
  writeFixtureFile(fixture.cwd, ".gitignore", readFileSync(join(fixture.cwd, ".gitignore"), "utf8") + output.path + "\n");
  fixtureGit(fixture.cwd, ["add", "."]);
  fixtureGit(fixture.cwd, ["commit", "-qm", "registered npm install output"]);
  fixture.baseSha = fixtureGit(fixture.cwd, ["rev-parse", "HEAD"]).trim();
  return fixture;
}

test("collects commit, staged, unstaged, untracked, deletion and both rename paths", () => {
  const fixture = createPlanFixture();
  try {
    writeFixtureFile(fixture.cwd, "crates/leaf/src/committed.rs", "// committed\n");
    fixtureGit(fixture.cwd, ["add", "crates/leaf/src/committed.rs"]);
    fixtureGit(fixture.cwd, ["commit", "-qm", "committed edit"]);
    const lib = "crates/leaf/src/lib.rs";
    writeFixtureFile(fixture.cwd, lib, readFileSync(join(fixture.cwd, lib), "utf8") + "// staged\n");
    fixtureGit(fixture.cwd, ["add", lib]);
    writeFixtureFile(fixture.cwd, lib, readFileSync(join(fixture.cwd, lib), "utf8") + "// unstaged\n");
    writeFixtureFile(fixture.cwd, "crates/leaf/src/new $(literal) file.rs", "// untracked\n");
    fixtureGit(fixture.cwd, ["mv", "crates/leaf/src/rename me.rs", "crates/other/src/renamed file.rs"]);
    rmSync(join(fixture.cwd, "crates/leaf/src/deleted.rs"));
    const request = collect(fixture);
    const paths = new Set(request.changes.map((change) => change.path));
    assert.ok(paths.has("crates/leaf/src/committed.rs"));
    assert.ok(paths.has(lib));
    assert.ok(paths.has("crates/leaf/src/new $(literal) file.rs"));
    assert.ok(request.changes.some((change) => change.status === "D" && change.path.endsWith("deleted.rs")));
    assert.ok(request.changes.some((change) => change.status === "R" && change.oldPath === "crates/leaf/src/rename me.rs" && change.path === "crates/other/src/renamed file.rs"));
    assert.equal(request.base.sha, fixture.baseSha);
    assert.equal(request.head.sha, fixtureGit(fixture.cwd, ["rev-parse", "HEAD"]).trim());
    assert.deepEqual(createValidationPlan(request).testPackages.map((pkg) => pkg.name).sort(), ["leaf", "other"]);
  } finally { fixture.cleanup(); }
});

test("reads an immutable base archive and leaves the source/index/lock/target unchanged", () => {
  const fixture = createPlanFixture();
  try {
    const source = readFileSync(join(fixture.cwd, "crates/leaf/src/lib.rs"));
    const lock = readFileSync(join(fixture.cwd, "Cargo.lock"));
    const index = readFileSync(join(fixture.cwd, ".git/index"));
    const status = fixtureGit(fixture.cwd, ["status", "--porcelain"]);
    fixture.environment.CARGO_TARGET_DIR = join(fixture.cwd, "target/preserved");
    fixture.environment.CARGO_INCREMENTAL = "1";
    writeFixtureFile(fixture.cwd, "target/preserved/sentinel", "keep target\n");
    const request = collect(fixture);
    assert.ok(request.base.metadata.resolve);
    assert.notEqual(resolve(request.base.metadata.workspace_root), resolve(fixture.cwd));
    assert.equal(existsSync(request.base.metadata.workspace_root), false, "temporary archive must be removed");
    assert.deepEqual(readFileSync(join(fixture.cwd, ".git/index")), index);
    assert.deepEqual(readFileSync(join(fixture.cwd, "Cargo.lock")), lock);
    assert.deepEqual(readFileSync(join(fixture.cwd, "crates/leaf/src/lib.rs")), source);
    assert.equal(fixtureGit(fixture.cwd, ["status", "--porcelain"]), status);
    assert.equal(readFileSync(join(fixture.cwd, "target/preserved/sentinel"), "utf8"), "keep target\n");
    assert.equal(fixture.environment.CARGO_INCREMENTAL, "1");
  } finally { fixture.cleanup(); }
});

test("content identity detects same-size writes even if source mtime is restored", () => {
  const fixture = createPlanFixture();
  try {
    const path = join(fixture.cwd, "crates/leaf/src/lib.rs"), beforeStat = statSync(path);
    const before = capture(fixture);
    const original = readFileSync(path, "utf8");
    writeFileSync(path, original.replace("    1\n", "    2\n"));
    utimesSync(path, beforeStat.atime, beforeStat.mtime);
    const after = capture(fixture);
    assert.notEqual(after.filesIdentity, before.filesIdentity);
    assert.notEqual(after.localChangeId, before.localChangeId);
    assert.equal(after.indexIdentity, before.indexIdentity);
    writeFileSync(path, original);
    assert.equal(capture(fixture).localChangeId, before.localChangeId, "mtime alone must not invalidate unchanged contents");
  } finally { fixture.cleanup(); }
});

test("staged blob identity remains distinct when the worktree is restored to original bytes", () => {
  const fixture = createPlanFixture();
  try {
    const path = join(fixture.cwd, "crates/leaf/src/lib.rs"), original = readFileSync(path);
    const before = capture(fixture);
    writeFileSync(path, Buffer.concat([original, Buffer.from("// staged only\n")]));
    fixtureGit(fixture.cwd, ["add", "crates/leaf/src/lib.rs"]);
    writeFileSync(path, original);
    const after = capture(fixture);
    assert.equal(after.filesIdentity, before.filesIdentity);
    assert.notEqual(after.indexIdentity, before.indexIdentity);
    assert.notEqual(after.localChangeId, before.localChangeId);
  } finally { fixture.cleanup(); }
});

test("exact registered ignored inputs participate in both content identity and impact selection", () => {
  const fixture = createPlanFixture({ externalInputs: ["generated/input.txt"] });
  try {
    const before = capture(fixture);
    writeFixtureFile(fixture.cwd, "generated/input.txt", "first\n");
    const request = collect(fixture);
    assert.ok(request.changes.some((change) => change.path === "generated/input.txt"));
    assert.equal(createValidationPlan(request).mode, "full");
    assert.notEqual(request.localChangeId, before.localChangeId);
    writeFixtureFile(fixture.cwd, "generated/input.txt", "other\n");
    assert.notEqual(capture(fixture).localChangeId, request.localChangeId);
  } finally { fixture.cleanup(); }
});

test("Cargo settings and compile environment are hashed without exposing their values", () => {
  const fixture = createPlanFixture();
  try {
    const before = capture(fixture);
    const environment = { ...fixture.environment, RUSTFLAGS: "--cfg private_fixture_flag", CARGO_INCREMENTAL: "1" };
    const configured = capture(fixture, environment);
    assert.notEqual(configured.environmentIdentity, before.environmentIdentity);
    const pathKey = Object.keys(fixture.environment).find((name) => name.toUpperCase() === "PATH") ?? "PATH";
    assert.notEqual(capture(fixture, {
      ...fixture.environment, [pathKey]: fixture.environment[pathKey] + delimiter + "fixture-tool-discovery-path",
    }).environmentIdentity, before.environmentIdentity);
    assert.equal(JSON.stringify(configured).includes("private_fixture_flag"), false);
    writeFixtureFile(fixture.cwd, "cargo-home/config.toml", "[build]\njobs = 2\n");
    assert.notEqual(capture(fixture).environmentIdentity, before.environmentIdentity);
  } finally { fixture.cleanup(); }
});

test("local defaults use offline metadata and CI can opt into locked cache downloads", () => {
  const fixture = createPlanFixture();
  try {
    const modes = [];
    const readMetadata = (cwd, options) => { modes.push(options.offline); return fixtureMetadata(cwd); };
    collect(fixture, {}, { readMetadata });
    assert.deepEqual(modes, [true, true]);
    modes.length = 0;
    collect(fixture, { offline: false }, { readMetadata });
    assert.deepEqual(modes, [false, false]);
  } finally { fixture.cleanup(); }
});

test("missing base metadata or refs produce full while missing head metadata is an error", () => {
  const fixture = createPlanFixture();
  try {
    const baseMissing = collect(fixture, {}, { readMetadata(cwd) {
      if (resolve(cwd) !== resolve(fixture.cwd)) throw new Error("offline cache miss");
      return fixtureMetadata(cwd);
    } });
    assert.equal(baseMissing.base.metadata, null);
    assert.equal(createValidationPlan(baseMissing).mode, "full");
    const refMissing = collect(fixture, { baseRef: "missing-fixture-ref" });
    assert.equal(refMissing.base, null);
    assert.equal(createValidationPlan(refMissing).mode, "full");
    assert.throws(() => collect(fixture, {}, { readMetadata() { throw new Error("head metadata unavailable"); } }), /head metadata unavailable/);
  } finally { fixture.cleanup(); }
});

test("requested source SHA and explicit base commit are pinned", () => {
  const fixture = createPlanFixture();
  try {
    assert.throws(() => collect(fixture, { headSha: "e".repeat(40) }), /differs/);
    const request = collect(fixture, { baseSha: fixture.baseSha, headSha: fixture.baseSha });
    assert.equal(request.base.sha, fixture.baseSha);
    assert.equal(request.head.sha, fixture.baseSha);
  } finally { fixture.cleanup(); }
});

test("changes during metadata collection are rejected without reverting user edits", () => {
  const fixture = createPlanFixture();
  try {
    assert.throws(() => collect(fixture, {}, { readMetadata(cwd) {
      const result = fixtureMetadata(cwd);
      if (resolve(cwd) === resolve(fixture.cwd)) {
        writeFixtureFile(fixture.cwd, "crates/leaf/src/lib.rs", "// concurrently changed\n");
      }
      return result;
    } }), /changed while collecting/);
    assert.equal(readFileSync(join(fixture.cwd, "crates/leaf/src/lib.rs"), "utf8"), "// concurrently changed\n");
  } finally { fixture.cleanup(); }
});

test("a base command that mutates its lock cannot authorize a selective plan", () => {
  const fixture = createPlanFixture();
  try {
    const lock = readFileSync(join(fixture.cwd, "Cargo.lock"));
    const request = collect(fixture, {}, { readMetadata(cwd) {
      const result = fixtureMetadata(cwd);
      if (resolve(cwd) !== resolve(fixture.cwd)) writeFileSync(join(cwd, "Cargo.lock"), "changed\n");
      return result;
    } });
    assert.equal(request.base.metadata, null);
    assert.equal(createValidationPlan(request).mode, "full");
    assert.deepEqual(readFileSync(join(fixture.cwd, "Cargo.lock")), lock);
  } finally { fixture.cleanup(); }
});

test("unreproducible ancestor Cargo configuration keeps the full fallback", () => {
  const parent = mkdtempSync(join(tmpdir(), "akra-cargo-ancestor-"));
  const fixture = createPlanFixture({ parentDirectory: parent });
  try {
    mkdirSync(join(parent, ".cargo"));
    writeFileSync(join(parent, ".cargo/config.toml"), "[build]\njobs = 2\n");
    const request = collect(fixture);
    assert.equal(request.base.metadata, null);
    assert.equal(createValidationPlan(request).mode, "full");
  } finally { fixture.cleanup(); rmSync(parent, { recursive: true, force: true }); }
});

test("symlinks and submodules do not silently escape the input fingerprint", (context) => {
  const fixture = createPlanFixture();
  const destination = mkdtempSync(join(tmpdir(), "akra-input-link-destination-"));
  try {
    writeFileSync(join(destination, "sentinel"), "outside link stays intact\n");
    try {
      symlinkSync(destination, join(fixture.cwd, "linked-input"), process.platform === "win32" ? "junction" : "dir");
    } catch (error) {
      if (["EPERM", "EACCES"].includes(error.code)) { context.skip("host cannot create a test symlink"); return; }
      throw error;
    }
    assert.throws(() => capture(fixture), /ordinary file|ordinary directory/);
    rmSync(join(fixture.cwd, "linked-input"), { force: true, recursive: true });
    fixtureGit(fixture.cwd, ["update-index", "--add", "--cacheinfo", "160000," + fixture.baseSha + ",deps/submodule"]);
    assert.throws(() => capture(fixture), /submodule contents/);
    assert.equal(readFileSync(join(destination, "sentinel"), "utf8"), "outside link stays intact\n");
  } finally { fixture.cleanup(); rmSync(destination, { recursive: true, force: true }); }
});

test("repeated collection is deterministic despite temporary archive locations", () => {
  const fixture = createPlanFixture();
  try {
    writeFixtureFile(fixture.cwd, "crates/leaf/src/lib.rs", "// leaf edit\n");
    const first = createValidationPlan(collect(fixture)), second = createValidationPlan(collect(fixture));
    assert.deepEqual(first, second);
    assert.equal(first.mode, "selective");
  } finally { fixture.cleanup(); }
});

test("immutable CI check rejects dirty source and ignored registered inputs", () => {
  const fixture = createPlanFixture({ externalInputs: ["generated/input.txt"] });
  try {
    assert.equal(assertCleanPlanCheckout({ cwd: fixture.cwd, environment: fixture.environment }).clean, true);
    collect(fixture, { requireCleanCheckout: true });
    writeFixtureFile(fixture.cwd, "generated/input.txt", "not bound to the source commit\n");
    assert.throws(() => assertCleanPlanCheckout({ cwd: fixture.cwd, environment: fixture.environment }), /outside Git/);
    assert.throws(() => collect(fixture, { requireCleanCheckout: true }), /outside Git/);
    rmSync(join(fixture.cwd, "generated/input.txt"));
    writeFixtureFile(fixture.cwd, "crates/leaf/src/new.rs", "// untracked\n");
    assert.throws(() => assertCleanPlanCheckout({ cwd: fixture.cwd, environment: fixture.environment }), /clean tracked and untracked/);
  } finally { fixture.cleanup(); }
});

test("ignored files beneath registered directory prefixes cannot evade local or CI identity", () => {
  const fixture = createPlanFixture({ externalInputs: ["generated/"] });
  try {
    const before = capture(fixture);
    writeFixtureFile(fixture.cwd, "generated/deep/input.txt", "prefix-owned bytes\n");
    const request = collect(fixture);
    assert.ok(request.changes.some((change) => change.path === "generated/deep/input.txt"));
    assert.notEqual(request.localChangeId, before.localChangeId);
    assert.equal(createValidationPlan(request).mode, "full");
    assert.throws(() => assertCleanPlanCheckout({ cwd: fixture.cwd, environment: fixture.environment }), /outside Git/);
    assert.throws(() => collect(fixture, { requireCleanCheckout: true }), /outside Git/);
  } finally { fixture.cleanup(); }
});

test("the actual registry's npm install output does not invalidate an otherwise unchanged full gate", () => {
  const fixture = installOutputFixture();
  try {
    const before = capture(fixture);
    writeFixtureFile(fixture.cwd, "assets/admin/game/node_modules/installed/index.js", "// produced by npm ci\n");
    assert.equal(capture(fixture).localChangeId, before.localChangeId);
    assert.equal(assertCleanPlanCheckout({ cwd: fixture.cwd, environment: fixture.environment }).clean, true);
    const request = collect(fixture, { requireCleanCheckout: true });
    assert.equal(request.changes.some((entry) => entry.path.includes("node_modules")), false);
    writeFixtureFile(fixture.cwd, "assets/admin/game/source.js", "// real source remains an input\n");
    assert.notEqual(capture(fixture).localChangeId, before.localChangeId);
    assert.throws(() => assertCleanPlanCheckout({ cwd: fixture.cwd, environment: fixture.environment }), /clean tracked and untracked/);
  } finally { fixture.cleanup(); }
});

test("install output policy cannot hide registered sources, contracts or arbitrary directories", () => {
  const fixture = installOutputFixture();
  try {
    const original = structuredClone(fixture.ownership);
    for (const mutate of [
      (ownership) => ownership.packages[0].sourcePrefixes.push("assets/"),
      (ownership) => ownership.packages[0].externalInputs.push("assets/admin/game/node_modules/needed.json"),
      (ownership) => { ownership.packages[0].generatedInputs = ["assets/admin/game/node_modules/"]; },
      (ownership) => ownership.contracts.push({ inputs: ["assets/"] }),
      (ownership) => { ownership.validationOutputs[0].path = "src/"; },
      (ownership) => { ownership.validationOutputs[0].lockfile = "untracked-lock.json"; },
    ]) {
      const ownership = structuredClone(original);
      mutate(ownership);
      writeFixtureFile(fixture.cwd, "scripts/ci-crate-ownership.json", JSON.stringify(ownership));
      assert.throws(() => capture(fixture), /validation output/);
    }
  } finally { fixture.cleanup(); }
});

test("tracked files beneath an install output always remain content inputs", () => {
  const fixture = installOutputFixture();
  try {
    const path = "assets/admin/game/node_modules/tracked-policy.json";
    writeFixtureFile(fixture.cwd, path, '{"first":true}\n');
    fixtureGit(fixture.cwd, ["add", "--force", path]);
    fixtureGit(fixture.cwd, ["commit", "-qm", "tracked output remains source"]);
    const before = capture(fixture);
    writeFixtureFile(fixture.cwd, path, '{"first":false}\n');
    const after = capture(fixture);
    assert.notEqual(after.filesIdentity, before.filesIdentity);
    assert.notEqual(after.localChangeId, before.localChangeId);
  } finally { fixture.cleanup(); }
});
