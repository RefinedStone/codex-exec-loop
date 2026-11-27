#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const posixPath = (value) => value.replaceAll("\\", "/");

export function validateCrateOwnership(registry, metadata, workspaceRoot = metadata.workspace_root) {
  const errors = [];
  if (registry.schemaVersion !== 1 || !Array.isArray(registry.packages)) {
    return ["Unsupported crate ownership registry."];
  }
  const declared = new Map(registry.packages.map((owner) => [owner.name, owner]));
  if (declared.size !== registry.packages.length || !declared.has(registry.rootPackage)) {
    errors.push("Package owners must be unique and include the root package.");
  }
  const ids = new Set(metadata.workspace_members ?? []);
  const members = (metadata.packages ?? []).filter((pkg) => ids.has(pkg.id));
  if (!members.length || members.length !== ids.size) errors.push("Workspace metadata is incomplete.");
  const membersByName = new Map(members.map((pkg) => [pkg.name, pkg]));
  for (const pkg of members) {
    const owner = declared.get(pkg.name);
    if (!owner) { errors.push("Unregistered workspace member: " + pkg.name); continue; }
    if (owner.state !== "active") errors.push("Member is not activated in ownership registry: " + pkg.name);
    const manifest = posixPath(relative(workspaceRoot, pkg.manifest_path));
    if (manifest !== owner.manifest) errors.push("Manifest owner mismatch: " + pkg.name + " at " + manifest);
    for (const dep of pkg.dependencies ?? []) {
      const workspaceDependency = membersByName.get(dep.name);
      if (!workspaceDependency) continue;
      if (pkg.name !== registry.rootPackage && dep.name === registry.rootPackage) {
        errors.push(pkg.name + " must not depend on root through " + (dep.kind ?? "normal") + " dependency.");
      }
      if (!owner.workspaceDependencies.includes(dep.name)) {
        errors.push("Forbidden workspace edge: " + pkg.name + " -> " + dep.name + " (" + (dep.kind ?? "normal") + ").");
      }
    }
  }
  for (const owner of declared.values()) {
    if (owner.state === "active" && !membersByName.has(owner.name)) {
      errors.push("Active owner is absent from Cargo workspace: " + owner.name);
    }
    if (!["active", "planned"].includes(owner.state)) errors.push("Invalid owner state: " + owner.name);
    for (const dep of owner.workspaceDependencies ?? []) {
      if (!declared.has(dep)) errors.push("Unknown allowed dependency: " + dep);
    }
  }
  const contracts = new Set();
  for (const contract of registry.contracts ?? []) {
    if (!contract.id || contracts.has(contract.id)) errors.push("Contract IDs must be nonempty and unique.");
    contracts.add(contract.id);
    if (!declared.has(contract.owner)) errors.push("Unregistered contract owner: " + contract.owner);
    if (!contract.filter || !contract.target?.name || !contract.target?.kind) errors.push("Incomplete contract: " + contract.id);
  }
  const transfers = new Set();
  for (const transfer of registry.transfers ?? []) {
    if (transfers.has(transfer.source)) errors.push("Duplicate transfer source: " + transfer.source);
    transfers.add(transfer.source);
    if (!declared.has(transfer.owner)) errors.push("Unregistered transfer owner: " + transfer.owner);
    if (transfer.ignored !== "preserve") errors.push("Transfer must preserve ignored tests: " + transfer.source);
    const prefix = /^[\p{ID_Start}_][\p{ID_Continue}_]*(?:::[\p{ID_Start}_][\p{ID_Continue}_]*)*(?:::)?$/u;
    const mappings = [];
    if (transfer.oldTestPrefix !== undefined || transfer.newTestPrefix !== undefined) mappings.push(transfer);
    if (transfer.testMappings !== undefined) {
      if (!Array.isArray(transfer.testMappings) || transfer.testMappings.length === 0) {
        errors.push("Additional test mappings must be a nonempty array: " + transfer.source);
      } else {
        mappings.push(...transfer.testMappings);
      }
    }
    const seenMappings = new Set();
    for (const mapping of mappings) {
      if (!mapping || typeof mapping.oldTestPrefix !== "string" || typeof mapping.newTestPrefix !== "string"
          || mapping.oldTestPrefix.trim() !== mapping.oldTestPrefix || mapping.newTestPrefix.trim() !== mapping.newTestPrefix
          || !prefix.test(mapping.oldTestPrefix) || !prefix.test(mapping.newTestPrefix)) {
        errors.push("Invalid test prefix mapping: " + transfer.source);
        continue;
      }
      const key = mapping.oldTestPrefix + "\0" + mapping.newTestPrefix;
      if (seenMappings.has(key)) errors.push("Duplicate test prefix mapping: " + transfer.source);
      seenMappings.add(key);
    }
  }
  return errors;
}

function main() {
  const args = process.argv.slice(2);
  const metadataIndex = args.indexOf("--metadata");
  const registry = JSON.parse(readFileSync(resolve(root, "scripts/ci-crate-ownership.json"), "utf8"));
  const metadata = metadataIndex >= 0
    ? JSON.parse(readFileSync(args[metadataIndex + 1], "utf8"))
    : JSON.parse(execFileSync("cargo", ["metadata", "--locked", "--format-version", "1", "--all-features"], {
      cwd: root, encoding: "utf8", maxBuffer: 32 * 1024 * 1024,
    }));
  const failures = validateCrateOwnership(registry, metadata);
  if (failures.length) throw new Error(failures.join("\n"));
  console.log("Crate ownership accepted every workspace member and dependency kind.");
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
