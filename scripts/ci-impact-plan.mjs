#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { classifyChangedPaths } from "./ci-scope.mjs";

const SCHEMA = 1;
const PLATFORMS = new Set(["linux", "macos", "windows"]);
const LIB_KINDS = new Set(["lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"]);
const OTHER_KINDS = new Set(["bin", "test", "bench", "example", "custom-build"]);
const MANIFEST_INPUT = /(?:^|\/)Cargo\.(?:toml|lock)$/;
const INFRASTRUCTURE = /^(?:Cargo\.(?:toml|lock)|rust-toolchain(?:\.toml)?|build\.rs|AGENTS\.md|(?:deny|clippy)\.toml)$|^(?:\.cargo|\.github|scripts)\//;

function object(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function sorted(values) {
  return [...new Set(values)].sort();
}
function canonical(value, sortArrays = false) {
  if (Array.isArray(value)) {
    const result = value.map((item) => canonical(item, sortArrays));
    return sortArrays ? result.sort((a, b) => JSON.stringify(a) < JSON.stringify(b) ? -1 : JSON.stringify(a) > JSON.stringify(b) ? 1 : 0) : result;
  }
  if (object(value)) {
    return Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key], sortArrays)]));
  }
  return value;
}
function hash(value, sortArrays = false) {
  return createHash("sha256").update(JSON.stringify(canonical(value, sortArrays))).digest("hex");
}
function stableObjects(values) {
  return [...new Map(values.map((value) => [JSON.stringify(canonical(value)), value])).entries()]
    .sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([, value]) => value);
}
function slash(value) {
  return value.replaceAll("\\", "/");
}
function relativePath(value, { prefix = false } = {}) {
  if (typeof value !== "string" || !value || value.includes("\0")) throw new Error("invalid repository path");
  const path = slash(value);
  if (path.startsWith("/") || /^[A-Za-z]:/.test(path) || path.split("/").some((part) => part === ".." || part === ".")) {
    throw new Error("path must be repository relative: " + value);
  }
  const trailing = prefix && path.endsWith("/");
  const parts = path.split("/");
  if (parts.some((part, index) => !part && !(trailing && index === parts.length - 1))) throw new Error("empty path component");
  return path;
}
function withinWorkspace(value, root) {
  if (typeof value !== "string" || typeof root !== "string") throw new Error("missing workspace path");
  const normalizedRoot = slash(root).replace(/\/+$/, "");
  const normalizedValue = slash(value);
  const windows = /^[A-Za-z]:\//.test(normalizedRoot);
  const lhs = windows ? normalizedValue.toLowerCase() : normalizedValue;
  const rhs = windows ? normalizedRoot.toLowerCase() : normalizedRoot;
  if (!lhs.startsWith(rhs + "/")) throw new Error("manifest is outside workspace: " + value);
  return relativePath(normalizedValue.slice(normalizedRoot.length + 1));
}
function matches(path, pattern) {
  return pattern.endsWith("/") ? path.startsWith(pattern) : path === pattern;
}
function targetKind(kinds) {
  if (!Array.isArray(kinds) || kinds.length === 0) throw new Error("missing Cargo target kind");
  if (kinds.every((kind) => LIB_KINDS.has(kind))) return "lib";
  if (kinds.length === 1 && OTHER_KINDS.has(kinds[0])) return kinds[0];
  throw new Error("unsupported Cargo target kind: " + JSON.stringify(kinds));
}
function targetSummary(target) {
  if (!object(target) || typeof target.name !== "string" || !target.name) throw new Error("missing Cargo target name");
  if (typeof target.test !== "boolean" || typeof target.doctest !== "boolean"
      || !Array.isArray(target["required-features"] ?? [])
      || (target["required-features"] ?? []).some((feature) => typeof feature !== "string" || !feature)) {
    throw new Error("incomplete Cargo target validation inventory");
  }
  return {
    name: target.name,
    kind: targetKind(target.kind),
    requiredFeatures: sorted(target["required-features"] ?? []),
    doctest: target.doctest === true,
    test: target.test === true,
  };
}
function packageKey(manifest, name) {
  return manifest + "::" + name;
}

