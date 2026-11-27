import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { createValidationPlan, explainPlan, parseGitNameStatusZ, verifyValidationPlanHash } from "./ci-impact-plan.mjs";

const BASE_SHA = "a".repeat(40);
const HEAD_SHA = "b".repeat(40);
const clone = (value) => structuredClone(value);
const key = (name) => name === "root" ? "Cargo.toml::root" : "crates/" + name + "/Cargo.toml::" + name;

function makeSnapshot(definitions = [
  { name: "root", manifest: "Cargo.toml" }, { name: "leaf" }, { name: "other" },
], { root = "/fixture/repository", sha = HEAD_SHA, contracts = [] } = {}) {
  const location = (relative) => root + "/" + relative;
  const ids = new Map(definitions.map((definition, index) => [definition.name, "opaque id with punctuation [" + index + "] " + definition.name]));
  const members = definitions.map((definition) => {
    const manifest = definition.manifest ?? "crates/" + definition.name + "/Cargo.toml";
    const directory = manifest === "Cargo.toml" ? "" : manifest.slice(0, -"Cargo.toml".length);
    const dependencyDeclarations = (definition.dependencies ?? []).map((dependency) => {
      const dep = typeof dependency === "string" ? { name: dependency } : dependency;
      const target = definitions.find((candidate) => candidate.name === dep.name);
      const targetManifest = target.manifest ?? "crates/" + target.name + "/Cargo.toml";
      return {
        name: dep.name, path: location(targetManifest === "Cargo.toml" ? "" : targetManifest.slice(0, -"/Cargo.toml".length)),
        source: null, kind: dep.kind ?? null, target: dep.target ?? null, optional: dep.optional ?? false,
        rename: dep.rename ?? null, features: dep.features ?? [], uses_default_features: true,
      };
    });
    const rawTargets = definition.targets ?? [{ name: definition.name.replaceAll("-", "_"), kind: ["lib"], test: true, doctest: true }];
    return {
      id: ids.get(definition.name), name: definition.name, version: "1.0.0", source: null,
      manifest_path: location(manifest), dependencies: dependencyDeclarations,
      features: definition.features ?? {},
      targets: rawTargets.map((target) => ({ src_path: location(directory + "src/lib.rs"), "required-features": [], ...target })),
    };
  });
  const metadata = {
    version: 1, workspace_root: root, packages: members, workspace_members: members.map((pkg) => pkg.id),
    resolve: { root: members[0].id, nodes: members.map((pkg) => ({
      id: pkg.id, features: Object.keys(pkg.features),
      deps: pkg.dependencies.map((dependency) => ({
        name: dependency.rename ?? dependency.name, pkg: ids.get(dependency.name),
        dep_kinds: [{ kind: dependency.kind, target: dependency.target }],
      })),
    })) },
  };
  const ownership = {
    schemaVersion: 1, revision: 1, rootPackage: "root", fullInputs: ["Cargo.toml", "Cargo.lock", "scripts/"], contracts,
    packages: definitions.map((definition) => {
      const manifest = definition.manifest ?? "crates/" + definition.name + "/Cargo.toml";
      return {
        name: definition.name, manifest, state: "active", consumerCoverage: "full",
        workspaceDependencies: (definition.dependencies ?? []).map((dependency) => typeof dependency === "string" ? dependency : dependency.name),
        sourcePrefixes: definition.inputs ?? [manifest === "Cargo.toml" ? "src/" : manifest.slice(0, -"Cargo.toml".length)],
        externalInputs: definition.externalInputs ?? [],
      };
    }),
  };
  return { sha, metadata, ownership };
}
function request({ base, head, changes = [{ status: "M", path: "crates/leaf/src/lib.rs" }], configuration, localChangeId } = {}) {
  const current = head ?? makeSnapshot();
  return {
    schemaVersion: 1, base: base ?? { ...clone(current), sha: BASE_SHA }, head: current,
    changes, configuration, localChangeId,
  };
}
function packages(plan, field = "testPackages") {
  return plan[field].map((pkg) => pkg.name).sort();
}
function assertFull(plan, reason) {
  assert.equal(plan.mode, "full");
  assert.ok(plan.fullReasons.some((value) => value.includes(reason)), JSON.stringify(plan.fullReasons));
  assert.ok(plan.testPackages.length > 0);
  assert.ok(plan.compilePackages.length > 0);
  assert.equal(plan.nodeRequired, true);
}
function contract(overrides = {}) {
  return {
    id: "plugin-contract", owner: "consumer", target: { name: "consumer", kind: "lib" },
    filter: "plugin::contract::", platforms: ["linux", "macos", "windows"], features: "all",
    isolation: "process-per-filter", ...overrides,
  };
}

