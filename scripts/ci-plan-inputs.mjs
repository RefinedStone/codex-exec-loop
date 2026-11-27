#!/usr/bin/env node

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  closeSync, constants, existsSync, fstatSync, lstatSync, mkdtempSync, openSync,
  readFileSync, readSync, realpathSync, rmSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { parseGitNameStatusZ } from "./ci-impact-plan.mjs";

const sha256 = (value) => createHash("sha256").update(value).digest("hex");
const digest = (value) => sha256(JSON.stringify(value));
const slash = (value) => value.replaceAll("\\", "/");
const comparablePath = (value) => process.platform === "win32" ? slash(value).toLowerCase() : slash(value);
function cleanGitEnvironment(environment) {
  return Object.fromEntries(Object.entries(environment).filter(([key]) => !key.toUpperCase().startsWith("GIT_")));
}
function git(cwd, args, environment) {
  return execFileSync("git", ["--no-optional-locks", "-C", cwd, ...args], {
    env: cleanGitEnvironment(environment), encoding: "buffer", maxBuffer: 128 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });
}
function utf8(buffer) {
  const text = buffer.toString("utf8");
  if (!Buffer.from(text, "utf8").equals(buffer)) throw new Error("non-UTF-8 Git paths cannot be planned safely");
  return text;
}
function nulFields(buffer) {
  const fields = utf8(buffer).split("\0");
  if (fields.pop() !== "") throw new Error("expected a NUL-terminated Git inventory");
  return fields;
}
// Git diff/status deliberately ignore these tracked entries. Do not allow a
// partially visible worktree to produce either a selective or clean CI plan.
function assertVisibleTrackedInputs(cwd, environment) {
  for (const entry of nulFields(git(cwd, ["ls-files", "-v", "-z"], environment))) {
    if (!/^[A-Za-z?] /.test(entry)) throw new Error("invalid Git visibility inventory");
    const flag = entry[0];
    if (flag === "S" || flag === flag.toLowerCase()) {
      throw new Error("validation refuses assume-unchanged or skip-worktree index entries: " + entry.slice(2));
    }
  }
}
function identity(stat) {
  return [stat.dev, stat.ino, stat.size, stat.mtimeNs, stat.ctimeNs].map(String).join(":");
}
function safeRelativePath(path) {
  if (!path || isAbsolute(path) || /^[A-Za-z]:/.test(path) || path.includes("\0")
      || path.split(/[\\/]/).some((part) => part === ".." || part === "." || part === "")) {
    throw new Error("unsafe input path: " + path);
  }
  return slash(path);
}
function assertNoSymlinkParents(root, path) {
  let current = root;
  for (const part of path.split("/").slice(0, -1)) {
    current = join(current, part);
    if (!existsSync(current)) return;
    const stat = lstatSync(current);
    if (stat.isSymbolicLink() || !stat.isDirectory()) throw new Error("input parent is not an ordinary directory: " + path);
  }
}
function fileRecord(root, input) {
  const path = safeRelativePath(input);
  assertNoSymlinkParents(root, path);
  const absolute = join(root, path);
  let initial;
  try { initial = lstatSync(absolute, { bigint: true }); } catch (error) {
    if (error.code === "ENOENT") return { path, type: "missing" };
    throw error;
  }
  if (!initial.isFile() || initial.isSymbolicLink()) throw new Error("input is not an ordinary file: " + path);
  const descriptor = openSync(absolute, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fstatSync(descriptor, { bigint: true });
    if (identity(before) !== identity(initial)) throw new Error("input changed while opening: " + path);
    const hash = createHash("sha256"), buffer = Buffer.allocUnsafe(64 * 1024);
    let size = 0;
    for (;;) {
      const count = readSync(descriptor, buffer, 0, buffer.length, null);
      if (count === 0) break;
      hash.update(buffer.subarray(0, count));
      size += count;
    }
    const after = fstatSync(descriptor, { bigint: true });
    if (identity(before) !== identity(after) || BigInt(size) !== after.size) throw new Error("input changed while hashing: " + path);
    return { path, type: "file", executable: (Number(after.mode) & 0o111) !== 0, bytes: size, sha256: hash.digest("hex") };
  } finally {
    closeSync(descriptor);
  }
}
function readOwnership(cwd) {
  try { return JSON.parse(readFileSync(join(cwd, "scripts/ci-crate-ownership.json"), "utf8")); } catch { return null; }
}
function registeredInputs(ownership) {
  if (!ownership || !Array.isArray(ownership.packages)) return [];
  const array = (value) => Array.isArray(value) ? value : [];
  const inputs = [
    ...array(ownership.fullInputs),
    ...ownership.packages.flatMap((entry) => [...array(entry.sourcePrefixes), ...array(entry.externalInputs), ...array(entry.generatedInputs)]),
    ...array(ownership.contracts).flatMap((entry) => array(entry.inputs)),
    ".cargo/config", ".cargo/config.toml", "rust-toolchain", "rust-toolchain.toml",
  ];
  return [...new Set(inputs.filter((path) => typeof path === "string"))];
}
function extraRegisteredFiles(ownership) {
  return registeredInputs(ownership).filter((path) => !path.endsWith("/"));
}
function validationOutputPrefixes(ownership, tracked) {
  const declarations = ownership?.validationOutputs ?? [];
  if (!Array.isArray(declarations)) throw new Error("validationOutputs must be an array");
  // Reviewed install outputs only. An arbitrary registry prefix cannot hide source bytes.
  const allowed = new Map([
    ["assets/admin/game/node_modules/", ["assets/admin/game/package.json", "assets/admin/game/package-lock.json"]],
  ]);
  const overlaps = (input, output) => input.startsWith(output)
    || (input.endsWith("/") && output.startsWith(input));
  return declarations.map((entry) => {
    const inputs = allowed.get(entry?.path);
    if (!inputs || entry.kind !== "npm-ci-install"
        || entry.manifest !== inputs[0] || entry.lockfile !== inputs[1]
        || !inputs.every((path) => tracked.has(path))) {
      throw new Error("unregistered validation output or missing tracked manifest/lock");
    }
    const packages = ownership.packages ?? [], contracts = ownership.contracts ?? [];
    const strictInputs = [
      ...(ownership.fullInputs ?? []),
      ...packages.flatMap((pkg) => [...(pkg.sourcePrefixes ?? []), ...(pkg.generatedInputs ?? [])]),
      ...contracts.flatMap((contract) => contract.inputs ?? []),
    ];
    // A broad external assets/ prefix can contain npm outputs. Explicit inputs cannot.
    const explicitInputs = packages.flatMap((pkg) => pkg.externalInputs ?? [])
      .filter((input) => !input.endsWith("/") || input.startsWith(entry.path));
    if ([...strictInputs, ...explicitInputs].some((input) => overlaps(input, entry.path))) {
      throw new Error("validation output overlaps explicitly registered source/contract inputs");
    }
    return entry.path;
  });
}
function ignoredRegisteredFiles(cwd, ownership, environment, tracked) {
  const outputs = validationOutputPrefixes(ownership, tracked);
  const prefixes = registeredInputs(ownership).filter((path) => path.endsWith("/"))
    .map((path) => ":(literal)" + safeRelativePath(path.slice(0, -1)) + "/");
  return prefixes.length ? nulFields(git(cwd, ["ls-files", "--others", "--ignored", "--exclude-standard", "-z", "--", ...prefixes], environment))
    .map(safeRelativePath).filter((path) => !outputs.some((output) => path.startsWith(output))) : [];
}
function cargoConfigurationFiles(cwd, environment) {
  const paths = new Set();
  for (let current = cwd;;) {
    for (const name of ["config", "config.toml"]) paths.add(join(current, ".cargo", name));
    const parent = dirname(current);
    if (parent === current) break;
    current = parent;
  }
  const cargoHome = environment.CARGO_HOME ? resolve(cwd, environment.CARGO_HOME) : join(homedir(), ".cargo");
  for (const name of ["config", "config.toml"]) paths.add(join(cargoHome, name));
  return [...paths].sort().filter(existsSync).map((path) => {
    const resolved = realpathSync(path), root = dirname(resolved);
    const entry = fileRecord(root, relative(root, resolved));
    return { path: slash(path), resolved: slash(resolved), sha256: entry.sha256 };
  });
}
function hasAncestorConfiguration(cwd, environment, files) {
  const root = comparablePath(resolve(cwd)) + "/";
  const cargoHome = comparablePath(environment.CARGO_HOME ? resolve(cwd, environment.CARGO_HOME) : join(homedir(), ".cargo")) + "/";
  return files.some((file) => !comparablePath(file.path).startsWith(root) && !comparablePath(file.path).startsWith(cargoHome));
}
function buildEnvironment(environment) {
  return Object.entries(environment)
    .filter(([key]) => /^(?:CARGO_|RUST|CC(?:_|$)|CXX(?:_|$)|AR(?:_|$)|CFLAGS$|CXXFLAGS$|CPPFLAGS$|LDFLAGS$|LIB$|LIBPATH$|INCLUDE$|SDKROOT$|MACOSX_DEPLOYMENT_TARGET$|PATH$|PATHEXT$|HOME$|USERPROFILE$|TMPDIR$|TMP$|TEMP$|SYSTEMROOT$|COMSPEC$|VCTOOLS|VSINSTALLDIR$|VCINSTALLDIR$|WINDOWSSDK|NPM_|NODE_|ELECTRON_|PLAYWRIGHT_)/i.test(key))
    .map(([key, value]) => [key, String(value)])
    .sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0);
}
function toolchainIdentity(cwd, environment) {
  const run = (command, args) => execFileSync(command, args, {
    cwd, env: environment, encoding: "utf8", maxBuffer: 1024 * 1024, stdio: ["ignore", "pipe", "pipe"],
  }).trim();
  return { cargo: run("cargo", ["--version"]), rustc: run(environment.RUSTC || "rustc", ["-vV"]), node: process.version };
}