function inventory(metadata, label) {
  if (!object(metadata) || metadata.version !== 1 || !Array.isArray(metadata.packages)
      || !Array.isArray(metadata.workspace_members) || metadata.workspace_members.length === 0) {
    throw new Error(label + " metadata has no usable workspace member inventory");
  }
  const byId = new Map();
  for (const pkg of metadata.packages) {
    if (!object(pkg) || typeof pkg.id !== "string" || byId.has(pkg.id)) throw new Error(label + " metadata has duplicate/invalid opaque package IDs");
    byId.set(pkg.id, pkg);
  }
  const members = new Map();
  const memberIds = new Map();
  for (const id of metadata.workspace_members) {
    const pkg = byId.get(id);
    if (!pkg || typeof pkg.name !== "string" || !pkg.name || !Array.isArray(pkg.targets) || pkg.targets.length === 0) {
      throw new Error(label + " member/target inventory is incomplete");
    }
    const manifest = withinWorkspace(pkg.manifest_path, metadata.workspace_root);
    const key = packageKey(manifest, pkg.name);
    if (members.has(key) || memberIds.has(id)) throw new Error(label + " workspace member is duplicated");
    const targets = stableObjects(pkg.targets.map(targetSummary));
    if (targets.length !== pkg.targets.length) throw new Error(label + " has duplicated Cargo targets");
    if (targets.filter((target) => target.kind !== "custom-build").length === 0) throw new Error(label + " has no executable Cargo target");
    const targetIds = targets.map((target) => target.kind + "::" + target.name);
    if (new Set(targetIds).size !== targetIds.length) throw new Error(label + " has ambiguous Cargo targets");
    members.set(key, { packageKey: key, name: pkg.name, manifest, targets, raw: pkg });
    memberIds.set(id, key);
  }
  return { members, memberIds, byId };
}

function ownership(raw, inv, label, reasons) {
  const fail = (code) => reasons.add(label + ":" + code);
  const result = { raw, entries: new Map(), fullInputs: [], contracts: [], revision: null, hash: raw == null ? null : hash(raw, true) };
  if (!object(raw) || raw.schemaVersion !== 1 || !Number.isInteger(raw.revision)
      || !Array.isArray(raw.packages) || !Array.isArray(raw.fullInputs) || !Array.isArray(raw.contracts)
      || typeof raw.rootPackage !== "string" || !raw.rootPackage) {
    fail("ownership-incomplete");
    return result;
  }
  result.revision = raw.revision;
  const names = new Set();
  try {
    result.fullInputs = sorted(raw.fullInputs.map((path) => relativePath(path, { prefix: true })));
    for (const entry of raw.packages) {
      if (!object(entry) || typeof entry.name !== "string" || !entry.name || names.has(entry.name)
          || !["active", "planned"].includes(entry.state) || !Array.isArray(entry.sourcePrefixes)
          || !Array.isArray(entry.externalInputs) || !Array.isArray(entry.workspaceDependencies)) {
        throw new Error("invalid package registration");
      }
      names.add(entry.name);
      const manifest = relativePath(entry.manifest);
      const key = packageKey(manifest, entry.name);
      result.entries.set(key, {
        ...entry, manifest, packageKey: key,
        inputs: sorted([manifest, ...entry.sourcePrefixes, ...entry.externalInputs].map((path) => relativePath(path, { prefix: true }))),
      });
      if (entry.state === "active" && !inv.members.has(key)) fail("registered-member-missing:" + key);
      if (entry.state === "active" && entry.consumerCoverage !== "full") fail("consumer-coverage-unregistered:" + key);
    }
    for (const [key] of inv.members) {
      if (result.entries.get(key)?.state !== "active") fail("unregistered-workspace-member:" + key);
    }
    if (![...inv.members.values()].some((pkg) => pkg.name === raw.rootPackage)) fail("root-package-missing");
    const ids = new Set();
    for (const contract of raw.contracts) {
      if (!object(contract) || typeof contract.id !== "string" || !contract.id || ids.has(contract.id)
          || typeof contract.owner !== "string" || !object(contract.target)
          || typeof contract.filter !== "string" || !contract.filter
          || !Array.isArray(contract.platforms) || contract.platforms.length === 0
          || contract.platforms.some((os) => !PLATFORMS.has(os))
          || typeof contract.features !== "string" || typeof contract.isolation !== "string") {
        throw new Error("invalid contract registration");
      }
      ids.add(contract.id);
      const owner = [...inv.members.values()].filter((pkg) => pkg.name === contract.owner);
      if (owner.length !== 1 || !owner[0].targets.some((target) => target.kind === contract.target.kind && target.name === contract.target.name)) {
        throw new Error("contract owner/target missing");
      }
      const providers = contract.providers ?? [];
      if (!Array.isArray(providers) || providers.some((name) => ![...inv.members.values()].some((pkg) => pkg.name === name))) {
        throw new Error("contract provider missing");
      }
      const inputs = (contract.inputs ?? []).map((path) => relativePath(path, { prefix: true }));
      result.contracts.push({ ...contract, providers: sorted(providers), inputs: sorted(inputs), packageKey: owner[0].packageKey });
    }
  } catch {
    fail("ownership-invalid");
  }
  return result;
}

