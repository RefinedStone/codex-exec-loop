#!/usr/bin/env node
import path from "node:path";
import { pathToFileURL } from "node:url";
import { buildBundleContract } from "./native-release/contract.mjs";
import { findAndVerifyArchive, verifyReleaseAssets, cleanupVerifiedReleaseAssets } from "./native-release/archives.mjs";
import { verifyExtractedBundle } from "./native-release/tree.mjs";
export { findAndVerifyArchive, verifyReleaseAssets, cleanupVerifiedReleaseAssets, extractVerifiedArchive } from "./native-release/archives.mjs";
export { verifyExtractedBundle, assertSafeExtractedTree } from "./native-release/tree.mjs";

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
