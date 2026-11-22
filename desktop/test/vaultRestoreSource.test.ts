// native 실행·키 저장소를 쓰지 않고 stdio 신뢰 경계와 프로세스 수명만 검증한다.
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import type { ChildProcessWithoutNullStreams } from "node:child_process";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createVaultRestoreSource } from "../src/main/vaultRestoreSource";
import { copyVaultRestoreTarget, type VaultRestoreTarget } from "../src/shared/vaultRestore";
import type { VaultRestoreChannel } from "../src/main/vaultRestoreSession";

const TARGET: VaultRestoreTarget = {
  workspace: "C:/fixture/workspace",
  issueUrl: "https://github.com/fixture-owner/fixture-repo/issues/7",
  issue: { provider: "github", server_id: "github.com", repository_id: "R_fixture", issue_id: "I_fixture",
    project_scope: { workspace_dir: "C:/fixture/workspace" } },
};
const UNAVAILABLE = { kind: "held", reason: "source_unavailable" };
const RESTORED = { kind: "restored", title: "fixture 제목", body: "fixture-private-body" };

class Helper extends EventEmitter {
  stdin = new PassThrough();
  stdout = new PassThrough();
  stderr = new PassThrough();
  kill = vi.fn(() => true);
  requests: Record<string, unknown>[] = [];
  private input = "";
  constructor() {
    super();
    this.stdin.on("data", (data: Buffer) => {
      this.input += data.toString("utf8");
      while (this.input.includes("\n")) {
        const end = this.input.indexOf("\n");
        this.requests.push(JSON.parse(this.input.slice(0, end)));
        this.input = this.input.slice(end + 1);
      }
    });
  }
  frame(result: unknown, overrides: Record<string, unknown> = {}): string {
    return JSON.stringify({ version: 1, id: this.requests[0]?.id, result, ...overrides }) + "\n";
  }
  reply(result: unknown, overrides: Record<string, unknown> = {}): void {
    this.stdout.write(this.frame(result, overrides));
  }
  exit(code = 0): void { this.emit("close", code, null); }
}

const fixtures: { channel: VaultRestoreChannel; helper: Helper }[] = [];
function setup(target = TARGET) {
  const helper = new Helper();
  const spawnHelper = vi.fn(() => helper as unknown as ChildProcessWithoutNullStreams);
  const executable = vi.fn(() => "C:/trusted/akra-gateway.exe");
  const source = createVaultRestoreSource({ executable, spawnHelper });
  const channel = source(target)!;
  fixtures.push({ channel, helper });
  return { helper, spawnHelper, executable, source, channel };
}
async function prepared(fixture: ReturnType<typeof setup>): Promise<void> {
  const pending = fixture.channel.prepare();
  fixture.helper.reply({ kind: "prepared" });
  expect(await pending).toEqual({ kind: "prepared" });
}
afterEach(() => {
  for (const fixture of fixtures.splice(0)) {
    fixture.channel.close();
    fixture.helper.exit();
  }
  vi.useRealTimers();
});

