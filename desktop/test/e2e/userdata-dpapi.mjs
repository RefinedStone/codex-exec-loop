// Real Windows profile seed -> migration -> independent reopen. No app/account login.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import electron from "electron";

if (process.platform !== "win32") {
  console.log("Windows DPAPI profile test: not applicable (macOS/Linux keychain testing requires a disposable OS account)");
} else {
  const root = mkdtempSync(join(tmpdir(), "akra-dpapi-profile-"));
  try {
    const fixture = fileURLToPath(new URL("../fixtures/userdata-dpapi.electron.mjs", import.meta.url));
    const env = { ...process.env };
    delete env.ELECTRON_RUN_AS_NODE;
    for (const mode of ["seed", "upgrade", "reopen"]) {
      const child = spawnSync(electron, [fixture, mode, root], {
        env, encoding: "utf8", timeout: 25_000, windowsHide: true,
      });
      if (child.error) throw child.error;
      assert.equal(child.status, 0, mode + ": " + child.stderr);
      assert.ok(child.stdout.includes("isolated Windows profile " + mode + " passed"), mode);
      console.log(child.stdout.trim());
    }
  } finally {
    rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
}
