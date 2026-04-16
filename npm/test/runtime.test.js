import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { detectPackageManager, resolveBinaryPath } from "../lib/runtime.js";

test("detectPackageManager recognizes bun hints", () => {
  assert.equal(
    detectPackageManager({
      env: { npm_config_user_agent: "bun/1.2.0 npm/? node/22.0.0" },
    }),
    "bun",
  );
  assert.equal(
    detectPackageManager({
      env: { npm_config_user_agent: "npm/11.6.1 node/v22.16.0" },
    }),
    "npm",
  );
  assert.equal(detectPackageManager({ env: {} }), null);
});

test("resolveBinaryPath uses local vendor fallback", () => {
  const packageRoot = fs.mkdtempSync(path.join(os.tmpdir(), "akra-runtime-"));
  const binaryPath = path.join(
    packageRoot,
    "vendor",
    "x86_64-unknown-linux-gnu",
    "akra",
    "codex-exec-loop-native",
  );
  fs.mkdirSync(path.dirname(binaryPath), { recursive: true });
  fs.writeFileSync(binaryPath, "");

  const resolved = resolveBinaryPath({
    packageRoot,
    platform: "linux",
    arch: "x64",
    resolvePackageJson: () => {
      throw new Error("missing optional dependency");
    },
  });

  assert.equal(resolved.binaryPath, binaryPath);
});

test("resolveBinaryPath prefers installed optional dependency", () => {
  const packageRoot = fs.mkdtempSync(path.join(os.tmpdir(), "akra-runtime-"));
  const packageJsonPath = path.join(
    packageRoot,
    "node_modules",
    "@refinedstone",
    "akra-linux-x64",
    "package.json",
  );
  const binaryPath = path.join(
    path.dirname(packageJsonPath),
    "vendor",
    "x86_64-unknown-linux-gnu",
    "akra",
    "codex-exec-loop-native",
  );

  fs.mkdirSync(path.dirname(binaryPath), { recursive: true });
  fs.writeFileSync(packageJsonPath, "{}");
  fs.writeFileSync(binaryPath, "");

  const resolved = resolveBinaryPath({
    packageRoot,
    platform: "linux",
    arch: "x64",
    resolvePackageJson: (specifier) => {
      assert.equal(specifier, "@refinedstone/akra-linux-x64/package.json");
      return packageJsonPath;
    },
  });

  assert.equal(resolved.binaryPath, binaryPath);
});

test("resolveBinaryPath throws for unsupported platform", () => {
  assert.throws(
    () =>
      resolveBinaryPath({
        packageRoot: "/tmp/akra",
        platform: "darwin",
        arch: "x64",
        resolvePackageJson: () => {
          throw new Error("unreachable");
        },
      }),
    /Unsupported platform: darwin \(x64\)/,
  );
});
