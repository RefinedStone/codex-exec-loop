// #2625: real Windows Desktop -> byte relay -> production WSL Gateway -> synthetic Codex.
import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { _electron as electron } from "playwright-core";

assert.equal(process.platform, "win32", "This scenario pairs Windows Electron with a WSL Gateway.");
const { AKRA_FORK_WSL_DISTRO: distro = "Ubuntu", AKRA_FORK_GATEWAY_BIN: binary, AKRA_FORK_ELECTRON: electronPath } = process.env;
assert(binary?.startsWith("/") && electronPath && isAbsolute(electronPath), "Set absolute Gateway (Linux) and Electron (Windows) executable paths.");
const run = promisify(execFile);
const wsl = ["-d", distro, "--exec"];
const windowsHelper = fileURLToPath(new URL("../fixtures/forkGatewayWsl.mjs", import.meta.url));
const { stdout } = await run("wsl.exe", [...wsl, "/usr/bin/wslpath", "-a", windowsHelper], { windowsHide: true, timeout: 15_000 });
const helper = stdout.trim();
const nonce = randomUUID();
const root = `/dev/shm/akra-fork-${nonce}`;
const args = mode => [...wsl, "/usr/bin/env", "-i", "PATH=/usr/bin:/bin", "/usr/bin/node", helper, mode, root, binary];
const control = async mode => (await run("wsl.exe", args(mode), { windowsHide: true, timeout: 15_000 })).stdout;
const userData = await mkdtemp(join(tmpdir(), "akra-fork-gateway-"));
const endpoint = `\\\\.\\pipe\\akra-fork-fixture-${nonce}`;
const gateway = spawn("wsl.exe", args("serve"), { windowsHide: true, stdio: ["pipe", "pipe", "inherit"] });
const gatewayExit = once(gateway, "exit");
const relays = new Set();
const relay = createServer(socket => {
  const child = spawn("wsl.exe", args("relay"), { windowsHide: true, stdio: ["pipe", "pipe", "inherit"] });
  relays.add(child);
  child.once("error", error => socket.destroy(error));
  child.once("close", () => { relays.delete(child); socket.destroy(); });
  child.stdin.on("error", error => socket.destroy(error));
  socket.on("error", () => child.stdin.end());
  socket.on("close", () => child.stdin.end());
  socket.pipe(child.stdin);
  child.stdout.pipe(socket);
});
let app;
try {
  const lines = createInterface({ input: gateway.stdout });
  const listening = once(lines, "line", { signal: AbortSignal.timeout(30_000) });
  const [line] = await Promise.race([listening, gatewayExit.then(([code]) => {
    throw new Error(`Gateway fixture exited before listening: ${code}`);
  })]);
  assert.equal(line, `akra-gateway listening on ${root}/akra/run/gateway.sock`);
  relay.listen(endpoint);
  await once(relay, "listening");
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) =>
    /^(systemroot|windir|systemdrive|path|comspec|pathext|temp|tmp|localappdata)$/i.test(key)));
  Object.assign(env, {
    AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_GATEWAY_BIN: join(userData, "no-native-daemon.exe"),
    AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData,
    HOME: userData, USERPROFILE: userData, APPDATA: userData, LOCALAPPDATA: userData,
    AKRA_HOME: join(userData, "akra"), CODEX_HOME: join(userData, "codex"),
  });
  const desktop = resolve(fileURLToPath(new URL("../../", import.meta.url)));
  app = await electron.launch({ executablePath: electronPath, args: [desktop], env, timeout: 30_000 });
  const page = await app.firstWindow({ timeout: 30_000 });
  page.setDefaultTimeout(12_000);
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor();
  const row = name => page.getByTestId("thread-entry").filter({ hasText: name });
  const source = row("Fork fixture source");
  const deleted = row("Deleted fork fixture child");
  await source.click();
  await page.getByTestId("writer-elsewhere-banner").waitFor();
  const fork = page.getByTestId("writer-elsewhere-fork");

  await fork.click();
  await deleted.waitFor();
  await control("release-delete");
  await deleted.waitFor({ state: "detached" });
  await control("release-reply");
  await page.getByText(/새 대화로 분기했습니다/).waitFor();
  assert.equal(await deleted.count(), 0, "a successful reply must not resurrect the deleted child");
  assert.equal(await page.getByTestId("forked-from-banner").count(), 0);
  assert(await page.getByTestId("writer-elsewhere-banner").isVisible(), "the locked source must stay selected");
  console.log("PASS production Gateway delivered deletion before delayed success; Desktop did not restore or select the child");

  await fork.click();
  const unknown = page.getByText(/분기 결과를 확인하지 못했습니다/);
  const opened = page.getByTestId("forked-from-banner");
  await unknown.or(opened).first().waitFor();
  assert(await unknown.isVisible(), "a wrong-worktree reply must not open the child");
  assert.equal(await row("Wrong worktree fixture child").count(), 0);
  assert.equal(await page.getByTestId("forked-from-banner").count(), 0);
  assert(await page.getByTestId("writer-elsewhere-banner").isVisible());
  assert.equal(await page.locator(`.project-header[title="${root}/second"]`).count(), 0);
  assert.equal(await page.locator(`.project-header[title="${root}/first"]`).count(), 1);
  const calls = (await control("requests")).trim().split(/\r?\n/).map(line => JSON.parse(line));
  assert.equal(calls.filter(call => call.method === "thread/fork").length, 2);
  assert.equal(calls.filter(call => call.method === "thread/resume").length, 1);
  assert(!calls.some(call => call.method === "thread/start" || call.method.startsWith("turn/")));
  console.log("PASS production Gateway relayed a wrong-worktree reply; Desktop held the source with no fallback or model turn");
} finally {
  try {
    await app?.close();
  } finally {
    for (const child of relays) child.stdin.end();
    relay.close();
    gateway.stdin.end();
    const [code] = await gatewayExit;
    await rm(userData, { recursive: true, force: true, maxRetries: 5 });
    assert.equal(code, 0, "disposable Gateway did not shut down cleanly");
  }
}
