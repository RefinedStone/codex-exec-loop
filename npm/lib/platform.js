export const PLATFORM_CONFIGS = [
  {
    nodePlatform: "linux",
    nodeArch: "x64",
    targetTriple: "x86_64-unknown-linux-gnu",
    packageAlias: "@refinedstone/akra-linux-x64",
    packageVersionSuffix: "linux-x64",
    binaryName: "codex-exec-loop-native",
    os: "linux",
    cpu: "x64",
  },
  {
    nodePlatform: "darwin",
    nodeArch: "arm64",
    targetTriple: "aarch64-apple-darwin",
    packageAlias: "@refinedstone/akra-darwin-arm64",
    packageVersionSuffix: "darwin-arm64",
    binaryName: "codex-exec-loop-native",
    os: "darwin",
    cpu: "arm64",
  },
  {
    nodePlatform: "win32",
    nodeArch: "x64",
    targetTriple: "x86_64-pc-windows-msvc",
    packageAlias: "@refinedstone/akra-win32-x64",
    packageVersionSuffix: "win32-x64",
    binaryName: "codex-exec-loop-native.exe",
    os: "win32",
    cpu: "x64",
  },
];

export function resolvePlatformConfig(
  platform = process.platform,
  arch = process.arch,
) {
  return (
    PLATFORM_CONFIGS.find(
      (config) =>
        config.nodePlatform === platform && config.nodeArch === arch,
    ) ?? null
  );
}

export function resolveConfigByTargetTriple(targetTriple) {
  return (
    PLATFORM_CONFIGS.find((config) => config.targetTriple === targetTriple) ??
    null
  );
}
