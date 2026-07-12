#!/usr/bin/env node

import { performance } from "node:perf_hooks";

import {
  AppServerClient,
  createIsolatedCodexHome,
  delay,
  removeIsolatedCodexHome,
  sampleLinuxProcessTree,
} from "./app-server-harness.mjs";

const binary = process.env.CODEX_APP_SERVER;
if (!binary) {
  throw new Error("set CODEX_APP_SERVER to the released codex-app-server binary");
}
if (process.platform !== "linux") {
  throw new Error("RSS collection requires Linux /proc");
}

const measuredRuns = Number.parseInt(process.env.AKRA_AUDIT_RUNS ?? "10", 10);
const pollingMs = Number.parseInt(process.env.AKRA_AUDIT_RSS_POLL_MS ?? "2", 10);
if (!Number.isInteger(measuredRuns) || measuredRuns < 1 || measuredRuns > 100) {
  throw new Error("AKRA_AUDIT_RUNS must be between 1 and 100");
}
if (!Number.isInteger(pollingMs) || pollingMs < 1 || pollingMs > 1_000) {
  throw new Error("AKRA_AUDIT_RSS_POLL_MS must be between 1 and 1000");
}

async function runSample() {
  const home = await createIsolatedCodexHome();
  const startedAt = performance.now();
  const client = AppServerClient.start(binary, home);
  let sampling = true;
  let peakRssKib = 0;
  let peakProcessCount = 0;
  const sampler = (async () => {
    while (sampling) {
      const sample = await sampleLinuxProcessTree(client.pid);
      peakRssKib = Math.max(peakRssKib, sample.rssKib);
      peakProcessCount = Math.max(peakProcessCount, sample.processCount);
      await delay(pollingMs);
    }
  })();

  try {
    const initialized = await client.initialize({ experimentalApi: false });
    if (initialized.error) {
      throw new Error(`initialize failed: ${JSON.stringify(initialized.error)}`);
    }
    client.notify("initialized");
    const account = await client.request("account/read", { refreshToken: false });
    if (account.error) {
      throw new Error(`account/read failed: ${JSON.stringify(account.error)}`);
    }
    const threads = await client.request("thread/list", { limit: 10 });
    if (threads.error) {
      throw new Error(`thread/list failed: ${JSON.stringify(threads.error)}`);
    }
    const elapsedMs = performance.now() - startedAt;
    sampling = false;
    await sampler;
    return {
      elapsedMs: Number(elapsedMs.toFixed(3)),
      sampledPeakRssKib: peakRssKib,
      peakProcessCount,
    };
  } finally {
    sampling = false;
    try {
      await sampler;
    } finally {
      try {
        await client.close();
      } finally {
        await removeIsolatedCodexHome(home);
      }
    }
  }
}

function median(values) {
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[middle - 1] + sorted[middle]) / 2
    : sorted[middle];
}

await runSample();
const samples = [];
for (let index = 0; index < measuredRuns; index += 1) {
  samples.push(await runSample());
}

const elapsed = samples.map((sample) => sample.elapsedMs);
const rss = samples.map((sample) => sample.sampledPeakRssKib);
const processes = samples.map((sample) => sample.peakProcessCount);
console.log(
  JSON.stringify(
    {
      boundary: "immediately before spawn through final thread/list response",
      warmupRuns: 1,
      measuredRuns,
      rss: {
        source: "recursive /proc/<pid>/task/*/children union and /proc/<pid>/status VmRSS",
        requestedPollingMs: pollingMs,
      },
      samples,
      summary: {
        elapsedMs: {
          min: Math.min(...elapsed),
          median: Number(median(elapsed).toFixed(3)),
          max: Math.max(...elapsed),
        },
        sampledPeakRssKib: {
          min: Math.min(...rss),
          median: Number(median(rss).toFixed(1)),
          max: Math.max(...rss),
        },
        peakProcessCount: {
          min: Math.min(...processes),
          median: median(processes),
          max: Math.max(...processes),
        },
      },
    },
    null,
    2,
  ),
);