test("an independent leaf selects its complete targets and excludes unrelated packages", () => {
  const plan = createValidationPlan(request());
  assert.equal(plan.mode, "selective");
  assert.deepEqual(packages(plan), ["leaf"]);
  assert.deepEqual(packages(plan, "compilePackages"), ["leaf"]);
  assert.equal(plan.testPackages[0].coverage, "full");
  assert.equal(plan.testPackages[0].packageKey, key("leaf"));
  assert.equal(plan.testPackages[0].targets[0].doctest, true);
  assert.deepEqual(plan.excludedPackages.map((pkg) => pkg.name).sort(), ["other", "root"]);
});

for (const kind of [null, "dev", "build"]) {
  test("retains " + (kind ?? "normal") + " consumers in the base and head graph", () => {
    const head = makeSnapshot([{ name: "root", manifest: "Cargo.toml", dependencies: [{ name: "leaf", kind }] }, { name: "leaf" }]);
    const plan = createValidationPlan(request({ head }));
    assertFull(plan, "root-consumer-requires-full");
    assert.deepEqual(packages(plan), ["leaf", "root"]);
  });
}

test("optional renamed Windows/macOS edges remain conservative for every requested platform", () => {
  for (const condition of ["cfg(windows)", 'cfg(target_os = "macos")']) {
    const head = makeSnapshot([
      { name: "root", manifest: "Cargo.toml", features: { default: [], native: ["dep:renamed"] },
        dependencies: [{ name: "leaf", kind: "build", target: condition, optional: true, rename: "renamed" }] },
      { name: "leaf" },
    ]);
    const plan = createValidationPlan(request({ head, configuration: { platforms: ["linux"], features: ["default"] } }));
    assertFull(plan, "root-consumer-requires-full");
    assert.equal(plan.configuration.platforms[0], "linux");
  }
});

test("no-deps and omitted optional feature resolution fall back to full", () => {
  const missing = request();
  missing.base.metadata.resolve = null;
  assertFull(createValidationPlan(missing), "metadata-resolve-missing");
  const optional = request();
  optional.head.metadata.packages[1].features = { optional_path: ["dep:optional"] };
  assertFull(createValidationPlan(optional), "all-features-graph-required");
});

test("a removed dependency edge still propagates through the base graph", () => {
  const base = makeSnapshot([{ name: "root", manifest: "Cargo.toml", dependencies: ["leaf"] }, { name: "leaf" }], { sha: BASE_SHA });
  const head = makeSnapshot([{ name: "root", manifest: "Cargo.toml" }, { name: "leaf" }]);
  assertFull(createValidationPlan(request({ base, head })), "root-consumer-requires-full");
});

test("deleted packages retain their former consumers and explain removed units", () => {
  const base = makeSnapshot([{ name: "root", manifest: "Cargo.toml", dependencies: ["leaf"] }, { name: "leaf" }], { sha: BASE_SHA });
  const head = makeSnapshot([{ name: "root", manifest: "Cargo.toml" }]);
  const plan = createValidationPlan(request({ base, head, changes: [{ status: "D", path: "crates/leaf/src/lib.rs" }] }));
  assertFull(plan, "root-consumer-requires-full");
  assert.deepEqual(packages(plan), ["root"]);
  assert.deepEqual(plan.removedPackages.map((pkg) => pkg.name), ["leaf"]);
});