function graph(metadata, inv, own, label, reasons) {
  const edges = [];
  const features = [];
  const fail = (code) => reasons.add(label + ":" + code);
  const nodes = new Map();
  if (!object(metadata.resolve) || !Array.isArray(metadata.resolve.nodes)) {
    fail("metadata-resolve-missing");
  } else {
    for (const node of metadata.resolve.nodes) {
      if (!object(node) || typeof node.id !== "string" || nodes.has(node.id)
          || !inv.byId.has(node.id) || !Array.isArray(node.deps) || !Array.isArray(node.features)) {
        fail("metadata-resolve-incomplete");
        continue;
      }
      nodes.set(node.id, node);
    }
  }
  const addEdge = (from, to, kind, target, extra = {}) => {
    if (![null, "normal", "dev", "build"].includes(kind) || !(target == null || typeof target === "string")) {
      fail("dependency-kind-or-target-unknown");
      return;
    }
    edges.push({ from, to, kind: kind ?? "normal", target: target ?? null, ...extra });
  };
  for (const [id, key] of inv.memberIds) {
    const pkg = inv.members.get(key);
    const node = nodes.get(id);
    if (!node) fail("workspace-resolve-node-missing:" + key);
    const declaredFeatures = object(pkg.raw.features) ? Object.keys(pkg.raw.features) : [];
    if (!object(pkg.raw.features)) fail("workspace-features-missing:" + key);
    if (node && declaredFeatures.some((feature) => !node.features.includes(feature))) fail("all-features-graph-required:" + key);
    features.push({ packageKey: key, declared: canonical(pkg.raw.features ?? {}, true), resolved: sorted(node?.features ?? []) });
    if (node) {
      for (const edge of node.deps) {
        if (!object(edge) || typeof edge.pkg !== "string" || !nodes.has(edge.pkg) || !inv.byId.has(edge.pkg)
            || !Array.isArray(edge.dep_kinds) || edge.dep_kinds.length === 0) {
          fail("resolved-dependency-missing:" + key);
          continue;
        }
        const to = inv.memberIds.get(edge.pkg);
        if (!to) continue;
        for (const kind of edge.dep_kinds) addEdge(key, to, kind.kind, kind.target, { origin: "resolve" });
      }
    }
    if (!Array.isArray(pkg.raw.dependencies)) {
      fail("declared-dependencies-missing:" + key);
      continue;
    }
    for (const dependency of pkg.raw.dependencies) {
      if (!object(dependency)) {
        fail("declared-dependency-invalid:" + key);
        continue;
      }
      if (!dependency.path) continue;
      let manifest;
      try {
        manifest = withinWorkspace(slash(dependency.path).replace(/\/+$/, "") + "/Cargo.toml", metadata.workspace_root);
      } catch {
        fail("local-dependency-outside-workspace:" + key);
        continue;
      }
      const target = [...inv.members.values()].filter((candidate) => candidate.manifest === manifest);
      if (target.length !== 1) {
        fail("local-dependency-not-member:" + manifest);
        continue;
      }
      if (target[0].name !== dependency.name) fail("declared-dependency-name-mismatch:" + key);
      addEdge(key, target[0].packageKey, dependency.kind, dependency.target, {
        origin: "manifest", optional: dependency.optional === true,
        rename: dependency.rename ?? null, features: sorted(dependency.features ?? []),
      });
      const registered = own.entries.get(key);
      if (registered && !registered.workspaceDependencies.includes(target[0].name)) fail("dependency-ownership-mismatch:" + key);
    }
  }
  // Validate the entire resolved graph, including dependency nodes, before using it.
  for (const node of nodes.values()) {
    for (const edge of node.deps) {
      if (!object(edge) || !nodes.has(edge.pkg) || !Array.isArray(edge.dep_kinds) || edge.dep_kinds.length === 0) {
        fail("transitive-resolve-incomplete");
        continue;
      }
      // A non-member may consume a patched workspace crate. The member-only
      // reverse graph cannot prove that consumer closure, so retain the full gate.
      if (!inv.memberIds.has(node.id) && inv.memberIds.has(edge.pkg)) {
        fail("external-resolve-consumes-workspace-member:" + inv.memberIds.get(edge.pkg));
      }
    }
  }
  const adjacency = new Map([...inv.members.keys()].map((key) => [key, new Set()]));
  for (const edge of edges) adjacency.get(edge.from)?.add(edge.to);
  const active = new Set(), complete = new Set();
  function visit(key) {
    if (active.has(key)) return true;
    if (complete.has(key)) return false;
    active.add(key);
    for (const child of adjacency.get(key) ?? []) if (visit(child)) return true;
    active.delete(key);
    complete.add(key);
    return false;
  }
  if ([...adjacency.keys()].some(visit)) fail("workspace-dependency-cycle");
  const normalized = {
    schemaVersion: SCHEMA,
    members: stableObjects([...inv.members.values()].map(({ packageKey, name, manifest, targets }) => ({ packageKey, name, manifest, targets }))),
    features: stableObjects(features),
    edges: stableObjects(edges),
    resolvedDependencies: stableObjects([...nodes.values()].map((node) => ({
      package: descriptor(inv.byId.get(node.id), inv, metadata),
      features: sorted(node.features),
      dependencies: stableObjects(node.deps.filter((edge) => object(edge) && inv.byId.has(edge.pkg)).map((edge) => ({
        package: descriptor(inv.byId.get(edge.pkg), inv, metadata),
        name: edge.name ?? null,
        kinds: stableObjects(edge.dep_kinds ?? []),
      }))),
    }))),
  };
  return { ...normalized, hash: hash(normalized) };
}
function descriptor(pkg, inv, metadata) {
  const member = inv.memberIds.get(pkg.id);
  if (member) return { packageKey: member };
  let repositoryPath = null;
  if (pkg.source == null) {
    try { repositoryPath = withinWorkspace(pkg.manifest_path, metadata.workspace_root); } catch { repositoryPath = "outside-workspace"; }
  }
  return { name: pkg.name, version: pkg.version ?? null, source: pkg.source ?? null, repositoryPath };
}

