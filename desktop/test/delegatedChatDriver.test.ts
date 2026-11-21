import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { acquireHostLease, type PagePort } from "akra-tunnel-core/chat";
import { expect, test, vi } from "vitest";

import { DelegatedChatDriver } from "../src/main/delegatedChatDriver";

test("reports observed progress and collected text through the trusted job handler", async () => {
  const home = await mkdtemp(join(tmpdir(), "akra-chat-test-"));
  let phase = "running";
  const insertText = vi.fn();
  const driver = new DelegatedChatDriver({
    home,
    manual: () => false,
    closePage: () => {},
    pageFor: () => ({
      evaluate: async (_fn: unknown, args: { action: string }) => {
        if (args.action === "focus") return { ready: true };
        if (args.action === "send") return { clicked: true };
        if (args.action === "observe") return { phase, markerObserved: true, assistantProgress: true, result: "Fixture answer" };
        throw new Error("Unexpected operation");
      },
      insertText,
      focus: () => {},
      load: async () => {},
      url: () => "https://chatgpt.com/",
    } as PagePort),
  });
  try {
    const report = vi.fn();
    await driver.dispatch("submit", "attempt", "Fixture prompt", undefined, report);
    await vi.waitFor(async () => {
      expect(await driver.dispatch("observe", "attempt", undefined, undefined, report))
        .toMatchObject({ phase: "working", delivery: "observed" });
    });
    expect(report).toHaveBeenCalledWith({ type: "checkpoint", message: "Assistant progress observed in the delegated chat." });
    phase = "complete";
    await driver.dispatch("observe", "attempt", undefined, undefined, report);
    expect(await driver.dispatch("readResult", "attempt", undefined, undefined, report)).toEqual({ text: "Fixture answer" });
    expect(report).toHaveBeenLastCalledWith({ type: "result", result: "Fixture answer" });
    expect(insertText).toHaveBeenCalledExactlyOnceWith(expect.stringMatching(/^\[akra-chat:[a-f0-9-]+\]\n\nFixture prompt$/));
  } finally {
    await driver.close();
    await rm(home, { recursive: true, force: true });
  }
});

test("the chat host serializes fresh sessions, supports manual pauses and cancels queued jobs", async () => {
  const home = await mkdtemp(join(tmpdir(), "akra-chat-test-"));
  const sends: string[] = [];
  let manual = false;
  const driver = new DelegatedChatDriver({
    home,
    manual: () => manual,
    closePage: () => {},
    pageFor(id) {
      const page = {
        evaluate: async (_fn: unknown, args: { action: string }) => {
          if (args.action === "focus") return { ready: true };
          if (args.action === "send") {
            sends.push(id);
            return { clicked: true };
          }
          if (args.action === "observe") return { phase: "complete", markerObserved: true, result: "fixture result" };
          throw new Error("Unexpected operation");
        },
        insertText: () => {},
        focus: () => {},
        load: async () => {},
        url: () => "https://chatgpt.com/",
      };
      return page as PagePort;
    },
  });
  try {
    const other = await acquireHostLease(home);
    try {
      expect(await driver.dispatch("status", "")).toMatchObject({ state: "unavailable", reason: "chat_host_busy" });
      expect(await driver.dispatch("submit", "busy", "Fixture")).toMatchObject({ delivery: "not_sent" });
      expect(sends).toEqual([]);
    } finally {
      await other?.release();
    }
    expect(await driver.dispatch("status", "")).toMatchObject({
      state: "ready", accountMatch: "unknown", tunnel: { host: true, readiness: "unknown", manual: false },
    });
    manual = true;
    expect(await driver.dispatch("submit", "blocked", "Fixture")).toMatchObject({ delivery: "not_sent", phase: "blocked" });
    expect(await driver.dispatch("submit", "blocked", "Fixture")).toMatchObject({ delivery: "not_sent", phase: "blocked" });
    expect(await driver.dispatch("observe", "blocked")).toMatchObject({ phase: "blocked" });
    expect(sends).toEqual([]);
    manual = false;
    expect(await driver.dispatch("observe", "blocked")).toMatchObject({ delivery: "sending" });
    await vi.waitFor(async () => {
      expect(await driver.dispatch("observe", "blocked")).toMatchObject({ delivery: "observed", phase: "complete" });
    });
    expect(await driver.dispatch("observe", "blocked")).toMatchObject({ phase: "complete" });
    expect(sends.splice(0)).toEqual(["blocked"]);
    expect(await driver.dispatch("submit", "first", "Fixture 1")).toEqual({ delivery: "not_sent", phase: "unknown" });
    expect(await driver.dispatch("submit", "second", "Fixture 2")).toEqual({ delivery: "not_sent", phase: "unknown" });
    expect(await driver.dispatch("submit", "cancelled", "Fixture 3")).toEqual({ delivery: "not_sent", phase: "unknown" });
    expect(await driver.dispatch("stop", "cancelled")).toEqual({});
    await vi.waitFor(async () => {
      expect(await driver.dispatch("submit", "first", "Fixture 1")).toMatchObject({ delivery: "accepted" });
    });
    expect(sends).toEqual(["first"]);
    manual = true;
    expect(await driver.dispatch("observe", "first")).toMatchObject({ delivery: "accepted", phase: "blocked" });
    manual = false;
    expect(await driver.dispatch("observe", "first")).toMatchObject({ delivery: "observed", phase: "complete" });
    expect(await driver.dispatch("readResult", "first")).toEqual({ text: "fixture result" });
    expect(await driver.dispatch("observe", "second")).toMatchObject({ delivery: "sending" });
    await vi.waitFor(async () => {
      expect(await driver.dispatch("submit", "second", "Fixture 2")).toMatchObject({ delivery: "accepted" });
    });
    expect(sends).toEqual(["first", "second"]);
  } finally {
    await driver.close();
    await rm(home, { recursive: true, force: true });
  }
});

