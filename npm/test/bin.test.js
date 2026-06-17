import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { resolvePlatformConfig } from "../lib/platform.js";

const __filename = fileURLToPath(import.meta.url);
const packageRoot = path.resolve(path.dirname(__filename), "..");

const signalTestConfig = resolvePlatformConfig(process.platform, process.arch);
const skipSignalTest =
  process.platform === "win32" || signalTestConfig === null
    ? "POSIX signal exit assertions require a supported non-Windows target"
    : false;

test(
  "akra bin maps native SIGTERM exits to a non-success signal exit code",
  { skip: skipSignalTest },
  async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "akra-bin-"));

    try {
      const config = signalTestConfig;
      assert(config);
      const testPackageRoot = createTestPackageRoot(tmp);
      const fixtureBinaryPath = path.join(
        testPackageRoot,
        "vendor",
        config.targetTriple,
        "akra",
        config.binaryName,
      );
      writeFile(
        fixtureBinaryPath,
        "#!/usr/bin/env node\nprocess.kill(process.pid, 'SIGTERM');\n",
      );
      fs.chmodSync(fixtureBinaryPath, 0o755);

      const result = await runNode(
        [path.join(testPackageRoot, "bin", "akra.js")],
        {
          cwd: testPackageRoot,
          env: {
            ...process.env,
            npm_config_user_agent: "npm/11.0.0 node/v22.0.0",
          },
        },
      );

      assert.equal(result.signal, null);
      assert.equal(result.code, 143);
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  },
);

function createTestPackageRoot(root) {
  const testPackageRoot = path.join(root, "package");
  fs.mkdirSync(path.join(testPackageRoot, "bin"), { recursive: true });
  fs.mkdirSync(path.join(testPackageRoot, "lib"), { recursive: true });
  fs.copyFileSync(
    path.join(packageRoot, "bin", "akra.js"),
    path.join(testPackageRoot, "bin", "akra.js"),
  );
  for (const fileName of ["platform.js", "runtime.js"]) {
    fs.copyFileSync(
      path.join(packageRoot, "lib", fileName),
      path.join(testPackageRoot, "lib", fileName),
    );
  }
  writeFile(path.join(testPackageRoot, "package.json"), '{"type":"module"}\n');
  return testPackageRoot;
}

function writeFile(filePath, body) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, body);
}

function runNode(args, options) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, args, {
      ...options,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    const timeout = setTimeout(() => {
      child.kill("SIGKILL");
      reject(
        new Error(`node subprocess timed out\nstdout:\n${stdout}\nstderr:\n${stderr}`),
      );
    }, 5000);

    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });
    child.on("close", (code, signal) => {
      clearTimeout(timeout);
      resolve({ code, signal, stdout, stderr });
    });
  });
}