function snapshot(input, label, reasons, required = false) {
  if (!object(input) || typeof input.sha !== "string" || !/^[a-f0-9]{40,64}$/i.test(input.sha)) {
    if (required) throw new Error("head snapshot must identify an exact source SHA");
    reasons.add("base:snapshot-unavailable");
    return null;
  }
  let inv;
  try { inv = inventory(input.metadata, label); } catch (error) {
    if (required) throw error;
    reasons.add("base:inventory-unavailable");
    return null;
  }
  const own = ownership(input.ownership, inv, label, reasons);
  let dependencyGraph;
  try { dependencyGraph = graph(input.metadata, inv, own, label, reasons); } catch {
    reasons.add(label + ":metadata-graph-invalid");
    dependencyGraph = { edges: [], hash: hash(input.metadata, true) };
  }
  return { sha: input.sha, ...inv, ownership: own, graph: dependencyGraph };
}

function normalizeChanges(input, reasons) {
  if (!Array.isArray(input) || input.length === 0) {
    reasons.add("empty-or-missing-change-set");
    return [];
  }
  const output = [];
  for (const change of input) {
    try {
      if (!object(change) || typeof change.status !== "string" || !/^[AMDRCTU](?:\d+)?$/.test(change.status)) throw new Error("unknown change status");
      const status = change.status[0];
      const record = { status, path: relativePath(change.path) };
      if (status === "R" || status === "C") record.oldPath = relativePath(change.oldPath);
      if (status === "U") reasons.add("unmerged-change:" + record.path);
      output.push(record);
    } catch {
      reasons.add("invalid-change-record");
    }
  }
  return stableObjects(output);
}