test("rename and copy include both owners, without depending on rename score", () => {
  for (const status of ["R100", "C75"]) {
    const plan = createValidationPlan(request({ changes: [{ status, oldPath: "crates/leaf/src/old.rs", path: "crates/other/src/new.rs" }] }));
    assert.equal(plan.mode, "selective");
    assert.deepEqual(packages(plan), ["leaf", "other"]);
  }
});

test("package moves match repository identity and the unique package name across snapshots", () => {
  const base = makeSnapshot([{ name: "root", manifest: "Cargo.toml" }, { name: "leaf", manifest: "old/leaf/Cargo.toml" }], { sha: BASE_SHA });
  const head = makeSnapshot([{ name: "root", manifest: "Cargo.toml" }, { name: "leaf", manifest: "new/leaf/Cargo.toml" }]);
  const plan = createValidationPlan(request({ base, head, changes: [{ status: "R", oldPath: "old/leaf/src/lib.rs", path: "new/leaf/src/lib.rs" }] }));
  assert.equal(plan.mode, "selective");
  assert.equal(plan.testPackages[0].manifest, "new/leaf/Cargo.toml");
  assert.deepEqual(plan.removedPackages[0].replacementPackageKeys, ["new/leaf/Cargo.toml::leaf"]);
});

test("modified paths consult both old and new ownership registrations", () => {
  const base = makeSnapshot([{ name: "root", manifest: "Cargo.toml" }, { name: "leaf", inputs: ["shared/"] }, { name: "other", inputs: ["other/"] }], { sha: BASE_SHA });
  const head = makeSnapshot([{ name: "root", manifest: "Cargo.toml" }, { name: "leaf", inputs: ["leaf/"] }, { name: "other", inputs: ["shared/"] }]);
  const plan = createValidationPlan(request({ base, head, changes: [{ status: "M", path: "shared/value.rs" }] }));
  assert.deepEqual(packages(plan), ["leaf", "other"]);
});

test("added source consults current ownership and missing base cannot produce an empty success", () => {
  const plan = createValidationPlan(request({ changes: [{ status: "A", path: "crates/leaf/src/new.rs" }] }));
  assert.deepEqual(packages(plan), ["leaf"]);
  const unavailable = request();
  unavailable.base = { error: "permission denied" };
  assertFull(createValidationPlan(unavailable), "snapshot-unavailable");
});

test("cycles are diagnosed and terminate with an explicit full plan", () => {
  const head = makeSnapshot([
    { name: "root", manifest: "Cargo.toml" }, { name: "leaf", dependencies: ["other"] }, { name: "other", dependencies: ["leaf"] },
  ]);
  assertFull(createValidationPlan(request({ head })), "workspace-dependency-cycle");
});

test("opaque IDs, checkout roots, path separators and metadata ordering do not change the plan", () => {
  const first = request();
  const second = request({ head: makeSnapshot(undefined, { root: "C:\\another\\checkout" }) });
  second.head.metadata.packages.reverse();
  second.head.metadata.workspace_members.reverse();
  second.head.metadata.resolve.nodes.reverse();
  second.head.ownership.packages.reverse();
  second.changes[0].path = "crates\\leaf\\src\\lib.rs";
  for (const snap of [second.base, second.head]) {
    const mapping = new Map(snap.metadata.packages.map((pkg, index) => [pkg.id, "DO NOT PARSE / " + index + " # ? opaque"]));
    for (const pkg of snap.metadata.packages) pkg.id = mapping.get(pkg.id);
    snap.metadata.workspace_members = snap.metadata.workspace_members.map((id) => mapping.get(id));
    snap.metadata.resolve.root = mapping.get(snap.metadata.resolve.root);
    for (const node of snap.metadata.resolve.nodes) {
      node.id = mapping.get(node.id);
      for (const dependency of node.deps) dependency.pkg = mapping.get(dependency.pkg);
    }
  }
  assert.deepEqual(createValidationPlan(second), createValidationPlan(first));
});

