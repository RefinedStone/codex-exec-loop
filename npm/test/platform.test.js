import test from "node:test";
import assert from "node:assert/strict";

import {
  PLATFORM_CONFIGS,
  resolveConfigByTargetTriple,
  resolvePlatformConfig,
} from "../lib/platform.js";

test("resolvePlatformConfig returns supported mappings", () => {
  assert.deepEqual(
    PLATFORM_CONFIGS.map((config) => [
      config.nodePlatform,
      config.nodeArch,
      config.targetTriple,
    ]),
    [
      ["linux", "x64", "x86_64-unknown-linux-gnu"],
      ["darwin", "arm64", "aarch64-apple-darwin"],
      ["win32", "x64", "x86_64-pc-windows-msvc"],
    ],
  );

  assert.equal(
    resolvePlatformConfig("linux", "x64")?.packageAlias,
    "@refinedstone/akra-linux-x64",
  );
  assert.equal(
    resolvePlatformConfig("darwin", "arm64")?.packageAlias,
    "@refinedstone/akra-darwin-arm64",
  );
  assert.equal(
    resolvePlatformConfig("win32", "x64")?.binaryName,
    "codex-exec-loop-native.exe",
  );
  assert.equal(resolvePlatformConfig("darwin", "x64"), null);
});

test("resolveConfigByTargetTriple finds published target", () => {
  assert.equal(
    resolveConfigByTargetTriple("x86_64-unknown-linux-gnu")?.cpu,
    "x64",
  );
  assert.equal(resolveConfigByTargetTriple("aarch64-pc-windows-msvc"), null);
});