export function parseGitNameStatusZ(output) {
  const fields = Buffer.isBuffer(output) ? output.toString("utf8").split("\0") : String(output).split("\0");
  if (fields.pop() !== "") throw new Error("git --name-status -z output must end with NUL");
  const changes = [];
  for (let index = 0; index < fields.length;) {
    const status = fields[index++];
    if (!/^[AMDRCTU](?:\d+)?$/.test(status)) throw new Error("unsupported git change status");
    const first = fields[index++];
    if (first === undefined) throw new Error("truncated git path");
    if (status[0] === "R" || status[0] === "C") {
      const path = fields[index++];
      if (path === undefined) throw new Error("truncated git rename");
      changes.push({ status: status[0], oldPath: relativePath(first), path: relativePath(path) });
    } else {
      changes.push({ status: status[0], path: relativePath(first) });
    }
  }
  return changes;
}

function configuration(raw = {}) {
  if (!object(raw) || Object.keys(raw).some((key) => !["platforms", "features", "profile", "targetTriples"].includes(key))) throw new Error("unsupported plan configuration");
  const platforms = raw.platforms ?? ["linux", "macos", "windows"];
  const features = raw.features ?? ["all", "default"];
  const profile = raw.profile ?? "test";
  const targetTriples = raw.targetTriples ?? [];
  if (!Array.isArray(platforms) || platforms.length === 0 || platforms.some((value) => !PLATFORMS.has(value))
      || !Array.isArray(features) || features.length === 0 || features.some((value) => !["all", "default"].includes(value))
      || typeof profile !== "string" || !/^[A-Za-z0-9_-]+$/.test(profile)
      || !Array.isArray(targetTriples) || targetTriples.some((value) => typeof value !== "string" || !/^[A-Za-z0-9_.-]+$/.test(value))) {
    throw new Error("invalid plan configuration");
  }
  return { platforms: sorted(platforms), features: sorted(features), profile, targetTriples: sorted(targetTriples) };
}
function addReason(map, key, reason) {
  if (!map.has(key)) map.set(key, new Set());
  map.get(key).add(reason);
}
function propagate(seeds, reverse, equivalents) {
  const output = new Map([...seeds].map(([key, reasons]) => [key, new Set(reasons)]));
  const queue = [...output.keys()];
  for (let index = 0; index < queue.length; index++) {
    const key = queue[index];
    for (const other of equivalents.get(key) ?? []) {
      const known = output.has(other);
      addReason(output, other, "same-package-across-snapshots:" + key);
      if (!known) queue.push(other);
    }
    for (const consumer of reverse.get(key) ?? []) {
      const known = output.has(consumer);
      addReason(output, consumer, "consumer-of:" + key);
      if (!known) queue.push(consumer);
    }
  }
  return output;
}
function selectedPackage(pkg, reasons, tests = false) {
  return {
    packageKey: pkg.packageKey, name: pkg.name, manifest: pkg.manifest,
    targets: pkg.targets.filter((target) => target.kind !== "custom-build" && (!tests || target.test || target.doctest)),
    ...(tests ? { coverage: "full" } : {}),
    reasons: sorted(reasons),
  };
}

