import { verifyArchiveForContract } from "./archives.mjs";

// This historical input is never eligible for npm staging or publication.
// Digest: GitHub v1.3.5 asset 452661989 and its independently downloaded sidecar.
export function findAndVerifyLegacyArchive(directory, version, target) {
  if (version.replace(/^v/, "") !== "1.3.5" || target !== "x86_64-pc-windows-msvc") {
    throw new Error("Legacy smoke currently supports only the published Windows x64 v1.3.5 archive");
  }
  return verifyArchiveForContract(directory, {
    format: "published-windows-1.3.5",
    name: "codex-exec-loop-native",
    version: "1.3.5",
    targetTriple: target,
    profile: "release",
    binaryName: "codex-exec-loop-native.exe",
    launcherName: "akra.cmd",
    expectedRoot: `codex-exec-loop-native-1.3.5-${target}`,
    archiveDigest: "496097730bc8b7cb37d6d79759456bc68bf47d5995eaad8e029a1d741ca01eaa",
  });
}