/** Content identity, including index blobs, source files and registered ignored inputs. */
export function capturePlanInputs({
  cwd = process.cwd(), environment = process.env, toolchain = toolchainIdentity(cwd, environment),
} = {}) {
  const sourceSha = utf8(git(cwd, ["rev-parse", "--verify", "HEAD^{commit}"], environment)).trim();
  assertVisibleTrackedInputs(cwd, environment);
  const stage = nulFields(git(cwd, ["ls-files", "--stage", "-z"], environment));
  const index = stage.map((record) => {
    const match = /^([0-7]{6}) ([a-f0-9]{40,64}) ([0-3])\t([\s\S]+)$/.exec(record);
    if (!match) throw new Error("invalid Git index entry");
    if (match[1] === "160000") throw new Error("submodule contents need an explicit validation input policy");
    return { mode: match[1], blob: match[2], stage: Number(match[3]), path: safeRelativePath(match[4]) };
  }).sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : a.stage - b.stage);
  const untracked = nulFields(git(cwd, ["ls-files", "--others", "--exclude-standard", "-z"], environment)).map(safeRelativePath);
  const ownership = readOwnership(cwd);
  const indexPaths = new Set(index.map((entry) => entry.path)), untrackedPaths = new Set(untracked);
  const ignored = ignoredRegisteredFiles(cwd, ownership, environment, indexPaths);
  const paths = [...new Set([...indexPaths, ...untracked, ...ignored, ...extraRegisteredFiles(ownership)])].sort();
  const files = paths.map((path) => fileRecord(cwd, path));
  const configurationFiles = cargoConfigurationFiles(cwd, environment);
  const body = { sourceSha, index, files, configurationFiles, toolchain, environment: buildEnvironment(environment) };
  return {
    sourceSha, localChangeId: "sha256:" + digest(body),
    indexIdentity: digest(index), filesIdentity: digest(files),
    environmentIdentity: digest({ configurationFiles, toolchain, environment: body.environment }),
    hasAncestorConfiguration: hasAncestorConfiguration(cwd, environment, configurationFiles),
    untracked,
    externalChanges: files.filter((file) => file.type === "file" && !indexPaths.has(file.path) && !untrackedPaths.has(file.path)).map((file) => file.path),
  };
}
function readMetadata(cwd, { environment, offline }) {
  const args = ["metadata", "--format-version=1", "--locked", "--all-features"];
  if (offline) args.push("--offline");
  return JSON.parse(execFileSync("cargo", args, {
    cwd, env: environment, encoding: "utf8", maxBuffer: 128 * 1024 * 1024, stdio: ["ignore", "pipe", "pipe"],
  }));
}
function archiveSnapshot(cwd, sha, environment) {
  const directory = mkdtempSync(join(tmpdir(), "akra-plan-base-")), archive = join(directory, "snapshot.tar");
  try {
    git(cwd, ["archive", "--format=tar", "--output", archive, sha], environment);
    // Relative archive path avoids GNU tar interpreting a Windows drive as a host.
    execFileSync("tar", ["-xf", "snapshot.tar"], {
      cwd: directory, env: cleanGitEnvironment(environment), stdio: ["ignore", "pipe", "pipe"],
    });
    return { cwd: directory, cleanup: () => rmSync(directory, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 }) };
  } catch (error) {
    rmSync(directory, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 });
    throw error;
  }
}
function contentChanges(cwd, baseSha, state, environment) {
  const commands = [
    ...(baseSha ? [["diff", "--no-ext-diff", "--no-textconv", "--name-status", "-z", "--find-renames", baseSha, "HEAD", "--"]] : []),
    ["diff", "--no-ext-diff", "--no-textconv", "--cached", "--name-status", "-z", "--find-renames", "--"],
    ["diff", "--no-ext-diff", "--no-textconv", "--name-status", "-z", "--find-renames", "--"],
  ];
  const changes = commands.flatMap((args) => parseGitNameStatusZ(git(cwd, args, environment)));
  changes.push(...state.untracked.map((path) => ({ status: "A", path })));
  changes.push(...state.externalChanges.map((path) => ({ status: "M", path })));
  return [...new Map(changes.map((change) => [JSON.stringify(change), change])).entries()]
    .sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([, change]) => change);
}
export function hostConfiguration(platform = process.platform) {
  const platforms = { win32: "windows", darwin: "macos", linux: "linux" };
  if (!platforms[platform]) throw new Error("unsupported local validation platform: " + platform);
  return { platforms: [platforms[platform]], features: ["all", "default"], profile: "test", targetTriples: [] };
}