/**
 * Read-only planner. The caller supplies full base/head metadata and a versioned
 * ownership registry; this module never invokes Cargo/Git or writes worktree files.
 */
export function createValidationPlan(request) {
  if (!object(request) || request.schemaVersion !== SCHEMA) throw new Error("unsupported request schema");
  if (request.localChangeId != null && (typeof request.localChangeId !== "string" || !request.localChangeId)) throw new Error("invalid local change identifier");
  const config = configuration(request.configuration);
  const fullReasons = new Set();
  const head = snapshot(request.head, "head", fullReasons, true);
  const base = snapshot(request.base, "base", fullReasons);
  const changes = normalizeChanges(request.changes, fullReasons);
  const snapshots = [["base", base], ["head", head]].filter(([, value]) => value);
  const allPackages = new Map(snapshots.flatMap(([, value]) => [...value.members]));
  const reverse = new Map(), equivalents = new Map();
  for (const [, value] of snapshots) for (const edge of value.graph.edges) {
    if (!reverse.has(edge.to)) reverse.set(edge.to, new Set());
    reverse.get(edge.to).add(edge.from);
  }
  if (base) {
    for (const oldPackage of base.members.values()) {
      const candidates = [...head.members.values()].filter((pkg) => pkg.name === oldPackage.name);
      if (!head.members.has(oldPackage.packageKey) && candidates.length === 1) {
        const newKey = candidates[0].packageKey;
        equivalents.set(oldPackage.packageKey, new Set([newKey]));
        equivalents.set(newKey, new Set([oldPackage.packageKey]));
      }
    }
  }
  const compileSeeds = new Map(), behaviorSeeds = new Map();
  const affectedContracts = new Set(), explanations = new Set();
  let nodeRequired = false;
  const paths = sorted(changes.flatMap((change) => [change.path, ...(change.oldPath ? [change.oldPath] : [])]));
  for (const path of paths) {
    const legacy = classifyChangedPaths([path]);
    if (!legacy.full && legacy.node) nodeRequired = true;
    if (MANIFEST_INPUT.test(path) || INFRASTRUCTURE.test(path) || snapshots.some(([, value]) => value.ownership.fullInputs.some((pattern) => matches(path, pattern)))) {
      fullReasons.add("global-input:" + path);
    }
  }
  for (const change of changes) {
    const observations = change.status === "A" ? [["head", head, change.path]]
      : change.status === "D" ? [["base", base, change.path]]
        : ["R", "C"].includes(change.status) ? [["base", base, change.oldPath], ["head", head, change.path]]
          : [["base", base, change.path], ["head", head, change.path]];
    for (const [label, value, path] of observations) {
      if (!value) continue;
      const owners = [...value.ownership.entries.values()].filter((entry) => entry.inputs.some((pattern) => matches(path, pattern)));
      const contracts = value.ownership.contracts.filter((contract) => contract.inputs.some((pattern) => matches(path, pattern)));
      const legacy = classifyChangedPaths([path]);
      const knownNonRust = !legacy.full && (legacy.docs_only || (legacy.node && !legacy.rust));
      if (owners.length === 0 && contracts.length === 0 && !knownNonRust && !INFRASTRUCTURE.test(path)) fullReasons.add("unregistered-input:" + label + ":" + path);
      if (knownNonRust) explanations.add("classified-non-rust-input:" + path);
      for (const owner of owners) {
        if (owner.state !== "active" || !value.members.has(owner.packageKey)) {
          fullReasons.add("planned-or-missing-owner:" + label + ":" + path);
          continue;
        }
        addReason(compileSeeds, owner.packageKey, label + ":input:" + path);
        explanations.add(label + ":owner:" + path + "=>" + owner.packageKey);
      }
      for (const contract of contracts) {
        addReason(behaviorSeeds, contract.packageKey, label + ":contract-input:" + contract.id + ":" + path);
        affectedContracts.add(contract.id);
      }
    }
  }
  const compileImpact = propagate(compileSeeds, reverse, equivalents);
  for (const [key, reasons] of compileImpact) for (const reason of reasons) addReason(behaviorSeeds, key, reason);
  let behaviorImpact = propagate(behaviorSeeds, reverse, equivalents);
  // Explicit providers describe behavioral contracts outside Cargo dependency edges.
  let changed = true;
  while (changed) {
    changed = false;
    const affectedNames = new Set([...behaviorImpact.keys()].map((key) => allPackages.get(key)?.name));
    for (const [label, value] of snapshots) for (const contract of value.ownership.contracts) {
      if (contract.providers.some((name) => affectedNames.has(name))) {
        if (!behaviorSeeds.has(contract.packageKey)) changed = true;
        addReason(behaviorSeeds, contract.packageKey, label + ":contract-provider:" + contract.id);
        affectedContracts.add(contract.id);
      }
    }
    behaviorImpact = propagate(behaviorSeeds, reverse, equivalents);
  }
  if ([...behaviorImpact.keys()].some((key) => allPackages.get(key)?.name === head.ownership.raw?.rootPackage)) {
    fullReasons.add("root-consumer-requires-full");
  }
  for (const pkg of head.members.values()) {
    if (behaviorImpact.has(pkg.packageKey) && !pkg.targets.some((target) => target.kind !== "custom-build" && (target.test || target.doctest))) {
      fullReasons.add("package-has-no-registered-test-target:" + pkg.packageKey);
    }
  }
  const mode = fullReasons.size ? "full" : "selective";
  if (mode === "full") nodeRequired = true;
  const compilePackages = [...head.members.values()]
    .filter((pkg) => mode === "full" || compileImpact.has(pkg.packageKey))
    .map((pkg) => selectedPackage(pkg, mode === "full" ? ["full-validation"] : compileImpact.get(pkg.packageKey)));
  const testPackages = [...head.members.values()]
    .filter((pkg) => mode === "full" || behaviorImpact.has(pkg.packageKey))
    .map((pkg) => selectedPackage(pkg, mode === "full" ? ["full-validation"] : behaviorImpact.get(pkg.packageKey), true));
  const contractTests = head.ownership.contracts
    .filter((contract) => mode === "full" || behaviorImpact.has(contract.packageKey) || affectedContracts.has(contract.id))
    .map((contract) => ({
      id: contract.id, packageKey: contract.packageKey,
      name: head.members.get(contract.packageKey).name, manifest: head.members.get(contract.packageKey).manifest,
      target: contract.target, filter: contract.filter,
      platforms: sorted(contract.platforms.filter((os) => config.platforms.includes(os))),
      features: contract.features, isolation: contract.isolation,
    })).filter((contract) => contract.platforms.length > 0);
  const removedPackages = base ? [...base.members.values()].filter((pkg) => !head.members.has(pkg.packageKey) && (compileImpact.has(pkg.packageKey) || behaviorImpact.has(pkg.packageKey))).map((pkg) => ({
    packageKey: pkg.packageKey, name: pkg.name, manifest: pkg.manifest,
    replacementPackageKeys: sorted(equivalents.get(pkg.packageKey) ?? []),
  })) : [];
  const output = {
    schemaVersion: SCHEMA, sourceSha: head.sha, baseSha: base?.sha ?? (typeof request.base?.sha === "string" && /^[a-f0-9]{40,64}$/i.test(request.base.sha) ? request.base.sha : null),
    localChangeId: request.localChangeId ?? null, configuration: config, mode,
    fullReasons: sorted(fullReasons), changes, nodeRequired,
    docsOnly: mode === "selective" && paths.length > 0 && paths.every((path) => classifyChangedPaths([path]).docs_only),
    compilePackages: stableObjects(compilePackages), testPackages: stableObjects(testPackages),
    contractTests: stableObjects(contractTests), removedPackages: stableObjects(removedPackages),
    excludedPackages: stableObjects([...head.members.values()].filter((pkg) => mode !== "full" && !compileImpact.has(pkg.packageKey) && !behaviorImpact.has(pkg.packageKey)).map((pkg) => ({
      packageKey: pkg.packageKey, name: pkg.name, manifest: pkg.manifest,
      reason: "outside-base-and-head-dependency-and-contract-closure",
    }))),
    ownership: { schemaVersion: 1, baseRevision: base?.ownership.revision ?? null, headRevision: head.ownership.revision, baseHash: base?.ownership.hash ?? null, headHash: head.ownership.hash },
    graphs: { schemaVersion: 1, baseHash: base?.graph.hash ?? null, headHash: head.graph.hash, edgePolicy: "all-normal-dev-build-optional-and-target-conditions-retained" },
    explanations: sorted(explanations),
  };
  if (mode === "selective" && !output.docsOnly && !nodeRequired && compilePackages.length === 0 && testPackages.length === 0) {
    throw new Error("cannot produce an executable validation plan");
  }
  return { ...output, planHash: hash(output) };
}

