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

test(
  "akra bin escalates a repeated SIGTERM when the native child ignores shutdown",
  { skip: skipSignalTest },
  async () => {
    const result = await runIgnoredSignalFixture({
      graceMs: 60000,
      repeatSignal: true,
    });
    assert.equal(result.signal, null);
    assert.equal(result.code, 137);
    assert(result.elapsedMs < 2500, `signal escalation took ${result.elapsedMs}ms`);
  },
);

test(
  "akra bin bounds shutdown when the native child ignores a single SIGTERM",
  { skip: skipSignalTest },
  async () => {
    const result = await runIgnoredSignalFixture({
      graceMs: 500,
      repeatSignal: false,
    });
    assert.equal(result.signal, null);
    assert.equal(result.code, 137);
    assert(result.elapsedMs < 2500, `bounded shutdown took ${result.elapsedMs}ms`);
  },
);

test(
  "akra bin reports a missing native binary without a Node stack trace",
  { skip: signalTestConfig === null ? "test host must map to a published target" : false },
  async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "akra-bin-missing-"));

    try {
      const testPackageRoot = createTestPackageRoot(tmp);
      const result = await runNode([path.join(testPackageRoot, "bin", "akra.js")], {
        cwd: testPackageRoot,
        env: { ...process.env, AKRA_NPM_DEBUG: "0" },
      });

      assert.equal(result.code, 1);
      assert.match(result.stderr, /^akra: /);
      assert.match(result.stderr, /Missing optional dependency|native binary was not found/);
      assert.doesNotMatch(result.stderr, /\n\s*at /);
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

async function runIgnoredSignalFixture({ graceMs, repeatSignal }) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "akra-bin-ignore-signal-"));
  let nativePid = null;
  let wrapper = null;

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
    const readyPath = path.join(tmp, "ready");
    const signalPath = path.join(tmp, "signal");
    writeFile(
      fixtureBinaryPath,
      `#!/usr/bin/env node
import fs from "node:fs";
process.on("SIGTERM", () => fs.appendFileSync(process.env.AKRA_TEST_SIGNAL, "SIGTERM\\n"));
fs.writeFileSync(process.env.AKRA_TEST_READY, String(process.pid));
setInterval(() => {}, 1000);
`,
    );
    fs.chmodSync(fixtureBinaryPath, 0o755);

    let stdout = "";
    let stderr = "";
    wrapper = spawn(
      process.execPath,
      [path.join(testPackageRoot, "bin", "akra.js")],
      {
        cwd: testPackageRoot,
        env: {
          ...process.env,
          AKRA_NPM_SHUTDOWN_GRACE_MS: String(graceMs),
          AKRA_TEST_READY: readyPath,
          AKRA_TEST_SIGNAL: signalPath,
        },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    wrapper.stdout.setEncoding("utf8");
    wrapper.stderr.setEncoding("utf8");
    wrapper.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    wrapper.stderr.on("data", (chunk) => {
      stderr += chunk;
    });

    await waitForPath(readyPath, wrapper);
    nativePid = Number(fs.readFileSync(readyPath, "utf8"));
    const startedAt = Date.now();
    wrapper.kill("SIGTERM");
    await waitForPath(signalPath, wrapper);
    if (repeatSignal) {
      wrapper.kill("SIGTERM");
    }

    const closed = await waitForClose(wrapper, 2500, stdout, stderr);
    nativePid = null;
    return { ...closed, elapsedMs: Date.now() - startedAt };
  } finally {
    if (wrapper?.exitCode === null && wrapper?.signalCode === null) {
      wrapper.kill("SIGKILL");
    }
    if (Number.isInteger(nativePid)) {
      try {
        process.kill(nativePid, "SIGKILL");
      } catch {
        // The expected path already reaped the native child.
      }
    }
    fs.rmSync(tmp, { recursive: true, force: true });
  }
}

async function waitForPath(filePath, child) {
  const deadline = Date.now() + 2000;
  while (Date.now() < deadline) {
    if (fs.existsSync(filePath)) {
      return;
    }
    if (child.exitCode !== null || child.signalCode !== null) {
      throw new Error(`wrapper exited before fixture path appeared: ${filePath}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`timed out waiting for fixture path: ${filePath}`);
}

function waitForClose(child, timeoutMs, stdout, stderr) {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(
        new Error(
          `wrapper did not exit after signal escalation\nstdout:\n${stdout}\nstderr:\n${stderr}`,
        ),
      );
    }, timeoutMs);
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
