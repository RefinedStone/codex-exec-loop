import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const proxyScript = fileURLToPath(new URL("./proxy_ws_check.mjs", import.meta.url));
const pythonScript = fileURLToPath(new URL("./multi_client_check.py", import.meta.url));
const invalid = ["0", "-1", "NaN", "Infinity", "1.5", ""];

test("proxy WebSocket checker rejects invalid client counts before spawning", () => {
  for (const clients of invalid) {
    const result = spawnSync(process.execPath, [proxyScript, "never-run-this", clients], {
      encoding: "utf8",
      timeout: 5000,
    });
    assert.equal(result.status, 2, "invalid client count: " + JSON.stringify(clients));
    assert.match(result.stderr, /clients must be a positive integer/);
    assert.equal(result.stdout, "");
  }
});

test("Python multi-client checker also rejects empty or negative counts", (t) => {
  const python = spawnSync("python3", ["--version"], {
    encoding: "utf8",
    timeout: 5000,
  });
  if (python.error || python.status !== 0) {
    t.skip("python3 is unavailable in this environment");
    return;
  }
  for (const clients of ["0", "-1", "NaN"]) {
    const result = spawnSync(
      "python3",
      ["-I", pythonScript, "--socket", "/does-not-exist", "--clients", clients],
      { encoding: "utf8", timeout: 5000 },
    );
    assert.equal(result.status, 2);
    assert.match(result.stderr, /--clients/);
    assert.doesNotMatch(result.stderr, /ConnectionRefusedError|FileNotFoundError/);
  }
});
