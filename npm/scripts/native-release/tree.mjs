import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { validateManifestPath, verifyBundleRecords, MAX_VERSION_METADATA_BYTES, MAX_BUNDLE_MANIFEST_BYTES } from "./contract.mjs";
import { readBoundedRegularFile, sha256File } from "./files.mjs";

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