describe("native Vault restore source", () => {
  it("binds native preparation to all coordinates, then sends only the same id to restore", async () => {
    const fixture = setup();
    await prepared(fixture);
    expect(fixture.executable).toHaveBeenCalledTimes(1);
    expect(fixture.spawnHelper).toHaveBeenCalledWith("C:/trusted/akra-gateway.exe", TARGET.workspace);
    const request = fixture.helper.requests[0]!;
    expect(request).toEqual({
      version: 1, id: expect.stringMatching(/^[a-zA-Z0-9-]{1,64}$/), command: "prepare",
      issue_url: TARGET.issueUrl, issue: TARGET.issue,
    });
    const pending = fixture.channel.restore();
    expect(fixture.helper.requests[1]).toEqual({ version: 1, id: request.id, command: "restore" });
    let settled = false;
    void pending.then(() => { settled = true; });
    fixture.helper.reply(RESTORED);
    await Promise.resolve();
    expect(settled).toBe(false);
    expect(fixture.helper.stdin.writableEnded).toBe(true);
    fixture.helper.exit();
    expect(await pending).toEqual(RESTORED);
    expect(fixture.channel.capabilities.reconnect).toBe(false);
  });

  it("captures a full target snapshot before either native step starts", async () => {
    const target = copyVaultRestoreTarget(TARGET);
    const fixture = setup(target);
    target.workspace = "C:/other";
    Object.assign(target.issue.project_scope, { workspace_dir: "C:/other" });
    Object.assign(target.issue, { issue_id: "I_other" });
    target.issueUrl = "https://github.com/other/repo/issues/8";
    await prepared(fixture);
    expect(fixture.helper.requests[0]!.issue).toEqual(TARGET.issue);
    expect(fixture.helper.requests[0]!.issue_url).toBe(TARGET.issueUrl);
    expect(fixture.spawnHelper).toHaveBeenCalledWith(expect.any(String), TARGET.workspace);
  });

  it("does not spawn for an invalid target or an out-of-order restore", async () => {
    const fixture = setup();
    expect(fixture.source({ ...TARGET, workspace: "C:/other" })).toBeNull();
    expect(await fixture.channel.restore()).toEqual(UNAVAILABLE);
    expect(fixture.spawnHelper).not.toHaveBeenCalled();
  });

  it.each(["platform_unsupported", "confirmation_cancelled", "not_bound", "record_unavailable"] as const)(
    "returns the supported native hold %s only after normal helper termination", async (reason) => {
      const { helper, channel } = setup();
      const pending = channel.prepare();
      helper.reply({ kind: "held", reason });
      expect(helper.stdin.writableEnded).toBe(true);
      helper.exit();
      expect(await pending).toEqual({ kind: "held", reason });
      expect(helper.requests).toHaveLength(1);
    },
  );

  it.each([
    ["wrong version", { kind: "prepared" }, { version: 2 }],
    ["wrong request id", { kind: "prepared" }, { id: "another-request" }],
    ["extra envelope data", { kind: "prepared" }, { approved: true }],
    ["extra result data", { kind: "prepared", approved: true }, {}],
    ["plaintext before preparation", RESTORED, {}],
    ["unknown hold reason", { kind: "held", reason: "fixture-private-error" }, {}],
    ["raw helper error", { kind: "error", message: "fixture-private-error" }, {}],
    ["missing result kind", { title: "fixture-private-error" }, {}],
  ])("rejects %s without relaying raw data", async (_name, result, overrides) => {
    const { helper, channel } = setup();
    const pending = channel.prepare();
    helper.reply(result, overrides);
    expect(await pending).toEqual(UNAVAILABLE);
    expect(helper.stdin.writableEnded).toBe(true);
  });

  it("rejects extra frames delivered in one chunk", async () => {
    const { helper, channel } = setup();
    const pending = channel.prepare();
    helper.stdout.write(helper.frame({ kind: "prepared" }) + helper.frame(RESTORED));
    expect(await pending).toEqual(UNAVAILABLE);
  });

  it.each(["extra output", "nonzero exit", "truncated JSON", "malformed UTF-8"] as const)(
    "drops restored data on %s", async (failure) => {
      const fixture = setup();
      await prepared(fixture);
      const pending = fixture.channel.restore();
      if (failure === "truncated JSON") {
        fixture.helper.stdout.write('{"version":1,"result":{"kind":"restored"');
      } else if (failure === "malformed UTF-8") {
        fixture.helper.stdout.write(Buffer.from([0xff, 0x0a]));
      } else {
        fixture.helper.reply(RESTORED);
        if (failure === "extra output") fixture.helper.stdout.write("fixture-private-error");
      }
      fixture.helper.exit(failure === "nonzero exit" ? 1 : 0);
      expect(await pending).toEqual(UNAVAILABLE);
    },
  );

  it("decodes split UTF-8 only after the full bounded line arrives", async () => {
    const fixture = setup();
    await prepared(fixture);
    const pending = fixture.channel.restore();
    const bytes = Buffer.from(fixture.helper.frame(RESTORED));
    const boundary = bytes.indexOf(Buffer.from("제")) + 1;
    fixture.helper.stdout.write(bytes.subarray(0, boundary));
    fixture.helper.stdout.write(bytes.subarray(boundary));
    fixture.helper.exit();
    expect(await pending).toEqual(RESTORED);
  });

  it("rejects a helper output exceeding the 8 MiB budget before a newline", async () => {
    const { helper, channel } = setup();
    const pending = channel.prepare();
    helper.stdout.write(Buffer.alloc(8 * 1024 * 1024 + 1, 0x61));
    expect(await pending).toEqual(UNAVAILABLE);
  });

  it("closes stdin immediately on cancellation and bounds process cleanup", async () => {
    vi.useFakeTimers();
    const { helper, channel } = setup();
    const pending = channel.prepare();
    channel.close();
    expect(await pending).toEqual(UNAVAILABLE);
    expect(helper.stdin.writableEnded).toBe(true);
    vi.advanceTimersByTime(1_000);
    expect(helper.kill).toHaveBeenCalledWith("SIGTERM");
    vi.advanceTimersByTime(1_000);
    expect(helper.kill).toHaveBeenCalledWith("SIGKILL");
    helper.reply({ kind: "prepared" });
    expect(await channel.restore()).toEqual(UNAVAILABLE);
    expect(helper.requests).toHaveLength(1);
  });

  it("expires a stuck confirmation after five minutes and starts cleanup", async () => {
    vi.useFakeTimers();
    const { helper, channel } = setup();
    const pending = channel.prepare();
    vi.advanceTimersByTime(5 * 60_000);
    expect(await pending).toEqual({ kind: "held", reason: "locked" });
    expect(helper.stdin.writableEnded).toBe(true);
  });

  it("discards stderr and normalizes executable, spawn, and pipe errors", async () => {
    for (const dependency of [
      { executable: () => { throw new Error("fixture-private-error"); } },
      { executable: () => "C:/trusted/helper", spawnHelper: () => { throw new Error("fixture-private-error"); } },
    ]) {
      const channel = createVaultRestoreSource(dependency)(TARGET)!;
      expect(await channel.prepare()).toEqual(UNAVAILABLE);
    }
    for (const pipe of ["stdin", "stdout", "stderr"] as const) {
      const { helper, channel } = setup();
      const pending = channel.prepare();
      helper.stderr.write("fixture-private-error");
      helper[pipe].emit("error", new Error("fixture-private-error"));
      expect(await pending).toEqual(UNAVAILABLE);
    }
  });
});