test("continue requires a verified account and recovery never resends the text", async () => {
  const home = await mkdtemp(join(tmpdir(), "akra-chat-test-"));
  const href = "https://chatgpt.com/c/12345678-1234-1234-1234-123456789abc";
  let url = "https://chatgpt.com/";
  const navigate = vi.fn(async (destination: string) => { url = destination; });
  const send = vi.fn();
  const driver = new DelegatedChatDriver({
    home,
    manual: () => false,
    closePage: () => {},
    pageStatus: async (expectedId) => ({
      readiness: "ready",
      accountMatch: expectedId === "fixture-account" ? "match" : expectedId ? "mismatch" : "unknown",
    }),
    pageFor: () => ({
      evaluate: async (_fn: unknown, args: { action: string }) => {
        if (args.action === "focus") return { ready: true };
        if (args.action === "send") {
          send();
          return { clicked: true };
        }
        if (args.action === "observe") return { phase: "disconnected", markerObserved: true, chatUrl: href };
        throw new Error("Unexpected operation");
      },
      insertText: () => {},
      focus: () => {},
      load: navigate,
      url: () => url,
    } as PagePort),
  });
  const session = { mode: "continue", chatUrl: href } as const;
  try {
    expect(await driver.dispatch("status", "", undefined, { expectedAccountId: "other-account" })).toEqual({
      state: "ready", accountMatch: "mismatch", tunnel: { host: true, readiness: "ready", manual: false },
    });
    for (const [id, expectedAccountId, reason] of [
      ["wrong", "other-account", "chat_account_mismatch"],
      ["unknown", undefined, "chat_account_unknown"],
    ] as const) {
      await driver.dispatch("submit", id, "Fixture", { session, expectedAccountId });
      await vi.waitFor(async () => {
        expect(await driver.dispatch("observe", id)).toMatchObject({ delivery: "not_sent", phase: "blocked", reason });
      });
    }
    expect(send).not.toHaveBeenCalled();
    expect(navigate).not.toHaveBeenCalled();
    const options = { session, expectedAccountId: "fixture-account", maxResumes: 2 };
    await driver.dispatch("submit", "matched", "Fixture", options);
    await vi.waitFor(async () => {
      expect(await driver.dispatch("submit", "matched", "Fixture", options)).toMatchObject({ delivery: "accepted" });
    });
    expect(await driver.dispatch("observe", "matched")).toMatchObject({
      delivery: "observed", phase: "disconnected", recovery: "in_progress", resumes: 1, conversationUrl: href,
    });
    expect(await driver.dispatch("observe", "matched")).toMatchObject({ recovery: "exhausted", resumes: 2 });
    expect(await driver.dispatch("observe", "matched")).toMatchObject({ recovery: "exhausted", resumes: 2 });
    expect(send).toHaveBeenCalledTimes(1);
    expect(navigate.mock.calls).toEqual([[href], [href], [href]]);
  } finally {
    await driver.close();
    await rm(home, { recursive: true, force: true });
  }
});