/** CI binds source to Git; ignored registered inputs must not bypass clean status. */
export function assertCleanPlanCheckout({ cwd = process.cwd(), environment = process.env } = {}) {
  const repoRoot = utf8(git(cwd, ["rev-parse", "--show-toplevel"], environment)).trim();
  const ownership = readOwnership(repoRoot);
  if (!ownership || !Array.isArray(ownership.packages)) throw new Error("cannot verify immutable checkout ownership");
  assertVisibleTrackedInputs(repoRoot, environment);
  const status = git(repoRoot, ["status", "--porcelain", "-z", "--untracked-files=all"], environment);
  if (status.length !== 0) throw new Error("CI validation requires a clean tracked and untracked checkout");
  const tracked = new Set(nulFields(git(repoRoot, ["ls-files", "--cached", "-z"], environment)));
  const unversioned = [...new Set([
    ...extraRegisteredFiles(ownership).filter((path) => !tracked.has(path) && existsSync(join(repoRoot, safeRelativePath(path)))),
    ...ignoredRegisteredFiles(repoRoot, ownership, environment, tracked),
  ])].sort();
  if (unversioned.length) throw new Error("CI validation refuses registered inputs outside Git, including ignored files: " + unversioned.join(", "));
  return { sourceSha: utf8(git(repoRoot, ["rev-parse", "--verify", "HEAD^{commit}"], environment)).trim(), clean: true };
}

