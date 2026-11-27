#!/usr/bin/env node
import { createHash, randomUUID } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  appendFileSync, closeSync, constants, fstatSync, futimesSync, lstatSync, mkdirSync, openSync,
  readFileSync, readSync, realpathSync, renameSync, rmSync, writeFileSync,
} from "node:fs";
import { dirname, isAbsolute, join, parse, relative, resolve, sep, win32 } from "node:path";
import { pathToFileURL } from "node:url";

const snapshotName = ".akra-ci-source-inputs-v1.json";
const sha256 = (value) => createHash("sha256").update(value).digest("hex");
const identity = (stat) => [stat.dev, stat.ino, stat.size, stat.mtimeNs, stat.ctimeNs].join(":");

function plainPath(path, allowMissing = false) {
  const absolute = resolve(path);
  const root = parse(absolute).root;
  let current = root;
  for (const part of absolute.slice(root.length).split(sep).filter(Boolean)) {
    current = join(current, part);
    let stat;
    try { stat = lstatSync(current); }
    catch (error) { if (allowMissing && error.code === "ENOENT") return; throw error; }
    if (stat.isSymbolicLink()) throw new Error("linked_path");
    if (current !== absolute && !stat.isDirectory()) throw new Error("non_directory_parent");
  }
}

function inputPath(repository, path) {
  if (typeof path !== "string" || !path || isAbsolute(path) || win32.isAbsolute(path)
    || /[\\:\0]/.test(path) || path.split("/").some((part) => ["", ".", "..", ".git"].includes(part))) {
    throw new Error("invalid_input_path");
  }
  const absolute = resolve(repository, path);
  const inside = relative(repository, absolute);
  if (!inside || inside.startsWith(".." + sep) || isAbsolute(inside)) throw new Error("outside_repository");
  plainPath(absolute);
  return absolute;
}

/** Keep an open identity while hashing and restoring, and never follow a source link. */
function openInput(repository, path) {
  const absolute = inputPath(repository, path);
  const before = lstatSync(absolute, { bigint: true });
  if (!before.isFile() || before.nlink !== 1n) throw new Error("non_regular_input");
  // Windows SetFileTime needs a writable handle; read-only inputs are safely skipped.
  const access = process.platform === "win32" ? constants.O_RDWR : constants.O_RDONLY;
  const descriptor = openSync(absolute, access | (constants.O_NOFOLLOW ?? 0));
  try {
    const stat = fstatSync(descriptor, { bigint: true });
    if (identity(before) !== identity(stat)) throw new Error("input_changed_during_open");
    const hash = createHash("sha256");
    const buffer = Buffer.allocUnsafe(64 * 1024);
    let count;
    while ((count = readSync(descriptor, buffer, 0, buffer.length, null)) > 0) hash.update(buffer.subarray(0, count));
    if (identity(stat) !== identity(fstatSync(descriptor, { bigint: true }))) throw new Error("input_changed_during_hash");
    return {
      descriptor, stat,
      input: { path, sha256: hash.digest("hex"), size: Number(stat.size),
        // Flooring avoids rounding a restored source later than its original timestamp.
        mtimeMs: Number(stat.mtimeNs / 1_000_000n), mode: Number(stat.mode & 0o777n),
        capturedIdentity: identity(stat) },
    };
  } catch (error) { closeSync(descriptor); throw error; }
}

export function trackedRegularPaths(repository) {
  const output = execFileSync("git", ["-C", repository, "ls-files", "--stage", "-z"],
    { encoding: "utf8", maxBuffer: 32 * 1024 * 1024,
      env: Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^GIT_/i.test(key))) });
  const paths = [];
  for (const entry of output.split("\0").filter(Boolean)) {
    const tab = entry.indexOf("\t");
    if (tab < 0) throw new Error("invalid_git_inventory");
    const [mode, , stage] = entry.slice(0, tab).split(" ");
    if (["100644", "100755"].includes(mode) && stage === "0") paths.push(entry.slice(tab + 1));
  }
  return [...new Set(paths)].sort();
}

