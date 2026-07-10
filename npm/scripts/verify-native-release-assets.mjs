#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { createHash, timingSafeEqual } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pipeline } from "node:stream/promises";
import { Writable } from "node:stream";
import { TextDecoder } from "node:util";
import { pathToFileURL } from "node:url";
import { createGunzip } from "node:zlib";

import {
  PLATFORM_CONFIGS,
  resolveConfigByTargetTriple,
} from "../lib/platform.js";

const MAX_ARCHIVE_BYTES = 512 * 1024 * 1024;
const MAX_ARCHIVE_MEMBERS = 4096;
const MAX_EXPANDED_BYTES = 1024 * 1024 * 1024;
const MAX_TAR_STREAM_BYTES =
  MAX_EXPANDED_BYTES + MAX_ARCHIVE_MEMBERS * 1024 + 1024 * 1024;
const MAX_TAR_OUTPUT_BYTES = 4 * 1024 * 1024;
const MAX_COMPRESSION_RATIO = 256n;
const COMPRESSION_RATIO_ALLOWANCE_BYTES = 16n * 1024n * 1024n;
const TAR_BLOCK_BYTES = 512;
const TAR_TIMEOUT_MS = 120_000;
const SAFE_MEMBER_PATH = /^[A-Za-z0-9._@+-]+(?:\/[A-Za-z0-9._@+-]+)*\/?$/;
const UTF8_DECODER = new TextDecoder("utf-8", { fatal: true });
const MAX_VERSION_METADATA_BYTES = 4096;
const MAX_BUNDLE_MANIFEST_BYTES = 4 * 1024 * 1024;
const VERSION_KEYS = new Set([
  "name",
  "version",
  "release_tag",
  "target",
  "profile",
  "binary",
  "launcher",
]);

function normalizeVersion(version) {
  return version.startsWith("v") ? version.slice(1) : version;
}

function buildBundleContract(version, targetTriple, profile = "release") {
  const normalizedVersion = normalizeVersion(version);
  const config = resolveConfigByTargetTriple(targetTriple);
  if (!config) {
    throw new Error(`Unsupported native release target: ${targetTriple}`);
  }
  if (!/^[0-9A-Za-z][0-9A-Za-z.+-]*$/.test(normalizedVersion)) {
    throw new Error(`Invalid native release version: ${version}`);
  }
  if (!new Set(["release", "debug"]).has(profile)) {
    throw new Error(`Invalid native release profile: ${profile}`);
  }
  return {
    name: "codex-exec-loop-native",
    version: normalizedVersion,
    releaseTag: `v${normalizedVersion}`,
    targetTriple,
    profile,
    binaryName: config.binaryName,
    launcherName: config.os === "win32" ? "akra.cmd" : "akra",
    expectedRoot: archiveName(normalizedVersion, targetTriple).slice(
      0,
      -".tar.gz".length,
    ),
  };
}

function decodeCanonicalLines(buffer, label, maximumBytes) {
  if (buffer.length === 0 || buffer.length > maximumBytes) {
    throw new Error(`${label} must be nonempty and no larger than ${maximumBytes} bytes`);
  }
  let body;
  try {
    body = UTF8_DECODER.decode(buffer);
  } catch {
    throw new Error(`${label} must be valid UTF-8`);
  }
  if (!body.endsWith("\n") || body.includes("\r") || body.includes("\0")) {
    throw new Error(`${label} must use canonical LF-terminated text`);
  }
  const lines = body.slice(0, -1).split("\n");
  if (lines.some((line) => line.length === 0)) {
    throw new Error(`${label} must not contain empty lines`);
  }
  return lines;
}

function parseVersionMetadata(buffer, contract) {
  const lines = decodeCanonicalLines(
    buffer,
    "Release VERSION.txt",
    MAX_VERSION_METADATA_BYTES,
  );
  const values = new Map();
  for (const line of lines) {
    const match = /^([a-z_]+)=([^\s=]+)$/.exec(line);
    if (!match || !VERSION_KEYS.has(match[1]) || values.has(match[1])) {
      throw new Error("Release VERSION.txt has an invalid, duplicate, or unexpected key");
    }
    values.set(match[1], match[2]);
  }
  if (
    values.size !== VERSION_KEYS.size ||
    [...VERSION_KEYS].some((key) => !values.has(key))
  ) {
    throw new Error("Release VERSION.txt does not contain the exact required key set");
  }
  const expected = new Map([
    ["name", contract.name],
    ["version", contract.version],
    ["release_tag", contract.releaseTag],
    ["target", contract.targetTriple],
    ["profile", contract.profile],
    ["binary", contract.binaryName],
    ["launcher", contract.launcherName],
  ]);
  for (const [key, expectedValue] of expected) {
    if (values.get(key) !== expectedValue) {
      throw new Error(
        `Release VERSION.txt ${key} mismatch: expected ${expectedValue}`,
      );
    }
  }
  return values;
}

