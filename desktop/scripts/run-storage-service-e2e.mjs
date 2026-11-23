// Explicit cross-stack gate. The ordinary Node/Admin E2E suite does not need Rust.
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(desktopRoot, "..");
async function buildFixture() {
  const artifacts = new Set();
  const cargo = process.env.CARGO || (process.platform === "win32" ? "cargo.exe" : "cargo");
  const child = spawn(cargo, ["test", "--locked", "--lib", "storage_desktop_fixture", "--no-run", "--message-format=json"], {
    cwd: repositoryRoot, env: process.env, stdio: ["ignore", "pipe", "inherit"],
  });
  createInterface({ input: child.stdout }).on("line", line => {
    let event;
    try { event = JSON.parse(line); } catch { console.error(line); return; }
    if (event.reason === "compiler-message" && event.message?.rendered) process.stderr.write(event.message.rendered);
    if (event.reason === "compiler-artifact" && event.profile?.test && event.target?.kind?.includes("lib") && event.executable) {
      artifacts.add(event.executable);
    }
  });
  const code = await new Promise((resolveExit, reject) => { child.once("error", reject); child.once("close", resolveExit); });
  if (code !== 0) throw new Error("storage fixture build failed: " + code);
  if (artifacts.size !== 1) throw new Error("expected one library test executable in Cargo JSON, got " + artifacts.size);
  return [...artifacts][0];
}

// Cargo reports the executable for its active target/platform. No target path, extension, or hash is assumed.
// An explicit override supports a separately built artifact; it must match the current source revision.
const executable = process.env.AKRA_STORAGE_FIXTURE_BIN || await buildFixture();
const child = spawn(process.execPath, [...process.execArgv, join(desktopRoot, "test/e2e/storage-service-roundtrip.mjs")], {
  cwd: desktopRoot, env: { ...process.env, AKRA_STORAGE_FIXTURE_BIN: executable }, stdio: "inherit",
});
const code = await new Promise((resolveExit, reject) => { child.once("error", reject); child.once("close", resolveExit); });
if (code !== 0) throw new Error("storage service roundtrip failed: " + code);
