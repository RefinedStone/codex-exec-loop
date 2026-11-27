import { relative } from "node:path";

export function resolveContractTargets(metadata, selection, ownership, includeMembers = false) {
  if (!Array.isArray(metadata?.workspace_members) || !Array.isArray(metadata?.packages)) {
    throw new Error("Complete Cargo workspace metadata is required.");
  }
  const memberIds = new Set(metadata.workspace_members);
  const members = metadata.packages.filter((pkg) => memberIds.has(pkg.id));
  if (members.length !== memberIds.size) throw new Error("Workspace member metadata is incomplete.");
  let candidates;
  if (selection?.package) {
    candidates = members.filter((pkg) => pkg.id === selection.package || pkg.name === selection.package);
    if (candidates.length !== 1) throw new Error("Package selector must identify exactly one workspace package.");
  } else {
    const registered = new Map((ownership?.packages ?? []).map((pkg) => [pkg.name, pkg]));
    for (const pkg of members) {
      if (registered.get(pkg.name)?.state !== "active") {
        throw new Error("Unregistered workspace package: " + pkg.name);
      }
    }
    candidates = members;
  }
  const kind = selection?.targetKind ?? "lib";
  if (!["lib", "test", "bin"].includes(kind)) throw new Error("Unsupported test target kind: " + kind);
  return candidates.flatMap((pkg) => {
    const activeFeatures = new Set(metadata.resolve?.nodes?.find((node) => node.id === pkg.id)?.features ?? []);
    const includeAll = includeMembers && pkg.name !== "akra-ade";
    const targets = pkg.targets.filter((target) =>
      (includeAll ? target.kind.some((value) => ["lib", "bin", "test"].includes(value)) : target.kind.includes(kind))
      && target.test !== false
      && (!selection?.targetName || target.name === selection.targetName)
      && (target["required-features"] ?? []).every((feature) => activeFeatures.has(feature)));
    if ((!includeAll && targets.length !== 1) || (includeAll && !targets.length)) {
      throw new Error("Expected a supported enabled test target in " + pkg.name + ".");
    }
    const manifest = relative(metadata.workspace_root, pkg.manifest_path).replaceAll("\\", "/");
    return targets.map((target) => ({
      packageId: pkg.id, package: pkg.name, manifest,
      packageKey: manifest + "::" + pkg.name,
      target: { name: target.name, kind: includeAll ? target.kind.find((value) => ["lib", "bin", "test"].includes(value)) : kind },
    }));
  });
}

export function exactTestExecutable(output, request) {
  if (!request?.packageId || !request.target?.name || !request.target?.kind) {
    throw new Error("Exact package ID and target are required.");
  }
  const executables = new Set();
  for (const line of output.split(/\r?\n/).filter(Boolean)) {
    const message = JSON.parse(line);
    if (message.reason === "compiler-artifact"
      && message.package_id === request.packageId
      && message.target?.name === request.target.name
      && message.target?.kind?.includes(request.target.kind)
      && message.profile?.test === true
      && typeof message.executable === "string" && message.executable) {
      executables.add(message.executable);
    }
  }
  if (executables.size !== 1) {
    throw new Error("Expected one executable for " + request.package + "/" + request.target.name
      + "; Cargo reported " + executables.size + ".");
  }
  return [...executables][0];
}

export function parseTestInventory(output) {
  const names = output.split(/\r?\n/).filter((line) => line.endsWith(": test"))
    .map((line) => line.slice(0, -6));
  if (new Set(names).size !== names.length) throw new Error("Duplicate test inventory entry.");
  return names;
}

function testNameMappings(transfers) {
  return transfers.flatMap((move) => [move, ...(move.testMappings ?? [])
    .map((mapping) => ({...mapping, owner:move.owner}))]);
}

function originalNames(name, prepared, transfers, explicit) {
  if (explicit) return [name];
  const previous = testNameMappings(transfers).filter((move) => move.owner === prepared.package
    && move.oldTestPrefix && move.newTestPrefix && name.startsWith(move.newTestPrefix))
    .map((move) => move.oldTestPrefix + name.slice(move.newTestPrefix.length));
  return prepared.package === "akra-ade" ? [name, ...previous] : previous;
}