function validateManifestPath(relativePath) {
  if (
    !SAFE_MEMBER_PATH.test(relativePath) ||
    relativePath.endsWith("/") ||
    Buffer.byteLength(relativePath) > 240 ||
    relativePath.split("/").length > 32 ||
    relativePath
      .split("/")
      .some((component) => component === "." || component === "..")
  ) {
    throw new Error(`Release SHA256SUMS.txt has an unsafe path: ${relativePath}`);
  }
}

function parseBundleManifest(buffer) {
  const lines = decodeCanonicalLines(
    buffer,
    "Release SHA256SUMS.txt",
    MAX_BUNDLE_MANIFEST_BYTES,
  );
  const entries = new Map();
  for (const line of lines) {
    const match = /^([0-9a-f]{64})  ([A-Za-z0-9._@+-]+(?:\/[A-Za-z0-9._@+-]+)*)$/.exec(
      line,
    );
    if (!match) {
      throw new Error("Release SHA256SUMS.txt has invalid grammar");
    }
    const relativePath = match[2];
    validateManifestPath(relativePath);
    if (relativePath === "SHA256SUMS.txt") {
      throw new Error("Release SHA256SUMS.txt must not include itself");
    }
    if (entries.has(relativePath)) {
      throw new Error(
        `Release SHA256SUMS.txt repeats a path: ${relativePath}`,
      );
    }
    entries.set(relativePath, match[1]);
  }
  if (entries.size === 0) {
    throw new Error("Release SHA256SUMS.txt must contain at least one entry");
  }
  return entries;
}

function verifyBundleRecords(fileDigests, metadataBodies, contract) {
  for (const requiredPath of [
    contract.binaryName,
    contract.launcherName,
    "VERSION.txt",
    "SHA256SUMS.txt",
  ]) {
    if (!fileDigests.has(requiredPath)) {
      throw new Error(`Release bundle is missing required file: ${requiredPath}`);
    }
  }
  parseVersionMetadata(metadataBodies.get("VERSION.txt") ?? Buffer.alloc(0), contract);
  const manifest = parseBundleManifest(
    metadataBodies.get("SHA256SUMS.txt") ?? Buffer.alloc(0),
  );
  const requiredFiles = [...fileDigests.keys()].filter(
    (relativePath) => relativePath !== "SHA256SUMS.txt",
  );
  if (
    manifest.size !== requiredFiles.length ||
    requiredFiles.some((relativePath) => !manifest.has(relativePath))
  ) {
    throw new Error(
      "Release SHA256SUMS.txt must cover every regular bundle file exactly once",
    );
  }
  for (const [relativePath, expectedDigest] of manifest) {
    const actualDigest = fileDigests.get(relativePath);
    if (actualDigest === undefined || actualDigest !== expectedDigest) {
      throw new Error(`Release bundle checksum mismatch: ${relativePath}`);
    }
  }
  return manifest;
}

function regularFileStat(filePath, label) {
  const stat = fs.lstatSync(filePath, { bigint: true });
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1n) {
    throw new Error(`${label} must be a single-link regular file: ${filePath}`);
  }
  return stat;
}

function descriptorRegularFileStat(descriptor, filePath, label) {
  const stat = fs.fstatSync(descriptor, { bigint: true });
  if (!stat.isFile() || stat.nlink !== 1n) {
    throw new Error(`${label} must remain a single-link regular file: ${filePath}`);
  }
  return stat;
}

function sameFileVersion(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.size === right.size &&
    left.mtimeNs === right.mtimeNs &&
    left.ctimeNs === right.ctimeNs
  );
}

function openReadOnlyNoFollow(filePath) {
  return fs.openSync(
    filePath,
    fs.constants.O_RDONLY | (fs.constants.O_NOFOLLOW ?? 0),
  );
}

