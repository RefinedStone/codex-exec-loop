#!/usr/bin/env node

import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { chmod, copyFile, mkdtemp, mkdir, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";

import {
  AppServerClient,
  createIsolatedCodexHome,
  delay,
  removeIsolatedCodexHome,
} from "./app-server-harness.mjs";

const binary = process.env.CODEX_APP_SERVER;
const authFile = process.env.CODEX_AUTH_FILE;
if (!binary || !authFile) {
  throw new Error("set CODEX_APP_SERVER and CODEX_AUTH_FILE");
}

const timeoutMs = Number(process.env.TERMINAL_CAPTURE_TIMEOUT_MS ?? 180_000);
const captureModel = process.env.TERMINAL_CAPTURE_MODEL || null;
let availableModelSlugs = [];
if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1_000 || timeoutMs > 600_000) {
  throw new Error("TERMINAL_CAPTURE_TIMEOUT_MS must be between 1000 and 600000");
}

async function sha256(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) {
    hash.update(chunk);
  }
  return hash.digest("hex");
}

async function installDisposableAuth(codexHome) {
  const metadata = await stat(authFile);
  if (!metadata.isFile()) {
    throw new Error("CODEX_AUTH_FILE must be a regular file");
  }
  if (typeof process.getuid === "function" && metadata.uid !== process.getuid()) {
    throw new Error("CODEX_AUTH_FILE must be owned by the current user");
  }
  if ((metadata.mode & 0o077) !== 0) {
    throw new Error("CODEX_AUTH_FILE must not grant group or other access");
  }
  const target = join(codexHome, "auth.json");
  await copyFile(authFile, target);
  await chmod(target, 0o600);
}

function turnStartParams(threadId, prompt) {
  return {
    threadId,
    input: [{ type: "text", text: prompt }],
    approvalPolicy: "never",
    sandboxPolicy: { type: "readOnly" },
    ...(captureModel ? { model: captureModel } : {}),
  };
}

async function awaitNotification(client, predicate, label) {
  const deadline = Date.now() + timeoutMs;
  let cursor = 0;
  while (Date.now() < deadline) {
    while (cursor < client.notifications.length) {
      const notification = client.notifications[cursor++];
      if (predicate(notification)) {
        return notification;
      }
    }
    await delay(25);
  }
  throw new Error(`timed out waiting for ${label}`);
}

function terminalShape(notification) {
  const params = notification.params ?? {};
  const turn = params.turn ?? {};
  const error = turn.error;
  return {
    method: notification.method,
    paramsFields: Object.keys(params).sort(),
    threadIdPresent: typeof params.threadId === "string" && params.threadId.length > 0,
    turnFields: Object.keys(turn).sort(),
    turnIdPresent: typeof turn.id === "string" && turn.id.length > 0,
    status: turn.status ?? null,
    itemsView: turn.itemsView ?? "omitted-default-full",
    itemCount: Array.isArray(turn.items) ? turn.items.length : null,
    startedAtType: turn.startedAt === null ? "null" : typeof turn.startedAt,
    completedAtType: turn.completedAt === null ? "null" : typeof turn.completedAt,
    durationMsType: turn.durationMs === null ? "null" : typeof turn.durationMs,
    error:
      error == null
        ? null
        : {
            fields: Object.keys(error).sort(),
            codexErrorInfoType:
              error.codexErrorInfo === null ? "null" : typeof error.codexErrorInfo,
          },
  };
}

function retryShapes(client) {
  return client.notifications
    .filter((notification) => notification.method === "error")
    .map((notification) => {
      const params = notification.params ?? {};
      const error = params.error ?? {};
      return {
        paramsFields: Object.keys(params).sort(),
        threadIdPresent: typeof params.threadId === "string" && params.threadId.length > 0,
        turnIdPresent: typeof params.turnId === "string" && params.turnId.length > 0,
        willRetry: params.willRetry ?? null,
        errorFields: Object.keys(error).sort(),
        codexErrorInfoType:
          error.codexErrorInfo === null ? "null" : typeof error.codexErrorInfo,
      };
    });
}

function terminalFailureClassification(terminal) {
  const turn = terminal.params?.turn ?? {};
  const error = turn.error ?? {};
  const info = error.codexErrorInfo;
  return {
    status: turn.status ?? null,
    errorFields: Object.keys(error).sort(),
    codexErrorInfo:
      typeof info === "string"
        ? info
        : info && typeof info === "object"
          ? { fields: Object.keys(info).sort(), type: info.type ?? null }
          : info ?? null,
  };
}

function rpcErrorClassification(response) {
  const error = response.error ?? {};
  const message = String(error.message ?? "");
  return {
    code: error.code ?? null,
    fields: Object.keys(error).sort(),
    messageBytes: Buffer.byteLength(message),
    messageSha256: createHash("sha256").update(message).digest("hex"),
  };
}

function requireTurnId(response, operation) {
  const turnId = response.result?.turn?.id;
  if (typeof turnId !== "string" || turnId.length === 0) {
    throw new Error(`${operation} omitted turn id`);
  }
  return turnId;
}

async function startThread(client, cwd) {
  const response = await client.request("thread/start", {
    cwd,
    approvalPolicy: "never",
    sandbox: "read-only",
    ephemeral: true,
    config: {
      projects: {
        [cwd]: { trust_level: "untrusted" },
      },
    },
  });
  if (response.error) {
    throw new Error("thread/start failed during terminal capture");
  }
  const threadId = response.result?.thread?.id;
  if (typeof threadId !== "string" || threadId.length === 0) {
    throw new Error("thread/start omitted thread id");
  }
  return threadId;
}

