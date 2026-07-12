import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { setTimeout as delay } from "node:timers/promises";
import { Worker } from "node:worker_threads";

import {
  AppServerClient,
  decodeCanonicalBase64,
  sampleLinuxProcessTree,
} from "./app-server-harness.mjs";

test("decodeCanonicalBase64 rejects malformed success payload fields", () => {
  assert.equal(decodeCanonicalBase64("YQ==", "result.dataBase64").toString(), "a");
  assert.equal(decodeCanonicalBase64("", "result.dataBase64").byteLength, 0);
  for (const value of [undefined, "%%%", "YQ", "YR=="]) {
    assert.throws(() => decodeCanonicalBase64(value, "result.dataBase64"));
  }
});

test("Linux sampler fails closed when task children inventory is unavailable", async () => {
  const procRoot = await mkdtemp(join(tmpdir(), "akra-proc-fixture-"));
  try {
    await mkdir(join(procRoot, "4242", "task", "4242"), { recursive: true });
    await assert.rejects(
      sampleLinuxProcessTree(4242, procRoot),
      /task children inventory is unavailable/,
    );
  } finally {
    await rm(procRoot, { recursive: true, force: true });
  }
});

test(
  "close does not write to a child that already exited by signal",
  { skip: process.platform === "win32", timeout: 5_000 },
  async () => {
    const child = spawn(process.execPath, ["-e", "setTimeout(() => {}, 30000)"], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    const client = new AppServerClient(child, 1_000);
    let stdinEndCalls = 0;
    const stdinEnd = child.stdin.end;
    child.stdin.end = function (...args) {
      stdinEndCalls += 1;
      return stdinEnd.apply(this, args);
    };

    try {
      await once(child, "spawn");
      assert.equal(child.kill("SIGTERM"), true);
      const exit = await Promise.race([
        client.exit,
        delay(1_000).then(() => {
          throw new Error("signal-terminated child did not exit within 1 second");
        }),
      ]);
      assert.equal(exit.code, null);
      assert.equal(exit.signal, "SIGTERM");
      assert.equal(child.exitCode, null);
      assert.equal(child.signalCode, "SIGTERM");

      assert.deepEqual(await client.close(), exit);
      assert.equal(stdinEndCalls, 0);
    } finally {
      if (child.exitCode === null && child.signalCode === null) {
        child.kill("SIGKILL");
      }
    }
  },
);

test(
  "Linux sampler finds a process spawned from a non-leader thread",
  { skip: process.platform !== "linux" },
  async () => {
    let childPid;
    const worker = new Worker(
      `
        const { spawn } = require("node:child_process");
        const { parentPort } = require("node:worker_threads");
        const child = spawn(process.execPath, ["-e", "setTimeout(() => {}, 30000)"]);
        parentPort.postMessage(child.pid);
        parentPort.once("message", () => child.kill("SIGTERM"));
        child.once("exit", () => process.exit(0));
      `,
      { eval: true },
    );

    try {
      [childPid] = await once(worker, "message");
      assert.ok(Number.isInteger(childPid));
      const sample = await sampleLinuxProcessTree(process.pid);
      assert.ok(sample.processCount >= 2, JSON.stringify(sample));
      assert.ok(sample.rssKib > 0, JSON.stringify(sample));
    } finally {
      if (Number.isInteger(childPid)) {
        try {
          process.kill(childPid, "SIGTERM");
        } catch (error) {
          if (error.code !== "ESRCH") {
            throw error;
          }
        }
      }
      const exited = once(worker, "exit");
      worker.postMessage("stop");
      if (await Promise.race([exited.then(() => false), delay(1_000, true)])) {
        await worker.terminate();
      }
    }
  },
);