function sha256File(filePath) {
  const before = regularFileStat(filePath, "release snapshot");
  const hash = createHash("sha256");
  const buffer = Buffer.allocUnsafe(1024 * 1024);
  const descriptor = openReadOnlyNoFollow(filePath);
  try {
    const opened = descriptorRegularFileStat(
      descriptor,
      filePath,
      "release snapshot",
    );
    if (!sameFileVersion(before, opened)) {
      throw new Error(`Release snapshot changed before it could be read: ${filePath}`);
    }
    for (;;) {
      const bytesRead = fs.readSync(descriptor, buffer, 0, buffer.length, null);
      if (bytesRead === 0) break;
      hash.update(buffer.subarray(0, bytesRead));
    }
    const after = descriptorRegularFileStat(
      descriptor,
      filePath,
      "release snapshot",
    );
    if (!sameFileVersion(opened, after)) {
      throw new Error(`Release snapshot changed while it was being read: ${filePath}`);
    }
  } finally {
    fs.closeSync(descriptor);
  }
  const afterPath = regularFileStat(filePath, "release snapshot");
  if (!sameFileVersion(before, afterPath)) {
    throw new Error(`Release snapshot path changed while it was being read: ${filePath}`);
  }
  return hash.digest();
}

function cleanTarEnvironment() {
  const environment = { ...process.env, LC_ALL: "C", TZ: "UTC" };
  for (const variable of [
    "GZIP",
    "POSIXLY_CORRECT",
    "TAR_OPTIONS",
    "TAR_READER_OPTIONS",
    "TAR_WRITER_OPTIONS",
  ]) {
    delete environment[variable];
  }
  return environment;
}

function runTar(args, inputDescriptor = "ignore") {
  return execFileSync("tar", args, {
    encoding: "utf8",
    env: cleanTarEnvironment(),
    maxBuffer: MAX_TAR_OUTPUT_BYTES,
    stdio: [inputDescriptor, "pipe", "pipe"],
    timeout: TAR_TIMEOUT_MS,
    windowsHide: true,
  });
}

function archiveName(version, targetTriple) {
  return `codex-exec-loop-native-${normalizeVersion(version)}-${targetTriple}.tar.gz`;
}

function readBoundedRegularFile(filePath, label, maximumBytes) {
  const before = regularFileStat(filePath, label);
  if (before.size > BigInt(maximumBytes)) {
    throw new Error(`${label} is unexpectedly large: ${filePath}`);
  }
  const descriptor = openReadOnlyNoFollow(filePath);
  try {
    const opened = descriptorRegularFileStat(descriptor, filePath, label);
    if (!sameFileVersion(before, opened)) {
      throw new Error(`${label} changed before it could be read: ${filePath}`);
    }
    const body = fs.readFileSync(descriptor);
    const after = descriptorRegularFileStat(descriptor, filePath, label);
    if (!sameFileVersion(opened, after)) {
      throw new Error(`${label} changed while it was being read: ${filePath}`);
    }
    const afterPath = regularFileStat(filePath, label);
    if (!sameFileVersion(before, afterPath)) {
      throw new Error(`${label} path changed while it was being read: ${filePath}`);
    }
    return body;
  } finally {
    fs.closeSync(descriptor);
  }
}

function parseChecksum(checksumPath, expectedArchiveName) {
  const body = readBoundedRegularFile(
    checksumPath,
    "release checksum",
    512,
  ).toString("utf8");
  const match = /^([0-9a-f]{64})  ([A-Za-z0-9._@+-]+)\n?$/.exec(body);
  if (!match || match[2] !== expectedArchiveName) {
    throw new Error(`Release checksum has an invalid or mismatched entry: ${checksumPath}`);
  }
  return Buffer.from(match[1], "hex");
}

function writeAll(descriptor, buffer, length) {
  let offset = 0;
  while (offset < length) {
    offset += fs.writeSync(descriptor, buffer, offset, length - offset);
  }
}