async function captureCompleted(client, cwd) {
  const threadId = await startThread(client, cwd);
  const response = await client.request(
    "turn/start",
    turnStartParams(threadId, "Reply with exactly OK. Do not use tools."),
  );
  if (response.error) {
    throw new Error("turn/start failed during completed capture");
  }
  const turnId = requireTurnId(response, "completed turn/start");
  const terminal = await awaitNotification(
    client,
    (notification) =>
      notification.method === "turn/completed" &&
      notification.params?.threadId === threadId &&
      notification.params?.turn?.id === turnId,
    "completed turn terminal",
  );
  if (terminal.params?.turn?.status !== "completed") {
    throw new Error(
      `normal capture did not end with completed status: ${JSON.stringify({ ...terminalFailureClassification(terminal), captureModel, availableModelSlugs })}`,
    );
  }
  return terminalShape(terminal);
}

async function captureInterrupted(client, cwd) {
  const threadId = await startThread(client, cwd);
  const startPromise = client.request(
    "turn/start",
    turnStartParams(
      threadId,
      "Use shell_command to run `sleep 30` as your first action. Do not reply before it finishes.",
    ),
  );
  const started = await awaitNotification(
    client,
    (notification) =>
      notification.method === "turn/started" &&
      notification.params?.threadId === threadId &&
      typeof notification.params?.turn?.id === "string" &&
      notification.params.turn.id.length > 0,
    "interrupted turn/started identity",
  );
  const turnId = started.params.turn.id;
  const interruptPromise = client.request("turn/interrupt", { threadId, turnId });
  const response = await startPromise;
  if (response.error) {
    throw new Error("turn/start failed during interrupted capture");
  }
  if (requireTurnId(response, "interrupted turn/start") !== turnId) {
    throw new Error("turn/start response did not match turn/started identity");
  }
  const interrupt = await interruptPromise;
  if (interrupt.error) {
    throw new Error(
      `turn/interrupt failed during interrupted capture: ${JSON.stringify({ ...rpcErrorClassification(interrupt), captureModel, availableModelSlugs })}`,
    );
  }
  const terminal = await awaitNotification(
    client,
    (notification) =>
      notification.method === "turn/completed" &&
      notification.params?.threadId === threadId &&
      notification.params?.turn?.id === turnId,
    "interrupted turn terminal",
  );
  if (terminal.params?.turn?.status !== "interrupted") {
    throw new Error("interrupt capture did not end with interrupted status");
  }
  return terminalShape(terminal);
}

let codexHome;
let workspaceRoot;
let client;
let primaryError;
try {
  codexHome = await createIsolatedCodexHome();
  workspaceRoot = await mkdtemp(join(tmpdir(), "akra-terminal-truth-"));
  await installDisposableAuth(codexHome);
  const completedWorkspace = join(workspaceRoot, "completed");
  const interruptedWorkspace = join(workspaceRoot, "interrupted");
  await mkdir(completedWorkspace);
  await mkdir(interruptedWorkspace);
  client = AppServerClient.start(binary, codexHome, timeoutMs);
  const initialized = await client.initialize({ experimentalApi: false });
  if (initialized.error) {
    throw new Error("initialize failed during terminal capture");
  }
  client.notify("initialized");
  const account = await client.request("account/read", { refreshToken: false });
  if (account.error || account.result?.account == null) {
    throw new Error("disposable app-server did not load an authenticated account");
  }
  const models = await client.request("model/list", { limit: 100 });
  if (!models.error && Array.isArray(models.result?.data)) {
    availableModelSlugs = models.result.data
      .map((model) => model.id ?? model.model ?? model.slug)
      .filter((model) => typeof model === "string")
      .sort();
  }

  const completed = await captureCompleted(client, completedWorkspace);
  const interrupted = await captureInterrupted(client, interruptedWorkspace);
  const output = {
    captureVersion: 1,
    capturedAt: new Date().toISOString(),
    platform: process.platform,
    architecture: process.arch,
    binary: {
      name: basename(binary),
      bytes: (await stat(binary)).size,
      sha256: await sha256(binary),
    },
    authenticatedAccountPresent: true,
    model: captureModel,
    interruptMode: "exact-turn-id",
    initializedResultFields: Object.keys(initialized.result ?? {}).sort(),
    completed,
    interrupted,
    errorNotifications: retryShapes(client),
  };
  process.stdout.write(`${JSON.stringify(output, null, 2)}\n`);
} catch (error) {
  primaryError = error;
  throw error;
} finally {
  const cleanupFailures = [];
  if (client) {
    const [closeResult] = await Promise.allSettled([client.close()]);
    if (closeResult.status === "rejected") {
      cleanupFailures.push(closeResult.reason);
    }
  }
  const removalResults = await Promise.allSettled([
    codexHome ? removeIsolatedCodexHome(codexHome) : Promise.resolve(),
    workspaceRoot
      ? rm(workspaceRoot, { recursive: true, force: true })
      : Promise.resolve(),
  ]);
  for (const result of removalResults) {
    if (result.status === "rejected") {
      cleanupFailures.push(result.reason);
    }
  }
  if (!primaryError && cleanupFailures.length > 0) {
    throw new AggregateError(cleanupFailures, "terminal capture cleanup failed");
  }
}