/**
 * Collects a planner request without building or modifying the checkout/index.
 * services are injectable for small real-Git fixtures; CLI users cannot supply them.
 */
export function collectPlanInput({
  cwd = process.cwd(), baseRef = "origin/prerelease", baseSha, headSha,
  configuration = hostConfiguration(), offline = true, requireCleanCheckout = false, environment: providedEnvironment = process.env,
} = {}, services = {}) {
  if (typeof offline !== "boolean" || typeof requireCleanCheckout !== "boolean") throw new Error("offline and requireCleanCheckout must be boolean");
  const environment = services.environment ?? providedEnvironment;
  const metadata = services.readMetadata ?? readMetadata, archive = services.archiveSnapshot ?? archiveSnapshot;
  const toolchain = (services.toolchainIdentity ?? toolchainIdentity)(cwd, environment);
  const repoRoot = utf8(git(cwd, ["rev-parse", "--show-toplevel"], environment)).trim();
  if (requireCleanCheckout) assertCleanPlanCheckout({ cwd: repoRoot, environment });
  const first = capturePlanInputs({ cwd: repoRoot, environment, toolchain });
  if (headSha !== undefined && headSha !== first.sourceSha) throw new Error("checkout HEAD differs from requested source SHA");
  let resolvedBase = null;
  try {
    if (baseSha !== undefined) {
      if (typeof baseSha !== "string" || !/^[a-f0-9]{40,64}$/i.test(baseSha)) throw new Error("baseSha must identify a commit");
      resolvedBase = utf8(git(repoRoot, ["rev-parse", "--verify", baseSha + "^{commit}"], environment)).trim();
    } else {
      if (typeof baseRef !== "string" || !baseRef || baseRef.startsWith("-")) throw new Error("invalid base ref");
      resolvedBase = utf8(git(repoRoot, ["merge-base", baseRef, first.sourceSha], environment)).trim();
    }
  } catch {
    // A missing base is represented explicitly. The planner will select full.
  }
  const changes = contentChanges(repoRoot, resolvedBase, first, environment);
  const currentMetadata = metadata(repoRoot, { environment, offline });
  if (comparablePath(realpathSync(currentMetadata.workspace_root)) !== comparablePath(realpathSync(repoRoot))) {
    throw new Error("Cargo workspace root must match the repository root for validation planning");
  }
  const currentOwnership = readOwnership(repoRoot);
  let base = resolvedBase ? { sha: resolvedBase, metadata: null, ownership: null } : null;
  if (resolvedBase) {
    let temporary;
    try {
      if (first.hasAncestorConfiguration) throw new Error("base Cargo ancestor configuration cannot be reproduced independently");
      temporary = archive(repoRoot, resolvedBase, environment);
      if (hasAncestorConfiguration(temporary.cwd, environment, cargoConfigurationFiles(temporary.cwd, environment))) {
        throw new Error("temporary base inherits an unrelated Cargo ancestor configuration");
      }
      const lockPath = join(temporary.cwd, "Cargo.lock");
      const beforeLock = existsSync(lockPath) ? sha256(readFileSync(lockPath)) : null;
      const baseMetadata = metadata(temporary.cwd, { environment, offline });
      const afterLock = existsSync(lockPath) ? sha256(readFileSync(lockPath)) : null;
      if (beforeLock !== afterLock) throw new Error("base metadata modified Cargo.lock");
      base = { sha: resolvedBase, metadata: baseMetadata, ownership: readOwnership(temporary.cwd) };
    } catch {
      // Offline cache misses or an unavailable old registry cannot narrow validation.
    } finally {
      temporary?.cleanup();
    }
  }
  const second = capturePlanInputs({ cwd: repoRoot, environment, toolchain });
  if (first.localChangeId !== second.localChangeId) throw new Error("validation inputs changed while collecting metadata; retry planning");
  if (requireCleanCheckout) assertCleanPlanCheckout({ cwd: repoRoot, environment });
  return {
    schemaVersion: 1, base, head: { sha: first.sourceSha, metadata: currentMetadata, ownership: currentOwnership },
    changes, localChangeId: first.localChangeId, configuration,
  };
}