function createVerifiedSnapshot(archivePath, expectedName, expectedDigest, before) {
  const snapshotDirectory = fs.mkdtempSync(
    path.join(os.tmpdir(), "akra-release-verify-"),
  );
  fs.chmodSync(snapshotDirectory, 0o700);
  const snapshotPath = path.join(snapshotDirectory, expectedName);
  let sourceDescriptor;
  let destinationDescriptor;
  let completed = false;

  try {
    sourceDescriptor = openReadOnlyNoFollow(archivePath);
    const opened = descriptorRegularFileStat(
      sourceDescriptor,
      archivePath,
      "release archive",
    );
    if (!sameFileVersion(before, opened)) {
      throw new Error(`Release archive changed before it could be snapshotted: ${expectedName}`);
    }

    destinationDescriptor = fs.openSync(
      snapshotPath,
      fs.constants.O_WRONLY | fs.constants.O_CREAT | fs.constants.O_EXCL,
      0o600,
    );
    const hash = createHash("sha256");
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    for (;;) {
      const bytesRead = fs.readSync(
        sourceDescriptor,
        buffer,
        0,
        buffer.length,
        null,
      );
      if (bytesRead === 0) break;
      hash.update(buffer.subarray(0, bytesRead));
      writeAll(destinationDescriptor, buffer, bytesRead);
    }
    fs.fsyncSync(destinationDescriptor);

    const sourceAfter = descriptorRegularFileStat(
      sourceDescriptor,
      archivePath,
      "release archive",
    );
    const pathAfter = regularFileStat(archivePath, "release archive");
    if (
      !sameFileVersion(opened, sourceAfter) ||
      !sameFileVersion(before, pathAfter)
    ) {
      throw new Error(`Release archive changed while it was being snapshotted: ${expectedName}`);
    }

    const actualDigest = hash.digest();
    if (!timingSafeEqual(expectedDigest, actualDigest)) {
      throw new Error(`Release archive checksum mismatch: ${expectedName}`);
    }
    completed = true;
  } finally {
    if (destinationDescriptor !== undefined) fs.closeSync(destinationDescriptor);
    if (sourceDescriptor !== undefined) fs.closeSync(sourceDescriptor);
    if (!completed) {
      fs.rmSync(snapshotDirectory, { recursive: true, force: true });
    }
  }

  fs.chmodSync(snapshotPath, 0o400);
  regularFileStat(snapshotPath, "release snapshot");
  return { snapshotDirectory, snapshotPath };
}

function parseTarOctal(field, label) {
  const nul = field.indexOf(0);
  const body = field
    .subarray(0, nul === -1 ? field.length : nul)
    .toString("ascii")
    .trim();
  if (!/^[0-7]+$/.test(body)) {
    throw new Error(`Release archive has an invalid ${label} field`);
  }
  const value = Number.parseInt(body, 8);
  if (!Number.isSafeInteger(value)) {
    throw new Error(`Release archive ${label} exceeds the supported integer range`);
  }
  return value;
}

function parseTarString(field, label) {
  const nul = field.indexOf(0);
  const body = field.subarray(0, nul === -1 ? field.length : nul);
  if (nul !== -1 && field.subarray(nul + 1).some((byte) => byte !== 0)) {
    throw new Error(`Release archive has ambiguous ${label} padding`);
  }
  try {
    return UTF8_DECODER.decode(body);
  } catch {
    throw new Error(`Release archive has a non-UTF-8 ${label}`);
  }
}

function verifyTarHeaderChecksum(header) {
  const expected = parseTarOctal(header.subarray(148, 156), "header checksum");
  let actual = 0;
  for (let index = 0; index < header.length; index += 1) {
    actual += index >= 148 && index < 156 ? 0x20 : header[index];
  }
  if (actual !== expected) {
    throw new Error("Release archive has an invalid tar header checksum");
  }
}

class TarStreamInspector extends Writable {
  constructor(contract, maximumStreamBytes) {
    super();
    this.contract = contract;
    this.expectedRoot = contract.expectedRoot;
    this.maximumStreamBytes = maximumStreamBytes;
    this.header = Buffer.alloc(TAR_BLOCK_BYTES);
    this.headerBytes = 0;
    this.payloadBytes = 0;
    this.paddingBytes = 0;
    this.streamBytes = 0;
    this.expandedBytes = 0;
    this.memberCount = 0;
    this.zeroBlocks = 0;
    this.finished = false;
    this.rootSeen = false;
    this.uniqueNames = new Set();
    this.fileDigests = new Map();
    this.metadataBodies = new Map();
    this.currentFile = null;
    this.manifest = null;
  }

  _write(chunk, _encoding, callback) {
    try {
      this.streamBytes += chunk.length;
      if (this.streamBytes > this.maximumStreamBytes) {
        throw new Error(
          `Release archive exceeds the bounded tar stream or compression-ratio limit (${this.maximumStreamBytes} bytes)`,
        );
      }
      this.consume(chunk);
      callback();
    } catch (error) {
      callback(error);
    }
  }