test("same-name lib and bin targets stay distinct", () => {
  const head = makeSnapshot([
    { name: "root", manifest: "Cargo.toml" },
    { name: "leaf", targets: [
      { name: "shared", kind: ["lib"], test: true, doctest: true },
      { name: "shared", kind: ["bin"], test: true, doctest: false },
    ] },
  ]);
  const plan = createValidationPlan(request({ head }));
  assert.equal(plan.testPackages[0].targets.length, 2);
  assert.deepEqual(plan.testPackages[0].targets.map((target) => target.kind).sort(), ["bin", "lib"]);
});

test("DTO/serialization/event/IPC/plugin inputs preserve root consumer validation", () => {
  for (const path of ["schema/event.json", "ipc/message.json", "plugins/protocol.json", "fixtures/shared-dto.json"]) {
    const head = makeSnapshot([{ name: "root", manifest: "Cargo.toml", externalInputs: [path] }, { name: "leaf" }]);
    assertFull(createValidationPlan(request({ head, changes: [{ status: "M", path }] })), "root-consumer-requires-full");
  }
});

test("behavior contracts propagate beyond Cargo edges without inventing a compile dependency", () => {
  const head = makeSnapshot([{ name: "root", manifest: "Cargo.toml" }, { name: "leaf" }, { name: "consumer" }], {
    contracts: [contract({ providers: ["leaf"] })],
  });
  const plan = createValidationPlan(request({ head }));
  assert.equal(plan.mode, "selective");
  assert.deepEqual(packages(plan, "compilePackages"), ["leaf"]);
  assert.deepEqual(packages(plan), ["consumer", "leaf"]);
  assert.equal(plan.contractTests[0].id, "plugin-contract");
  assert.ok(plan.testPackages.every((pkg) => pkg.coverage === "full"));
});

test("external contract fixtures select behavior owners and preserve OS/filter conditions", () => {
  const head = makeSnapshot([{ name: "root", manifest: "Cargo.toml" }, { name: "leaf" }, { name: "consumer" }], {
    contracts: [contract({ inputs: ["contract-fixtures/"], platforms: ["windows"] })],
  });
  const plan = createValidationPlan(request({ head, changes: [{ status: "M", path: "contract-fixtures/saved-event.json" }] }));
  assert.deepEqual(packages(plan, "compilePackages"), []);
  assert.deepEqual(packages(plan), ["consumer"]);
  assert.deepEqual(plan.contractTests[0].platforms, ["windows"]);
  assert.equal(plan.contractTests[0].filter, "plugin::contract::");
});

test("unknown contract provider, missing consumer coverage and generated inputs fail closed", () => {
  const unregistered = request();
  unregistered.head.ownership.packages[1].consumerCoverage = "contracts";
  assertFull(createValidationPlan(unregistered), "consumer-coverage-unregistered");
  const provider = request();
  provider.head.ownership.contracts = [contract({ owner: "leaf", target: { name: "leaf", kind: "lib" }, providers: ["missing"] })];
  assertFull(createValidationPlan(provider), "ownership-invalid");
  assertFull(createValidationPlan(request({ changes: [{ status: "M", path: "generated/unowned.schema" }] })), "unregistered-input");
});

test("Askama admin HTML keeps both Rust and Node validation", () => {
  const head = makeSnapshot([{ name: "root", manifest: "Cargo.toml", externalInputs: ["templates/"] }, { name: "leaf" }]);
  const plan = createValidationPlan(request({ head, changes: [{ status: "M", path: "templates/admin/index.html" }] }));
  assertFull(plan, "root-consumer-requires-full");
  assert.equal(plan.nodeRequired, true);
});

test("mixed leaf and Desktop changes keep the Node surface requirement", () => {
  const plan = createValidationPlan(request({ changes: [
    { status: "M", path: "crates/leaf/src/lib.rs" }, { status: "M", path: "desktop/src/view.ts" },
  ] }));
  assert.equal(plan.mode, "selective");
  assert.equal(plan.nodeRequired, true);
  assert.deepEqual(packages(plan), ["leaf"]);
});

