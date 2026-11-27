import { execFileSync } from "node:child_process";
import { createHash, timingSafeEqual } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
const MAX_TAR_OUTPUT_BYTES = 4 * 1024 * 1024;
export const TAR_TIMEOUT_MS = 120_000;

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

export { regularFileStat, descriptorRegularFileStat, sameFileVersion, openReadOnlyNoFollow, sha256File, runTar, readBoundedRegularFile, parseChecksum, createVerifiedSnapshot };