  _final(callback) {
    try {
      if (
        !this.finished ||
        this.headerBytes !== 0 ||
        this.payloadBytes !== 0 ||
        this.paddingBytes !== 0
      ) {
        throw new Error("Release archive ended before a complete ustar stream terminator");
      }
      if (
        this.memberCount === 0 ||
        this.memberCount > MAX_ARCHIVE_MEMBERS ||
        !this.rootSeen ||
        this.currentFile !== null
      ) {
        throw new Error("Release archive has an invalid member layout");
      }
      this.manifest = verifyBundleRecords(
        this.fileDigests,
        this.metadataBodies,
        this.contract,
      );
      callback();
    } catch (error) {
      callback(error);
    }
  }

  consume(chunk) {
    let offset = 0;
    while (offset < chunk.length) {
      if (this.finished) {
        if (chunk.subarray(offset).some((byte) => byte !== 0)) {
          throw new Error("Release archive contains data after its ustar terminator");
        }
        return;
      }
      if (this.payloadBytes > 0) {
        const consumed = Math.min(this.payloadBytes, chunk.length - offset);
        const payload = chunk.subarray(offset, offset + consumed);
        this.currentFile.hash.update(payload);
        if (this.currentFile.chunks !== null) {
          this.currentFile.chunks.push(Buffer.from(payload));
        }
        this.payloadBytes -= consumed;
        offset += consumed;
        if (this.payloadBytes === 0) this.finishCurrentFile();
        continue;
      }
      if (this.paddingBytes > 0) {
        const consumed = Math.min(this.paddingBytes, chunk.length - offset);
        if (chunk.subarray(offset, offset + consumed).some((byte) => byte !== 0)) {
          throw new Error("Release archive has nonzero file padding");
        }
        this.paddingBytes -= consumed;
        offset += consumed;
        continue;
      }

      const copied = Math.min(
        TAR_BLOCK_BYTES - this.headerBytes,
        chunk.length - offset,
      );
      chunk.copy(this.header, this.headerBytes, offset, offset + copied);
      this.headerBytes += copied;
      offset += copied;
      if (this.headerBytes === TAR_BLOCK_BYTES) {
        this.inspectHeader();
        this.headerBytes = 0;
      }
    }
  }

  inspectHeader() {
    if (this.header.every((byte) => byte === 0)) {
      this.zeroBlocks += 1;
      if (this.zeroBlocks === 2) this.finished = true;
      return;
    }
    if (this.zeroBlocks !== 0) {
      throw new Error("Release archive has an interrupted ustar terminator");
    }

    verifyTarHeaderChecksum(this.header);
    if (this.header.subarray(257, 262).toString("ascii") !== "ustar") {
      throw new Error("Release archive must use the bounded ustar format");
    }

    const name = parseTarString(this.header.subarray(0, 100), "member name");
    const prefix = parseTarString(this.header.subarray(345, 500), "member prefix");
    const member = prefix ? `${prefix}/${name}` : name;
    const type = this.header[156];
    const isFile = type === 0 || type === 0x30;
    const isDirectory = type === 0x35;
    if (!isFile && !isDirectory) {
      throw new Error(`Release archive contains a non-file member: ${member}`);
    }

    const size = parseTarOctal(this.header.subarray(124, 136), "member size");
    if (isDirectory && size !== 0) {
      throw new Error(`Release archive directory has a payload: ${member}`);
    }
    this.expandedBytes += size;
    if (this.expandedBytes > MAX_EXPANDED_BYTES) {
      throw new Error("Release archive expands beyond the allowed size");
    }

    if (!SAFE_MEMBER_PATH.test(member)) {
      throw new Error(`Release archive has an unsafe member path: ${member}`);
    }
    const normalized = member.endsWith("/") ? member.slice(0, -1) : member;
    const components = normalized.split("/");
    if (
      Buffer.byteLength(normalized) > 240 ||
      components.length > 32 ||
      components.some((component) => component === "." || component === "..") ||
      components[0] !== this.expectedRoot ||
      (normalized === this.expectedRoot && !isDirectory) ||
      (normalized !== this.expectedRoot && components.length < 2) ||
      (isFile && member.endsWith("/"))
    ) {
      throw new Error(`Release archive member escapes the expected root: ${member}`);
    }
    if (this.uniqueNames.has(normalized)) {
      throw new Error(`Release archive repeats a member path: ${member}`);
    }
    this.uniqueNames.add(normalized);
    if (normalized === this.expectedRoot) this.rootSeen = true;

    this.memberCount += 1;
    if (this.memberCount > MAX_ARCHIVE_MEMBERS) {
      throw new Error("Release archive has too many members");
    }
    this.payloadBytes = size;
    this.paddingBytes =
      size === 0 ? 0 : (TAR_BLOCK_BYTES - (size % TAR_BLOCK_BYTES)) % TAR_BLOCK_BYTES;
    if (isFile) {
      const relativePath = components.slice(1).join("/");
      const captureLimit =
        relativePath === "VERSION.txt"
          ? MAX_VERSION_METADATA_BYTES
          : relativePath === "SHA256SUMS.txt"
            ? MAX_BUNDLE_MANIFEST_BYTES
            : null;
      if (captureLimit !== null && size > captureLimit) {
        throw new Error(
          `Release ${relativePath} is unexpectedly large (${size} bytes)`,
        );
      }
      this.currentFile = {
        relativePath,
        hash: createHash("sha256"),
        chunks: captureLimit === null ? null : [],
      };
      if (size === 0) this.finishCurrentFile();
    }
  }