function loadSnapshot(path, { repository, boundaryHash, now, pending = false }) {
  let snapshot;
  try {
    plainPath(path);
    const stat = lstatSync(path);
    if (!stat.isFile() || stat.nlink !== 1 || stat.size > 32 * 1024 * 1024) return { status: "invalid_snapshot" };
    snapshot = JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    return { status: error.code === "ENOENT" ? "missing_snapshot" : "invalid_snapshot" };
  }
  if (snapshot.schemaVersion !== 1 || !Array.isArray(snapshot.inputs)
    || !Number.isFinite(Date.parse(snapshot.capturedAt)) || Date.parse(snapshot.capturedAt) > now
    || !/^[0-9a-f]{40,64}$/i.test(snapshot.sourceRevision ?? "")) return { status: "invalid_snapshot" };
  if (snapshot.state !== (pending ? "pending" : "sealed")) return { status: pending ? "unprepared_snapshot" : "unsealed_snapshot" };
  if (snapshot.repository !== repository) return { status: "repository_path_changed" };
  if (snapshot.boundaryHash !== boundaryHash) return { status: "build_boundary_changed" };
  const inputs = new Map();
  for (const input of snapshot.inputs) {
    if (!input || typeof input.path !== "string" || inputs.has(input.path)
      || !/^[0-9a-f]{64}$/.test(input.sha256 ?? "")
      || !Number.isSafeInteger(input.size) || input.size < 0
      || !Number.isSafeInteger(input.mtimeMs) || input.mtimeMs < 0 || input.mtimeMs > Date.parse(snapshot.capturedAt)
      || !Number.isSafeInteger(input.mode) || input.mode < 0 || input.mode > 0o777
      || !/^\d+(?::\d+){4}$/.test(input.capturedIdentity ?? "")) return { status: "invalid_snapshot" };
    // Only current git-listed paths will ever be opened. Foreign snapshot entries are inert.
    inputs.set(input.path, input);
  }
  return { status: "matched_snapshot", snapshot, inputs };
}

function writeSnapshot(path, snapshot) {
  plainPath(dirname(path), true);
  mkdirSync(dirname(path), { recursive: true });
  const temporary = path + "." + randomUUID() + ".tmp";
  try {
    writeFileSync(temporary, JSON.stringify(snapshot) + "\n", { flag: "wx" });
    renameSync(temporary, path);
  } finally { rmSync(temporary, { force: true }); }
}

/**
 * Call after target restore and before the first Cargo command. The next snapshot therefore
 * describes pre-build inputs. A successful trusted push saves it with the compiled target.
 * This never edits Cargo's dep-info, fingerprint, outputs, contents, modes or directory mtimes.
 */
export function prepareCacheInputs({
  repository = process.cwd(), targetDirectory, boundary, sourceRevision,
  paths, now = Date.now(),
}) {
  if (!targetDirectory || !boundary || !/^[0-9a-f]{40,64}$/i.test(sourceRevision ?? "")) {
    throw new Error("missing_cache_input_boundary");
  }
  plainPath(repository);
  repository = realpathSync(repository);
  targetDirectory = resolve(targetDirectory);
  plainPath(targetDirectory, true);
  const snapshotPath = join(targetDirectory, snapshotName);
  const boundaryHash = sha256(JSON.stringify(boundary));
  const previous = loadSnapshot(snapshotPath, { repository, boundaryHash, now });
  const currentPaths = paths ?? trackedRegularPaths(repository);
  const inputs = [];
  const report = {
    schemaVersion: 1, capturedAt: new Date(now).toISOString(), sourceRevision,
    previousRevision: previous.snapshot?.sourceRevision ?? null, boundaryHash,
    status: previous.status, trackedFiles: currentPaths.length, restoredFiles: 0,
    unchangedFiles: 0, changedFiles: 0, newFiles: 0, skipped: [],
  };
  // Consume the old seal before changing timestamps. Interrupted preparation cannot publish old hints.
  writeSnapshot(snapshotPath, { schemaVersion: 1, state: "preparing", sourceRevision });
  for (const path of [...new Set(currentPaths)].sort()) {
    let opened;
    try {
      opened = openInput(repository, path);
      const previousInput = previous.inputs?.get(path);
      if (!previousInput) report.newFiles += 1;
      else if (previousInput.sha256 !== opened.input.sha256 || previousInput.size !== opened.input.size
        || previousInput.mode !== opened.input.mode) report.changedFiles += 1;
      else if (previousInput.mtimeMs === opened.input.mtimeMs) report.unchangedFiles += 1;
      else {
        // Restore only byte-identical regular inputs inside the same checkout/build boundary.
        // Preserve atime; futimes targets the verified descriptor, not a replaceable path.
        if (identity(opened.stat) !== identity(fstatSync(opened.descriptor, { bigint: true }))) {
          throw new Error("input_changed_during_restore");
        }
        futimesSync(opened.descriptor, Number(opened.stat.atimeNs) / 1e9, previousInput.mtimeMs / 1000);
        report.restoredFiles += 1;
        const restored = fstatSync(opened.descriptor, { bigint: true });
        opened.input.mtimeMs = Number(restored.mtimeNs / 1_000_000n);
        opened.input.capturedIdentity = identity(restored);
      }
      // Future-dated or concurrently changed inputs cannot justify a later cache restore.
      if (opened.input.mtimeMs <= now) inputs.push(opened.input);
      else report.skipped.push({ path, reason: "future_input_mtime" });
    } catch (error) {
      report.skipped.push({ path, reason: error.code || error.message });
    } finally { if (opened) closeSync(opened.descriptor); }
  }
  writeSnapshot(snapshotPath, {
    schemaVersion: 1, state: "pending", capturedAt: report.capturedAt, sourceRevision,
    repository, boundaryHash, inputs,
  });
  return report;
}

