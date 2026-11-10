// 실측 녹화에서 만든 픽스처가 형식에 맞고, 개인 정보가 남지 않았는지 확인한다.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { loadReplayTable } from "./fake-gateway/fakeGateway";

const fixture = (name: string) => fileURLToPath(new URL(`./fake-gateway/fixtures/${name}`, import.meta.url));

interface RecordedLine {
  direction: string;
  text: string;
}

function methods(name: string): string[] {
  return readFileSync(fixture(name), "utf8")
    .split(/\r?\n/)
    .filter((line) => line.trim().length > 0)
    .map((line) => JSON.parse((JSON.parse(line) as RecordedLine).text) as { method?: string })
    .flatMap((message) => (message.method ? [message.method] : []));
}

describe("recorded fixtures", () => {
  it("approval-turn covers a replayed approval, command output, streaming and completion", () => {
    const seen = methods("approval-turn.jsonl");
    for (const method of [
      "item/commandExecution/requestApproval",
      "serverRequest/resolved",
      "item/commandExecution/outputDelta",
      "item/agentMessage/delta",
      "turn/completed",
    ]) {
      expect(seen).toContain(method);
    }
    expect(loadReplayTable(fixture("approval-turn.jsonl")).has("thread/resume")).toBe(true);
  });

  it("interrupted-turn covers streaming, interrupt and archive", () => {
    const seen = methods("interrupted-turn.jsonl");
    for (const method of ["turn/started", "item/agentMessage/delta", "turn/interrupt", "turn/completed", "thread/archived"]) {
      expect(seen).toContain(method);
    }
  });

  it("thread-start answers a new thread without turns", () => {
    expect(methods("thread-start.jsonl")).toEqual(["thread/start"]);
    expect(loadReplayTable(fixture("thread-start.jsonl")).has("thread/start")).toBe(true);
  });

  it("contains no local paths or account data", () => {
    for (const name of ["approval-turn.jsonl", "interrupted-turn.jsonl", "thread-start.jsonl"]) {
      const text = readFileSync(fixture(name), "utf8");
      expect(text).not.toMatch(/[A-Za-z]:\\\\|\/Users\/|\/home\/[a-z]|@[a-z0-9-]+\.[a-z]/i);
      expect(text).not.toMatch(/"account\/|rateLimits/);
    }
  });
});