function commonPrefix(names) {
  let prefix = names[0];
  for (const name of names.slice(1)) {
    while (!name.startsWith(prefix)) prefix = prefix.slice(0, -1);
  }
  return prefix;
}

/** Partition old filter selections by their actual package/target owner without broadening them. */
export function planContractFilters(prepared, filters, transfers = [], explicit = false) {
  const assignments = [];
  const selected = new Set();
  for (const filter of filters) {
    let matched = 0;
    for (const target of prepared) {
      const ignored = new Set(target.ignored);
      const names = target.inventory.filter((name) => !ignored.has(name)
        && originalNames(name, target, transfers, explicit).some((old) => old.includes(filter)));
      if (!names.length) continue;
      matched += names.length;
      for (const name of names) {
        const key = target.packageId + "\0" + target.target.kind + "\0" + target.target.name + "\0" + name;
        if (selected.has(key)) throw new Error("Overlapping native contract filters select " + name + " twice.");
        selected.add(key);
      }
      let args;
      if (names.length === 1) args = [names[0], "--exact", "--nocapture"];
      else {
        const candidates = [filter, commonPrefix(names)].filter(Boolean);
        const actual = candidates.find((candidate) => {
          const matchedNames = target.inventory.filter((name) => !ignored.has(name) && name.includes(candidate));
          return matchedNames.length === names.length && matchedNames.every((name) => names.includes(name));
        });
        if (!actual) throw new Error("Cannot preserve the process-per-filter boundary for " + filter + ".");
        args = [actual, "--nocapture"];
      }
      assignments.push({ ...target, filter, names, args });
    }
    if (!matched) throw new Error("Native contract filter matched no non-ignored tests: " + filter);
  }
  return assignments;
}

/** Complete each new member exactly once, including bin/integration harnesses. */
export function remainingMemberTests(prepared, assignments, {includeRoot = false, allowEmpty = false} = {}) {
  const remaining = [];
  for (const target of prepared) {
    if (!includeRoot && target.package === "akra-ade") continue;
    const consumed = assignments.filter((item) => item.packageId === target.packageId
      && item.target.kind === target.target.kind && item.target.name === target.target.name)
      .flatMap((item) => item.names);
    const ignored = new Set(target.ignored);
    const names = target.inventory.filter((name) => !ignored.has(name) && !consumed.includes(name));
    if (!names.length && (!allowEmpty || consumed.length)) continue;
    remaining.push({...target, filter:"member remainder", names,
      args:["--exact", "--nocapture", ...consumed.flatMap((name) => ["--skip", name])]});
  }
  return remaining;
}

const identity = (test) => [test.package, test.target.kind, test.target.name, test.name].join("\0");

/** Explicit additions and renames are required; ignored status must never change during a move. */
export function compareContractInventory(before, after, renames = [], additions = []) {
  const old = new Map(before.map((test) => [identity(test), test]));
  const next = new Map(after.map((test) => [identity(test), test]));
  if (old.size !== before.length || next.size !== after.length) throw new Error("Duplicate package/target inventory.");
  const moves = new Map();
  for (const move of renames) {
    const from = identity(move.from);
    if (!old.has(from) || moves.has(from)) throw new Error("Unknown or duplicate inventory rename.");
    moves.set(from, identity(move.to));
  }
  const expected = new Set();
  for (const [key, item] of old) {
    const moved = moves.get(key) ?? key;
    if (expected.has(moved)) throw new Error("Inventory rename collision.");
    expected.add(moved);
    if (!next.has(moved)) throw new Error("Missing test after extraction: " + item.name);
    if (Boolean(next.get(moved).ignored) !== Boolean(item.ignored)) throw new Error("Ignored state changed: " + item.name);
  }
  for (const addition of additions) {
    const key = identity(addition);
    if (expected.has(key) || !next.has(key)) throw new Error("Invalid inventory addition.");
    if (Boolean(next.get(key).ignored) !== Boolean(addition.ignored)) throw new Error("Added test ignored state differs.");
    expected.add(key);
  }
  if (expected.size !== next.size) throw new Error("Unregistered tests added during extraction.");
  return { before: before.length, after: after.length, renamed: renames.length, added: additions.length };
}