test("documentation and executable documentation are classified explicitly", () => {
  const docs = createValidationPlan(request({ changes: [{ status: "M", path: "docs/reference/example.md" }] }));
  assert.equal(docs.docsOnly, true);
  assert.equal(docs.nodeRequired, false);
  assert.deepEqual(docs.testPackages, []);
  const executable = createValidationPlan(request({ changes: [{ status: "M", path: "docs/plan/17-akra-desktop/scripts/proxy_ws_check.mjs" }] }));
  assert.equal(executable.docsOnly, false);
  assert.equal(executable.nodeRequired, true);
});

test("global build inputs, unmerged and invalid paths all select full validation", () => {
  for (const path of ["Cargo.toml", "Cargo.lock", ".cargo/config.toml", "rust-toolchain.toml", "scripts/ci-impact-plan.mjs", "build.rs"]) {
    assertFull(createValidationPlan(request({ changes: [{ status: "M", path }] })), "global-input");
  }
  assertFull(createValidationPlan(request({ changes: [{ status: "U", path: "crates/leaf/src/lib.rs" }] })), "unmerged-change");
  assertFull(createValidationPlan(request({ changes: [{ status: "M", path: "../outside.rs" }] })), "invalid-change-record");
  assertFull(createValidationPlan(request({ changes: [] })), "empty-or-missing-change-set");
});

test("planned and unregistered workspace members cannot silently disappear", () => {
  const planned = request();
  planned.head.ownership.packages[1].state = "planned";
  assertFull(createValidationPlan(planned), "unregistered-workspace-member");
  const absent = request();
  absent.head.ownership.packages = absent.head.ownership.packages.filter((pkg) => pkg.name !== "leaf");
  assertFull(createValidationPlan(absent), "unregistered-workspace-member");
});

test("broken resolve graphs fall back to full but unusable head inventories throw", () => {
  const broken = request();
  broken.head.metadata.resolve.nodes = [];
  assertFull(createValidationPlan(broken), "workspace-resolve-node-missing");
  const malformed = request();
  malformed.head.metadata.resolve.nodes[1].deps = [null];
  assertFull(createValidationPlan(malformed), "resolved-dependency-missing");
  const noHead = request();
  noHead.head.metadata = null;
  assert.throws(() => createValidationPlan(noHead), /workspace member inventory/);
  const noTargets = request();
  noTargets.head.metadata.packages[1].targets = [];
  assert.throws(() => createValidationPlan(noTargets), /target inventory/);
});

test("a package without an executable test target is an explicit full fallback", () => {
  const head = makeSnapshot([{ name: "root", manifest: "Cargo.toml" }, {
    name: "leaf", targets: [{ name: "leaf", kind: ["bin"], test: false, doctest: false }],
  }]);
  assertFull(createValidationPlan(request({ head })), "package-has-no-registered-test-target");
});

test("source, local changes, configuration, ownership and graph affect the plan hash", () => {
  const input = request();
  const baseline = createValidationPlan(input);
  const changes = [
    (next) => { next.head.sha = "c".repeat(40); },
    (next) => { next.base.sha = "d".repeat(40); },
    (next) => { next.localChangeId = "content-sha256:" + "e".repeat(64); },
    (next) => { next.configuration = { platforms: ["windows"], features: ["all"], profile: "test" }; },
    (next) => { next.head.ownership.revision++; },
    (next) => { next.head.metadata.packages[1].features = { new_feature: [] }; next.head.metadata.resolve.nodes[1].features = ["new_feature"]; },
  ];
  for (const change of changes) {
    const next = clone(input);
    change(next);
    assert.notEqual(createValidationPlan(next).planHash, baseline.planHash);
  }
  assert.deepEqual(input, request(), "planning must not mutate its input snapshot");
});