/** Publish reuse hints only when all captured inputs stayed unchanged through validation. */
export function sealCacheInputs({
  repository = process.cwd(), targetDirectory, boundary, sourceRevision, now = Date.now(),
}) {
  plainPath(repository);
  repository = realpathSync(repository);
  targetDirectory = resolve(targetDirectory);
  plainPath(targetDirectory);
  const snapshotPath = join(targetDirectory, snapshotName);
  const boundaryHash = sha256(JSON.stringify(boundary));
  const previous = loadSnapshot(snapshotPath, { repository, boundaryHash, now, pending: true });
  const report = { schemaVersion: 1, phase: "seal", sourceRevision, status: previous.status, changed: [] };
  if (!previous.snapshot) return report;
  if (previous.snapshot.sourceRevision !== sourceRevision) {
    return { ...report, status: "source_revision_changed" };
  }
  for (const input of previous.snapshot.inputs) {
    let opened;
    try {
      opened = openInput(repository, input.path);
      if (input.sha256 !== opened.input.sha256 || input.size !== opened.input.size
        || input.mode !== opened.input.mode || input.capturedIdentity !== opened.input.capturedIdentity) {
        report.changed.push(input.path);
      }
    } catch { report.changed.push(input.path); }
    finally { if (opened) closeSync(opened.descriptor); }
  }
  report.status = report.changed.length === 0 ? "sealed" : "inputs_changed_during_validation";
  writeSnapshot(snapshotPath, {
    ...previous.snapshot, state: report.changed.length === 0 ? "sealed" : "invalidated",
    sealedAt: new Date(now).toISOString(),
  });
  return report;
}

export function main(args, env = process.env) {
  if (args.length !== 1 || !["prepare", "seal"].includes(args[0])) throw new Error("usage: ci-cache-inputs.mjs <prepare|seal>");
  const directory = env.AKRA_CI_MEASUREMENTS || "artifacts/ci-performance";
  mkdirSync(directory, { recursive: true });
  let report;
  try {
    const policy = JSON.parse(readFileSync(join(directory, "cache-policy.json"), "utf8"));
    const operation = args[0] === "prepare" ? prepareCacheInputs : sealCacheInputs;
    report = operation({
      repository: process.cwd(), targetDirectory: policy.target.paths[0],
      boundary: policy.boundary, sourceRevision: policy.sourceRevision,
    });
  } catch (error) {
    // Optional cache preparation must not replace the following Cargo validation.
    report = { schemaVersion: 1, status: "disabled", reason: error.code || error.message };
  }
  writeFileSync(join(directory, "cache-inputs-" + args[0] + ".json"), JSON.stringify(report, null, 2) + "\n");
  if (args[0] === "seal" && env.GITHUB_OUTPUT) {
    appendFileSync(env.GITHUB_OUTPUT, "sealed=" + String(report.status === "sealed") + "\n");
  }
  console.log(JSON.stringify({ ...report, skipped: report.skipped?.length }));
  if (report.status === "disabled") console.warn("Source timestamp reuse disabled; Cargo will validate ordinary checkout inputs.");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main(process.argv.slice(2));
