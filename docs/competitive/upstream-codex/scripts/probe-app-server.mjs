#!/usr/bin/env node

import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";

import {
  AppServerClient,
  createIsolatedCodexHome,
  decodeCanonicalBase64,
  delay,
  removeIsolatedCodexHome,
} from "./app-server-harness.mjs";

let fatalReported = false;
function reportFatal(error) {
  if (fatalReported) {
    return;
  }
  fatalReported = true;
  const message = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  process.stderr.write(
    `${JSON.stringify({
      fatal: {
        messageBytes: Buffer.byteLength(message),
        messageSha256: createHash("sha256").update(message).digest("hex"),
      },
    })}\n`,
  );
  process.exitCode = 1;
}

process.on("uncaughtException", reportFatal);
process.on("unhandledRejection", reportFatal);

const binary = process.env.CODEX_APP_SERVER;
if (!binary) {
  throw new Error("set CODEX_APP_SERVER to the released codex-app-server binary");
}

const SAFE_ERROR_MESSAGES = new Set([
  "Already initialized",
  "Not initialized",
  "process/spawn requires experimentalApi capability",
]);

function boundedText(value, maximumCharacters) {
  const text = String(value).replace(/\s+/g, " ").trim();
  return text.length <= maximumCharacters
    ? text
    : `${text.slice(0, maximumCharacters - 3)}...`;
}

async function sha256(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) {
    hash.update(chunk);
  }
  return hash.digest("hex");
}

function responseOutcome(message) {
  if (message.error) {
    const errorMessage = String(message.error.message ?? "");
    const error = {
      code: Number.isSafeInteger(message.error.code) ? message.error.code : "nonNumeric",
    };
    if (SAFE_ERROR_MESSAGES.has(errorMessage)) {
      error.message = errorMessage;
    } else {
      error.messageBytes = Buffer.byteLength(errorMessage);
      error.messageSha256 = createHash("sha256").update(errorMessage).digest("hex");
    }
    return { error };
  }
  return { result: true };
}

function resultSummary(method, result) {
  if (method === "account/read") {
    return { account: result.account === null ? null : "present" };
  }
  if (Array.isArray(result.data)) {
    return {
      rows: result.data.length,
      nextCursor: result.nextCursor == null ? null : "present",
    };
  }
  if (method === "plugin/list") {
    return {
      marketplaces: Array.isArray(result.marketplaces) ? result.marketplaces.length : null,
      loadErrors: Array.isArray(result.marketplaceLoadErrors)
        ? result.marketplaceLoadErrors.length
        : null,
    };
  }
  if (method === "remoteControl/status/read") {
    return { status: boundedText(result.status, 80) };
  }
  const keys = Object.keys(result).sort();
  return {
    keys: keys.slice(0, 32).map((key) => boundedText(key, 80)),
    truncated: keys.length > 32,
  };
}

async function withClient(capabilities, operation) {
  const home = await createIsolatedCodexHome();
  let client;
  try {
    client = AppServerClient.start(binary, home);
    return await operation(client);
  } finally {
    try {
      if (client) {
        await client.close();
      }
    } finally {
      await removeIsolatedCodexHome(home);
    }
  }
}

async function initializedClient(capabilities, operation) {
  return withClient(capabilities, async (client) => {
    const initialized = await client.initialize(capabilities);
    if (initialized.error) {
      throw new Error(`initialize failed: ${JSON.stringify(initialized.error)}`);
    }
    client.notify("initialized");
    return operation(client, initialized.result);
  });
}

const positiveMethods = [
  ["account/read", { refreshToken: false }],
  ["thread/list", { limit: 10 }],
  ["thread/loaded/list", {}],
  ["model/list", { limit: 100 }],
  ["collaborationMode/list", {}],
  ["app/list", { forceRefetch: false, limit: 100 }],
  ["skills/list", { cwds: [], forceReload: false }],
  ["plugin/list", { cwds: [] }],
  ["remoteControl/status/read", {}],
  ["permissionProfile/list", { limit: 100 }],
  ["hooks/list", { cwds: [] }],
  ["config/read", { includeLayers: false }],
  ["mcpServerStatus/list", { limit: 100 }],
  ["experimentalFeature/list", { limit: 200 }],
];

const positive = await initializedClient({ experimentalApi: true }, async (client) => {
  const observations = [];
  for (const [method, params] of positiveMethods) {
    const response = await client.request(method, params);
    observations.push({
      method,
      ...(response.error
        ? responseOutcome(response)
        : { result: resultSummary(method, response.result) }),
    });
  }
  return observations;
});

const beforeInitialize = await withClient({}, async (client) =>
  responseOutcome(await client.request("account/read", { refreshToken: false })),
);

const repeatedInitialize = await withClient({}, async (client) => {
  const first = await client.initialize({ experimentalApi: false });
  client.notify("initialized");
  const second = await client.initialize({ experimentalApi: false });
  return { first: responseOutcome(first), second: responseOutcome(second) };
});

const stableBoundary = await initializedClient({ experimentalApi: false }, async (client) => {
  const read = await client.request("fs/readFile", { path: "/etc/hostname" });
  const spawn = await client.request("process/spawn", {
    command: ["/usr/bin/true"],
    cwd: "/tmp",
    processHandle: "akra-audit-capability-gate",
  });
  return {
    fsReadFile: read.error
      ? responseOutcome(read)
      : {
          result: true,
          decodedBytes: decodeCanonicalBase64(
            read.result?.dataBase64,
            "fs/readFile result.dataBase64",
          ).byteLength,
        },
    processSpawn: responseOutcome(spawn),
  };
});

async function initialRemoteControlNotifications(optOut) {
  const capabilities = {
    experimentalApi: false,
    ...(optOut ? { optOutNotificationMethods: ["remoteControl/status/changed"] } : {}),
  };
  return initializedClient(capabilities, async (client) => {
    await client.request("account/read", { refreshToken: false });
    await delay(100);
    return client.notifications.filter(
      (notification) => notification.method === "remoteControl/status/changed",
    ).length;
  });
}

const notificationOptOut = {
  controlCount: await initialRemoteControlNotifications(false),
  optedOutCount: await initialRemoteControlNotifications(true),
};

const binaryStat = await stat(binary);
console.log(
  JSON.stringify(
    {
      artifact: {
        bytes: binaryStat.size,
        sha256: await sha256(binary),
      },
      runtime: { node: process.version, platform: process.platform, arch: process.arch },
      positive,
      negative: {
        beforeInitialize,
        repeatedInitialize,
        stableBoundary,
        notificationOptOut,
      },
    },
    null,
    2,
  ),
);
