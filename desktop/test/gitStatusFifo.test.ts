import { execFileSync } from "node:child_process";
import { closeSync, constants, mkdtempSync, openSync, realpathSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { expect, it, vi } from "vitest";

const barrier = vi.hoisted(() => ({ beforeOpen: null as ((path: string) => void) | null }));
vi.mock("node:fs/promises", async (importOriginal) => {
  const actual = await importOriginal<typeof import("node:fs/promises")>();
  return {
    ...actual,
    open: (...args: Parameters<typeof actual.open>) => {
      barrier.beforeOpen?.(String(args[0]));
      return actual.open(...args);
    },
  };
});

import { readGitFileDiff } from "../src/main/gitStatus";

it.skipIf(process.platform === "win32")("rejects a regular file replaced by a writerless FIFO before open", async () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "akra-untracked-fifo-")));
  const target = join(root, "changes.txt");
  const environment = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GIT_")));
  execFileSync("git", ["init", "-q"], { cwd: root, env: environment });
  writeFileSync(target, "ordinary file\n");
  let replaced = false;
  let neededWriter = false;
  let rescue: ReturnType<typeof setInterval> | undefined;
  barrier.beforeOpen = (path) => {
    if (path !== target) return;
    barrier.beforeOpen = null;
    unlinkSync(target);
    execFileSync("mkfifo", [target]);
    replaced = true;
  };
  // The fixed read must fail without a writer. On regression only, release the
  // blocked read after the deadline so the test fails without leaking a worker.
  const watchdog = setTimeout(() => {
    neededWriter = true;
    rescue = setInterval(() => {
      try { closeSync(openSync(target, constants.O_WRONLY | constants.O_NONBLOCK)); }
      catch { /* Retry until the blocked open has reached the FIFO. */ }
    }, 20);
  }, 2000);
  try {
    await expect(readGitFileDiff(root, "changes.txt", true)).rejects.toThrow(/읽기 도중 변경|일반 파일/);
    expect(replaced).toBe(true);
    expect(neededWriter).toBe(false);
  } finally {
    clearTimeout(watchdog);
    if (rescue) clearInterval(rescue);
    barrier.beforeOpen = null;
    rmSync(root, { recursive: true, force: true, maxRetries: 5 });
  }
});