  finishCurrentFile() {
    if (this.currentFile === null) {
      throw new Error("Release archive payload is not bound to a regular file");
    }
    const { relativePath, hash, chunks } = this.currentFile;
    this.fileDigests.set(relativePath, hash.digest("hex"));
    if (chunks !== null) {
      this.metadataBodies.set(relativePath, Buffer.concat(chunks));
    }
    this.currentFile = null;
  }
}

async function inspectArchiveMembers(archivePath, contract, compressedSize) {
  const ratioLimit =
    compressedSize * MAX_COMPRESSION_RATIO + COMPRESSION_RATIO_ALLOWANCE_BYTES;
  const maximumStreamBytes = Number(
    ratioLimit < BigInt(MAX_TAR_STREAM_BYTES)
      ? ratioLimit
      : BigInt(MAX_TAR_STREAM_BYTES),
  );
  const before = regularFileStat(archivePath, "release snapshot");
  const descriptor = openReadOnlyNoFollow(archivePath);
  const opened = descriptorRegularFileStat(
    descriptor,
    archivePath,
    "release snapshot",
  );
  if (!sameFileVersion(before, opened)) {
    fs.closeSync(descriptor);
    throw new Error(`Release snapshot changed before inspection: ${archivePath}`);
  }

  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), TAR_TIMEOUT_MS);
  timeout.unref();
  const inspector = new TarStreamInspector(contract, maximumStreamBytes);
  try {
    await pipeline(
      fs.createReadStream(archivePath, { fd: descriptor, autoClose: true }),
      createGunzip(),
      inspector,
      { signal: controller.signal },
    );
  } catch (error) {
    if (error?.name === "AbortError") {
      throw new Error(`Release archive inspection exceeded ${TAR_TIMEOUT_MS}ms`);
    }
    throw error;
  } finally {
    clearTimeout(timeout);
  }

  const after = regularFileStat(archivePath, "release snapshot");
  if (!sameFileVersion(before, after)) {
    throw new Error(`Release snapshot changed while it was being inspected: ${archivePath}`);
  }
  return {
    fileDigests: inspector.fileDigests,
    manifest: inspector.manifest,
  };
}

export async function findAndVerifyArchive(
  releaseAssetsDir,
  version,
  targetTriple,
  profile = "release",
) {
  const contract = buildBundleContract(version, targetTriple, profile);
  const expectedName = archiveName(version, targetTriple);
  const archivePath = path.join(releaseAssetsDir, expectedName);
  const checksumPath = `${archivePath}.sha256`;
  const before = regularFileStat(archivePath, "release archive");
  if (before.size <= 0n || before.size > BigInt(MAX_ARCHIVE_BYTES)) {
    throw new Error(`Release archive size is outside the allowed range: ${archivePath}`);
  }
  const expectedDigest = parseChecksum(checksumPath, expectedName);
  const { snapshotDirectory, snapshotPath } = createVerifiedSnapshot(
    archivePath,
    expectedName,
    expectedDigest,
    before,
  );
  try {
    const inspected = await inspectArchiveMembers(
      snapshotPath,
      contract,
      before.size,
    );
    return {
      archivePath,
      checksumPath,
      digest: expectedDigest,
      expectedRoot: contract.expectedRoot,
      snapshotDirectory,
      snapshotPath,
      contract,
      manifest: inspected.manifest,
      fileDigests: inspected.fileDigests,
    };
  } catch (error) {
    fs.rmSync(snapshotDirectory, { recursive: true, force: true });
    throw error;
  }
}

