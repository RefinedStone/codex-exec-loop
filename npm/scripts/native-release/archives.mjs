import { timingSafeEqual } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { PLATFORM_CONFIGS } from "../../lib/platform.js";
import { buildBundleContract } from "./contract.mjs";
import { regularFileStat, descriptorRegularFileStat, sameFileVersion, openReadOnlyNoFollow, sha256File, runTar, parseChecksum, createVerifiedSnapshot } from "./files.mjs";
import { inspectArchiveMembers } from "./tar.mjs";
import { verifyExtractedBundle } from "./tree.mjs";
const MAX_ARCHIVE_BYTES = 512 * 1024 * 1024;

export async function findAndVerifyArchive(
  releaseAssetsDir,
  version,
  targetTriple,
  profile = "release",
) {
  const contract = buildBundleContract(version, targetTriple, profile);
  return verifyArchiveForContract(releaseAssetsDir, contract);
}

export async function verifyArchiveForContract(releaseAssetsDir, contract) {
  const expectedName = `${contract.expectedRoot}.tar.gz`;
  const archivePath = path.join(releaseAssetsDir, expectedName);
  const checksumPath = `${archivePath}.sha256`;
  const before = regularFileStat(archivePath, "release archive");
  if (before.size <= 0n || before.size > BigInt(MAX_ARCHIVE_BYTES)) {
    throw new Error(`Release archive size is outside the allowed range: ${archivePath}`);
  }
  const expectedDigest = parseChecksum(checksumPath, expectedName);
  if (contract.archiveDigest !== null &&
      expectedDigest.toString("hex") !== contract.archiveDigest) {
    throw new Error("Historical release checksum does not match the pinned published artifact");
  }
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
