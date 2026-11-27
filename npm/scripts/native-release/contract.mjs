import { createHash } from "node:crypto";
import { TextDecoder } from "node:util";
import { resolveConfigByTargetTriple } from "../../lib/platform.js";

const SAFE_MEMBER_PATH = /^[A-Za-z0-9._@+-]+(?:\/[A-Za-z0-9._@+-]+)*\/?$/;
const UTF8_DECODER = new TextDecoder("utf-8", { fatal: true });
const MAX_VERSION_METADATA_BYTES = 4096;
const MAX_BUNDLE_MANIFEST_BYTES = 4 * 1024 * 1024;

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
    format: "current",
    archiveDigest: null,
    name: "akra-ade",
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
  const expected = new Map([
    ["name", contract.name],
    ["version", contract.version],
    ["target", contract.targetTriple],
    ["profile", contract.profile],
    ["binary", contract.binaryName],
    ["launcher", contract.launcherName],
  ]);
  switch (contract.format) {
    case "current":
      expected.set("release_tag", contract.releaseTag);
      break;
    case "published-windows-1.3.5":
      expected.set("launcher", "akra");
      break;
    default:
      throw new Error("Unsupported release contract");
  }
  const lines = decodeCanonicalLines(
    buffer,
    "Release VERSION.txt",
    MAX_VERSION_METADATA_BYTES,
  );
  const values = new Map();
  for (const line of lines) {
    const match = /^([a-z_]+)=([^\s=]+)$/.exec(line);
    if (!match || !expected.has(match[1]) || values.has(match[1])) {
      throw new Error("Release VERSION.txt has an invalid, duplicate, or unexpected key");
    }
    values.set(match[1], match[2]);
  }
  if (
    values.size !== expected.size ||
    [...expected.keys()].some((key) => !values.has(key))
  ) {
    throw new Error("Release VERSION.txt does not contain the exact required key set");
  }
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
  // A self-consistent manifest can still omit mandatory license files. Refuse
  // incomplete release payloads before extraction, npm staging or publication.
  switch (contract.format) {
    case "current": {
      const emptyDigest = createHash("sha256").update(Buffer.alloc(0)).digest("hex");
      if (!fileDigests.has("LICENSE") || fileDigests.get("LICENSE") === emptyDigest) {
        throw new Error("Project license must be a nonempty regular file: LICENSE");
      }
      for (const notice of ["Galmuri-OFL.txt", "PixiJS-MIT.txt", "rust-crates.txt", "README.md"]) {
        const relativePath = "THIRD_PARTY_NOTICES/" + notice;
        if (!fileDigests.has(relativePath)) {
          throw new Error("Required third-party notice is missing: " + relativePath);
        }
        if (fileDigests.get(relativePath) === emptyDigest) {
          throw new Error("Required third-party notice must be a nonempty regular file: " + relativePath);
        }
      }
      break;
    }
    case "published-windows-1.3.5":
      // Historical input only: the archive reader requires its pinned release digest.
      break;
    default:
      throw new Error("Unsupported release contract");
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


function archiveName(version, targetTriple) {
  return `akra-ade-${normalizeVersion(version)}-${targetTriple}.tar.gz`;
}


export { buildBundleContract, archiveName, verifyBundleRecords, validateManifestPath, UTF8_DECODER, SAFE_MEMBER_PATH, MAX_VERSION_METADATA_BYTES, MAX_BUNDLE_MANIFEST_BYTES };