export function cleanupVerifiedReleaseAssets(verified) {
  const directories = new Set(
    [...verified.values()].map((entry) => entry.snapshotDirectory),
  );
  for (const directory of directories) {
    fs.rmSync(directory, { recursive: true, force: true });
  }
}

export async function verifyReleaseAssets(
  releaseAssetsDir,
  version,
  profile = "release",
) {
  const expectedFiles = new Set();
  const verified = new Map();
  try {
    for (const config of PLATFORM_CONFIGS) {
      const result = await findAndVerifyArchive(
        releaseAssetsDir,
        version,
        config.targetTriple,
        profile,
      );
      expectedFiles.add(path.basename(result.archivePath));
      expectedFiles.add(path.basename(result.checksumPath));
      verified.set(config.targetTriple, result);
    }

    const actualFiles = fs.readdirSync(releaseAssetsDir);
    if (
      actualFiles.length !== expectedFiles.size ||
      actualFiles.some((entry) => !expectedFiles.has(entry))
    ) {
      throw new Error("Release assets contain missing, duplicate, nested, or unexpected payloads");
    }
    return verified;
  } catch (error) {
    cleanupVerifiedReleaseAssets(verified);
    throw error;
  }
}

export function extractVerifiedArchive(verified, outputDir) {
  const before = regularFileStat(verified.snapshotPath, "release snapshot");
  fs.mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  const descriptor = openReadOnlyNoFollow(verified.snapshotPath);
  try {
    const opened = descriptorRegularFileStat(
      descriptor,
      verified.snapshotPath,
      "release snapshot",
    );
    if (!sameFileVersion(before, opened)) {
      throw new Error(
        `Release snapshot changed before it could be extracted: ${verified.archivePath}`,
      );
    }
    runTar(
      [
        "--extract",
        "--gzip",
        "--file",
        "-",
        "--directory",
        outputDir,
        "--no-same-owner",
        "--no-same-permissions",
      ],
      descriptor,
    );
    const descriptorAfter = descriptorRegularFileStat(
      descriptor,
      verified.snapshotPath,
      "release snapshot",
    );
    if (!sameFileVersion(opened, descriptorAfter)) {
      throw new Error(
        `Release snapshot changed while it was being extracted: ${verified.archivePath}`,
      );
    }
  } finally {
    fs.closeSync(descriptor);
  }
  const after = regularFileStat(verified.snapshotPath, "release snapshot");
  if (!sameFileVersion(before, after)) {
    throw new Error(`Release snapshot changed while it was being extracted: ${verified.archivePath}`);
  }
  const finalDigest = sha256File(verified.snapshotPath);
  if (!timingSafeEqual(verified.digest, finalDigest)) {
    throw new Error(`Release snapshot digest changed while it was being extracted: ${verified.archivePath}`);
  }

  const entries = fs.readdirSync(outputDir, { withFileTypes: true });
  if (
    entries.length !== 1 ||
    entries[0].name !== verified.expectedRoot ||
    !entries[0].isDirectory() ||
    entries[0].isSymbolicLink()
  ) {
    throw new Error(`Expected exactly the verified root directory after extracting ${verified.archivePath}`);
  }
  const root = path.join(outputDir, entries[0].name);
  verifyExtractedBundle(
    root,
    verified.contract,
    verified.manifest,
    verified.fileDigests,
  );
  return root;
}

export function assertSafeExtractedTree(root) {
  const visit = (entryPath) => {
    const stat = fs.lstatSync(entryPath, { bigint: true });
    if (stat.isSymbolicLink() || (!stat.isDirectory() && !stat.isFile())) {
      throw new Error(`Extracted release tree contains an unsafe object: ${entryPath}`);
    }
    if (stat.isFile()) {
      if (stat.nlink !== 1n) {
        throw new Error(
          `Extracted release tree contains a multiply linked file: ${entryPath}`,
        );
      }
      return;
    }
    for (const entry of fs.readdirSync(entryPath)) {
      visit(path.join(entryPath, entry));
    }
  };
  visit(root);
}

function manifestsMatch(left, right) {
  return (
    left.size === right.size &&
    [...left].every(
      ([relativePath, digest]) => right.get(relativePath) === digest,
    )
  );
}

