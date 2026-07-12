import assert from "node:assert/strict";
import { once } from "node:events";
import { test } from "node:test";
import { setTimeout as delay } from "node:timers/promises";
import { Worker } from "node:worker_threads";

import { sampleLinuxProcessTree } from "./app-server-harness.mjs";

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
