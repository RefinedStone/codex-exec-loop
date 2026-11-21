import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { acquireHostLease, type PagePort } from "akra-tunnel-core/chat";
import { expect, test, vi } from "vitest";

import { DelegatedChatDriver } from "../src/main/delegatedChatDriver";

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
    expect(await driver.dispatch("status", "")).toEqual({ state: "ready" });
    manual = true;
    expect(await driver.dispatch("submit", "blocked", "Fixture")).toMatchObject({ delivery: "not_sent", phase: "blocked" });
    expect(await driver.dispatch("observe", "blocked")).toMatchObject({ phase: "blocked" });
    expect(sends).toEqual([]);
    manual = false;
    expect(await driver.dispatch("observe", "blocked")).toMatchObject({ delivery: "sending" });
    await vi.waitFor(async () => {
      expect(await driver.dispatch("submit", "blocked", "Fixture")).toMatchObject({ delivery: "accepted" });
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