export function verifyExtractedBundle(
  root,
  contract,
  expectedManifest = null,
  expectedFileDigests = null,
) {
  const resolvedRoot = path.resolve(root);
  if (path.basename(resolvedRoot) !== contract.expectedRoot) {
    throw new Error(
      `Release bundle root mismatch: expected ${contract.expectedRoot}`,
    );
  }
  const rootStat = fs.lstatSync(resolvedRoot, { bigint: true });
  if (!rootStat.isDirectory() || rootStat.isSymbolicLink()) {
    throw new Error(`Release bundle root must be a real directory: ${resolvedRoot}`);
  }

  const fileDigests = new Map();
  const metadataBodies = new Map();
  const visit = (directory) => {
    for (const entry of fs.readdirSync(directory)) {
      const entryPath = path.join(directory, entry);
      const relativePath = path
        .relative(resolvedRoot, entryPath)
        .split(path.sep)
        .join("/");
      validateManifestPath(relativePath);
      const stat = fs.lstatSync(entryPath, { bigint: true });
      if (stat.isDirectory() && !stat.isSymbolicLink()) {
        visit(entryPath);
        continue;
      }
      if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1n) {
        throw new Error(`Extracted release tree contains an unsafe object: ${entryPath}`);
      }
      if (relativePath === "VERSION.txt") {
        const body = readBoundedRegularFile(
          entryPath,
          "Release VERSION.txt",
          MAX_VERSION_METADATA_BYTES,
        );
        metadataBodies.set(relativePath, body);
        fileDigests.set(
          relativePath,
          createHash("sha256").update(body).digest("hex"),
        );
      } else if (relativePath === "SHA256SUMS.txt") {
        const body = readBoundedRegularFile(
          entryPath,
          "Release SHA256SUMS.txt",
          MAX_BUNDLE_MANIFEST_BYTES,
        );
        metadataBodies.set(relativePath, body);
        fileDigests.set(
          relativePath,
          createHash("sha256").update(body).digest("hex"),
        );
      } else {
        fileDigests.set(relativePath, sha256File(entryPath).toString("hex"));
      }
    }
  };
  visit(resolvedRoot);

  const manifest = verifyBundleRecords(fileDigests, metadataBodies, contract);
  if (expectedManifest !== null && !manifestsMatch(manifest, expectedManifest)) {
    throw new Error(
      "Extracted release manifest differs from the verified archive manifest",
    );
  }
  if (
    expectedFileDigests !== null &&
    !manifestsMatch(fileDigests, expectedFileDigests)
  ) {
    throw new Error(
      "Extracted release files differ from the verified archive payloads",
    );
  }
  return manifest;
}

function parseArgs(argv) {
  const args = {
    releaseAssetsDir: "",
    bundleDir: "",
    version: "",
    targetTriple: "",
    profile: "release",
  };
  const seen = new Set();
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const keys = new Map([
      ["--release-assets-dir", "releaseAssetsDir"],
      ["--bundle-dir", "bundleDir"],
      ["--version", "version"],
      ["--target", "targetTriple"],
      ["--profile", "profile"],
    ]);
    const key = keys.get(argument);
    if (key === undefined) throw new Error(`Unsupported option: ${argument}`);
    if (seen.has(argument)) throw new Error(`Duplicate option: ${argument}`);
    const value = argv[++index];
    if (value === undefined || value.length === 0) {
      throw new Error(`Missing value for ${argument}`);
    }
    args[key] = value;
    seen.add(argument);
  }
  if (!args.version || (!args.releaseAssetsDir && !args.bundleDir)) {
    throw new Error(
      "--version and at least one of --release-assets-dir or --bundle-dir are required",
    );
  }
  if (args.bundleDir && !args.targetTriple) {
    throw new Error("--target is required with --bundle-dir");
  }
  return args;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.releaseAssetsDir) {
    const releaseAssetsDir = path.resolve(process.cwd(), args.releaseAssetsDir);
    const verified = args.targetTriple
      ? new Map([
          [
            args.targetTriple,
            await findAndVerifyArchive(
              releaseAssetsDir,
              args.version,
              args.targetTriple,
              args.profile,
            ),
          ],
        ])
      : await verifyReleaseAssets(
          releaseAssetsDir,
          args.version,
          args.profile,
        );
    try {
      console.log(`verified_release_assets=${releaseAssetsDir}`);
    } finally {
      cleanupVerifiedReleaseAssets(verified);
    }
  }
  if (args.bundleDir) {
    const bundleDir = path.resolve(process.cwd(), args.bundleDir);
    const contract = buildBundleContract(
      args.version,
      args.targetTriple,
      args.profile,
    );
    verifyExtractedBundle(bundleDir, contract);
    console.log(`verified_release_bundle=${bundleDir}`);
  }
}

const invokedPath = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : "";
if (import.meta.url === invokedPath) {
  main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
}