export function verifyValidationPlanHash(plan) {
  if (!object(plan) || plan.schemaVersion !== SCHEMA || typeof plan.planHash !== "string") return false;
  const { planHash, ...payload } = plan;
  return planHash === hash(payload);
}

export function explainPlan(plan) {
  return [
    "Validation plan " + plan.planHash + " (" + plan.mode + ")",
    "Source: " + plan.sourceSha + "; local: " + (plan.localChangeId ?? "clean"),
    ...plan.fullReasons.map((reason) => "Full: " + reason),
    ...plan.explanations.map((reason) => "Reason: " + reason),
    ...plan.compilePackages.map((pkg) => "Compile: " + pkg.packageKey + " [" + pkg.targets.map((target) => target.kind + ":" + target.name).join(", ") + "]"),
    ...plan.testPackages.map((pkg) => "Test (" + pkg.coverage + "): " + pkg.packageKey),
    ...plan.contractTests.map((contract) => "Contract: " + contract.id + " => " + contract.packageKey + "/" + contract.filter),
    ...plan.excludedPackages.map((pkg) => "Excluded: " + pkg.packageKey + " (" + pkg.reason + ")"),
    "Node surfaces: " + plan.nodeRequired,
  ].join("\n");
}

function main(args) {
  const index = args.indexOf("--input");
  if (index < 0 || !args[index + 1] || args.some((arg, i) => i !== index + 1 && !["--input", "--explain"].includes(arg))) {
    throw new Error("usage: node scripts/ci-impact-plan.mjs --input <request.json|-> [--explain]");
  }
  const request = JSON.parse(readFileSync(args[index + 1] === "-" ? 0 : args[index + 1], "utf8"));
  const plan = createValidationPlan(request);
  console.log(JSON.stringify(plan, null, 2));
  if (args.includes("--explain")) console.error(explainPlan(plan));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { main(process.argv.slice(2)); } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