test("NUL-delimited diffs preserve rename sides, whitespace and shell metacharacters", () => {
  assert.deepEqual(parseGitNameStatusZ("R100\0old/a b.rs\0new/a\tb.rs\0M\0src/$(literal).rs\0"), [
    { status: "R", oldPath: "old/a b.rs", path: "new/a\tb.rs" }, { status: "M", path: "src/$(literal).rs" },
  ]);
  assert.throws(() => parseGitNameStatusZ("R100\0one\0"), /truncated git rename/);
  assert.throws(() => parseGitNameStatusZ("M\0path"), /end with NUL/);
  assert.throws(() => parseGitNameStatusZ("M\0../escape\0"), /repository relative/);
});

test("explanations expose selection, exclusion and fallback evidence", () => {
  const selective = explainPlan(createValidationPlan(request()));
  assert.match(selective, /Compile: crates\/leaf/);
  assert.match(selective, /Excluded: Cargo.toml::root/);
  const fallback = explainPlan(createValidationPlan(request({ changes: [{ status: "M", path: "Cargo.lock" }] })));
  assert.match(fallback, /Full: global-input:Cargo.lock/);
});

test("CLI reads JSON snapshots without modifying a lockfile or invoking Cargo", () => {
  const directory = mkdtempSync(join(tmpdir(), "akra-impact-plan-"));
  try {
    const inputPath = join(directory, "request.json"), lockPath = join(directory, "Cargo.lock");
    const sentinel = "unchanged lock sentinel\n";
    writeFileSync(inputPath, JSON.stringify(request()));
    writeFileSync(lockPath, sentinel);
    const script = fileURLToPath(new URL("./ci-impact-plan.mjs", import.meta.url));
    const result = spawnSync(process.execPath, [script, "--input", inputPath, "--explain"], { cwd: directory, encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(JSON.parse(result.stdout).mode, "selective");
    assert.match(result.stderr, /Validation plan/);
    assert.equal(readFileSync(lockPath, "utf8"), sentinel);
    const invalid = spawnSync(process.execPath, [script, "--input", "-"], { input: "{", encoding: "utf8" });
    assert.equal(invalid.status, 1);
    assert.equal(invalid.stdout, "");
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});


test("plan integrity detects changed selections even when a saved hash is retained", () => {
  const plan = createValidationPlan(request());
  assert.equal(verifyValidationPlanHash(plan), true);
  plan.testPackages = [];
  assert.equal(verifyValidationPlanHash(plan), false);
  assert.equal(verifyValidationPlanHash({}), false);
});

test("a registry dependency with the same package name is not a workspace consumer edge", () => {
  const head = makeSnapshot();
  const external = {
    id: "opaque external ID / not a workspace ID", name: "leaf", version: "99.0.0",
    source: "registry+opaque-source", manifest_path: "/cargo/registry/leaf/Cargo.toml",
    targets: [{ name: "leaf", kind: ["lib"], test: true, doctest: true }], dependencies: [], features: {},
  };
  head.metadata.packages.push(external);
  head.metadata.resolve.nodes.push({ id: external.id, features: [], deps: [] });
  head.metadata.resolve.nodes[0].deps.push({ name: "leaf", pkg: external.id, dep_kinds: [{ kind: null, target: null }] });
  const plan = createValidationPlan(request({ head }));
  assert.equal(plan.mode, "selective");
  assert.deepEqual(packages(plan), ["leaf"]);
});

test("declared target-specific local edges survive a platform-pruned resolve edge", () => {
  const head = makeSnapshot([
    { name: "root", manifest: "Cargo.toml", dependencies: [{ name: "leaf", target: "cfg(windows)", kind: "build" }] },
    { name: "leaf" },
  ]);
  head.metadata.resolve.nodes[0].deps = [];
  assertFull(createValidationPlan(request({ head })), "root-consumer-requires-full");
});

test("unknown configurations and missing target test flags cannot produce a misleading plan", () => {
  assert.throws(() => createValidationPlan(request({ configuration: { features: ["unrepresented-mode"] } })), /configuration/);
  const head = makeSnapshot();
  delete head.metadata.packages[1].targets[0].test;
  assert.throws(() => createValidationPlan(request({ head })), /target validation inventory/);
});
