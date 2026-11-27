// Native Electron IPC with a real Git repository. The Gateway never starts a model turn.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { _electron as electron } from "playwright-core";

const desktop = join(dirname(fileURLToPath(import.meta.url)), "../..");
const fake = await import(pathToFileURL(join(desktop, "test/fake-gateway/fakeGateway.ts")).href);
const root = await mkdtemp(join(tmpdir(), "akra-snapshot-ipc-"));
const run = promisify(execFile);
const gitEnv = { ...Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GIT_"))), GIT_MASTER: "1" };
const git = async (...args) => (await run("git", ["-c", "user.name=Tester", "-c",
  "user.email=fixture@localhost.invalid", "-c", "commit.gpgsign=false", ...args],
{ cwd: root, windowsHide: true, env: gitEnv })).stdout;
await git("init", "-q", "-b", "main");
await git("config", "core.autocrlf", "false");
await writeFile(join(root, "file.txt"), "HEAD_ONLY\nunchanged\n");
await git("add", "."); await git("commit", "-qm", "fixture");
await writeFile(join(root, "file.txt"), "INDEX_ONLY\nunchanged\n");
await git("add", ".");
await writeFile(join(root, "file.txt"), "WORKTREE_ONLY\nunchanged\n");
const indexBefore = await git("diff", "--cached");
const recording = join(root, "recording.jsonl");
await writeFile(recording, (await readFile(fake.APPROVAL_THREAD_LIST_RECORDING, "utf8")).replaceAll("/work/example-app", root.replaceAll("\\", "/")));
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [recording]);
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: join(root, "profile") };
delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ executablePath: process.env.AKRA_ELECTRON_EXECUTABLE ?? createRequire(import.meta.url)("electron"), args: [desktop], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  const captured = await page.evaluate(async (cwd) => {
    const diff = await window.akra.gitFileDiff(cwd, "file.txt", false, "index_worktree");
    const request = { snapshotId: diff.snapshot.snapshotId, source: "index_worktree", side: "old", startLine: 1, endLine: 2 };
    return { request, old: await window.akra.gitDiffContext(cwd, request),
      next: await window.akra.gitDiffContext(cwd, { ...request, side: "new" }) };
  }, root);
  assert.equal(captured.old.text, "INDEX_ONLY\nunchanged\n");
  assert.equal(captured.next.text, "WORKTREE_ONLY\nunchanged\n");
  await writeFile(join(root, "file.txt"), "LATER_FILE\n");
  const retained = await page.evaluate(({ cwd, request }) => window.akra.gitDiffContext(cwd, request), { cwd: root, request: captured.request });
  assert.equal(retained.text, "INDEX_ONLY\nunchanged\n");
  assert.equal(retained.current, false);
  assert.equal(await git("diff", "--cached"), indexBefore);
  console.log("PASS native snapshot IPC: old INDEX_ONLY, new WORKTREE_ONLY, immutable stale context, unchanged index");
} finally {
  const exited = once(app.process(), "exit", { signal: AbortSignal.timeout(10_000) });
  for (const window of app.windows()) await window.close();
  await exited;
  await gateway.close();
  await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
}
