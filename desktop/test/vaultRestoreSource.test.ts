// native 실행·키 저장소를 쓰지 않고 stdio 신뢰 경계와 프로세스 수명만 검증한다.
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import type { ChildProcessWithoutNullStreams } from "node:child_process";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createVaultRestoreSource } from "../src/main/vaultRestoreSource";
import { copyVaultRestoreTarget, type VaultRestoreTarget } from "../src/shared/vaultRestore";
import { VaultRestoreSession, type VaultRestoreChannel } from "../src/main/vaultRestoreSession";

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
    return JSON.stringify({ version: 2, id: this.requests[0]?.id, result, ...overrides }) + "\n";
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
  fixture.helper.reply({ kind: "prepared", capabilities: { reconnect: true } });
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
      version: 2, id: expect.stringMatching(/^[a-zA-Z0-9-]{1,64}$/), command: "prepare",
      issue_url: TARGET.issueUrl, issue: TARGET.issue,
    });
    const pending = fixture.channel.restore();
    expect(fixture.helper.requests[1]).toEqual({ version: 2, id: request.id, command: "restore" });
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
      helper.reply({ kind: "held", reason, can_reconnect: false });
      expect(helper.stdin.writableEnded).toBe(true);
      helper.exit();
      expect(await pending).toEqual({ kind: "held", reason });
      expect(helper.requests).toHaveLength(1);
    },
  );

  it.each([
    ["wrong version", { kind: "prepared", capabilities: { reconnect: true } }, { version: 1 }],
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
    helper.stdout.write(helper.frame({ kind: "prepared", capabilities: { reconnect: true } }) + helper.frame(RESTORED));
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
    helper.reply({ kind: "prepared", capabilities: { reconnect: true } });
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

describe("native reconnect v2", () => {
  it("keeps one native helper through changed copy review, reconnect, and final restore", async () => {
    const { helper, channel } = setup();
    const prepare = channel.prepare();
    helper.reply({ kind: "prepared", capabilities: { reconnect: true } }, { version: 2 });
    expect(await prepare).toEqual({ kind: "prepared" });
    expect(channel.capabilities.reconnect).toBe(true);
    const firstRestore = channel.restore();
    helper.reply({ kind: "held", reason: "marker_duplicated", can_reconnect: true }, { version: 2 });
    expect(await firstRestore).toEqual({ kind: "held", reason: "marker_duplicated", canReconnect: true });
    expect(helper.stdin.writableEnded).toBe(false);
    const reviewing = channel.review();
    const reviewedHash = "1:" + "a".repeat(64);
    helper.reply({ kind: "review", review: {
      reviewed_hash: reviewedHash, published_title: "fixture", published_body: "[[mask]] [[mask]]",
      markers: [{ marker: "[[mask]]", previous: { field: "body", position: 100 },
        candidates: [{ field: "body", position: 0, context: "[[mask]]" }, { field: "body", position: 9, context: "[[mask]]" }] }],
    } }, { version: 2 });
    const review = await reviewing;
    expect(review.kind).toBe("review");
    const reconnecting = channel.reconnect(reviewedHash, [{ marker: "[[mask]]", field: "body", position: 9 }]);
    helper.reply({ kind: "reconnected" }, { version: 2 });
    expect(await reconnecting).toEqual({ kind: "reconnected" });
    const finalRestore = channel.restore();
    helper.reply(RESTORED, { version: 2 });
    helper.exit();
    expect(await finalRestore).toEqual(RESTORED);
    expect(helper.requests.map((request) => request.command)).toEqual(["prepare", "restore", "review", "reconnect", "restore"]);
  });
});


function wireReview() {
  return {
    reviewed_hash: "1:" + "a".repeat(64), published_title: "fixture",
    published_body: "가 [[mask]]",
    markers: [{ marker: "[[mask]]", previous: { field: "body", position: 500 },
      candidates: [{ field: "body", position: 4, context: "가 [[mask]]" }] }],
  };
}
async function reviewingFixture() {
  const fixture = setup();
  await prepared(fixture);
  const first = fixture.channel.restore();
  fixture.helper.reply({ kind: "held", reason: "marker_moved", can_reconnect: true });
  expect(await first).toEqual({ kind: "held", reason: "marker_moved", canReconnect: true });
  const reviewing = fixture.channel.review();
  return { ...fixture, reviewing };
}
async function reviewedFixture(review = wireReview()) {
  const fixture = await reviewingFixture();
  fixture.helper.reply({ kind: "review", review });
  expect((await fixture.reviewing).kind).toBe("review");
  return fixture;
}

describe("native reconnect wire boundaries", () => {
  it.each([
    { kind: "prepared" },
    { kind: "prepared", capabilities: {} },
    { kind: "prepared", capabilities: { reconnect: true, approved: true } },
    { kind: "held", reason: "marker_moved" },
    { kind: "held", reason: "locked", can_reconnect: true },
  ])("rejects missing or inconsistent capability metadata: %j", async (result) => {
    const { helper, channel } = setup();
    const pending = channel.prepare();
    helper.reply(result);
    expect(await pending).toEqual(UNAVAILABLE);
  });

  it("uses byte positions and accepts a previous position beyond the new published text", async () => {
    const fixture = await reviewingFixture();
    fixture.helper.reply({ kind: "review", review: wireReview() });
    expect(await fixture.reviewing).toEqual({ kind: "review", review: {
      reviewedHash: "1:" + "a".repeat(64), publishedTitle: "fixture", publishedBody: "가 [[mask]]",
      markers: [{ marker: "[[mask]]", previous: { field: "body", position: 500 },
        candidates: [{ field: "body", position: 4, context: "가 [[mask]]" }] }],
    } });
  });

  const malformed: [string, (review: ReturnType<typeof wireReview>) => void][] = [
    ["leading zero hash version", (review) => { review.reviewed_hash = "01:" + "a".repeat(64); }],
    ["zero hash version", (review) => { review.reviewed_hash = "0:" + "a".repeat(64); }],
    ["overflow hash version", (review) => { review.reviewed_hash = "4294967296:" + "a".repeat(64); }],
    ["uppercase digest", (review) => { review.reviewed_hash = "1:" + "A".repeat(64); }],
    ["extra review field", (review) => { Object.assign(review, { approved: true }); }],
    ["extra marker field", (review) => { Object.assign(review.markers[0]!, { plaintext: "private" }); }],
    ["extra previous field", (review) => { Object.assign(review.markers[0]!.previous, { approved: true }); }],
    ["extra candidate field", (review) => { Object.assign(review.markers[0]!.candidates[0]!, { approved: true }); }],
    ["duplicate marker", (review) => { review.markers.push(structuredClone(review.markers[0]!)); }],
    ["duplicate candidate", (review) => { review.markers[0]!.candidates.push(structuredClone(review.markers[0]!.candidates[0]!)); }],
    ["fractional position", (review) => { review.markers[0]!.candidates[0]!.position = 4.5; }],
    ["negative previous position", (review) => { review.markers[0]!.previous.position = -1; }],
    ["unsafe position", (review) => { review.markers[0]!.previous.position = Number.MAX_SAFE_INTEGER + 1; }],
    ["character offset instead of byte offset", (review) => { review.markers[0]!.candidates[0]!.position = 2; }],
    ["different marker bytes", (review) => { review.markers[0]!.marker = "[[other]]"; }],
    ["invalid field", (review) => { review.markers[0]!.candidates[0]!.field = "comment"; }],
    ["unpaired surrogate", (review) => { review.published_title = "\ud800"; }],
  ];
  it.each(malformed)("rejects %s before exposing review data", async (_name, change) => {
    const fixture = await reviewingFixture();
    const review = wireReview();
    change(review);
    fixture.helper.reply({ kind: "review", review });
    expect(await fixture.reviewing).toEqual(UNAVAILABLE);
    expect(fixture.helper.requests.map((request) => request.command)).toEqual(["prepare", "restore", "review"]);
  });

  it("permits empty mappings and makes a final changed-copy hold terminal", async () => {
    const fixture = await reviewedFixture();
    const pending = fixture.channel.reconnect(wireReview().reviewed_hash, []);
    fixture.helper.reply({ kind: "reconnected" });
    expect(await pending).toEqual({ kind: "reconnected" });
    expect(fixture.helper.requests.at(-1)?.mappings).toEqual([]);
    const restoring = fixture.channel.restore();
    fixture.helper.reply({ kind: "held", reason: "marker_missing", can_reconnect: false });
    expect(fixture.helper.stdin.writableEnded).toBe(true);
    fixture.helper.exit();
    expect(await restoring).toEqual({ kind: "held", reason: "marker_missing" });
    expect(await fixture.channel.review()).toEqual(UNAVAILABLE);
    expect(fixture.helper.requests).toHaveLength(5);
  });

  it("rejects reconnectable holds outside the initial restoration", async () => {
    const fixture = await reviewingFixture();
    fixture.helper.reply({ kind: "held", reason: "marker_missing", can_reconnect: true });
    expect(await fixture.reviewing).toEqual(UNAVAILABLE);
    expect(fixture.helper.stdin.writableEnded).toBe(true);
  });

  it("does not offer reconnect when a native initial hold marks the attempt terminal", async () => {
    const fixture = setup();
    await prepared(fixture);
    const pending = fixture.channel.restore();
    fixture.helper.reply({ kind: "held", reason: "marker_moved", can_reconnect: false });
    fixture.helper.exit();
    expect(await pending).toEqual({ kind: "held", reason: "marker_moved" });
    expect(await fixture.channel.review()).toEqual(UNAVAILABLE);
    expect(fixture.helper.requests).toHaveLength(2);
  });

  it("refuses a changed review hash without sending a write request", async () => {
    const fixture = await reviewedFixture();
    expect(await fixture.channel.reconnect("1:" + "b".repeat(64), [])).toEqual({ kind: "held", reason: "changed_during_review" });
    expect(fixture.helper.requests).toHaveLength(3);
  });

  it("refuses extra mapping fields without sending a write request", async () => {
    const fixture = await reviewedFixture();
    const mapping = { marker: "[[mask]]", field: "body" as const, position: 4, approved: true };
    expect(await fixture.channel.reconnect(wireReview().reviewed_hash, [mapping])).toEqual({ kind: "held", reason: "mapping_rejected" });
    expect(fixture.helper.requests).toHaveLength(3);
  });

  it.each(["unexpected exit", "malformed response", "nonzero exit", "cancel"] as const)(
    "reports uncertain persistence on %s after a reconnect request, with no resend", async (failure) => {
      const fixture = await reviewedFixture();
      const pending = fixture.channel.reconnect(wireReview().reviewed_hash, []);
      if (failure === "malformed response") fixture.helper.stdout.write("{broken}\n");
      else if (failure === "cancel") fixture.channel.close();
      else if (failure === "nonzero exit") {
        fixture.helper.reply({ kind: "held", reason: "record_unavailable", can_reconnect: false });
        fixture.helper.exit(1);
      } else fixture.helper.exit();
      expect(await pending).toEqual({ kind: "held", reason: "write_outcome_unknown" });
      expect(await fixture.channel.reconnect(wireReview().reviewed_hash, [])).toEqual(UNAVAILABLE);
      expect(fixture.helper.requests.filter((request) => request.command === "reconnect")).toHaveLength(1);
    },
  );

  it("ends an oversized reconnect before writing and does not claim it was sent", async () => {
    const review = wireReview();
    review.published_body = "x".repeat(17 * 1024);
    review.markers[0]!.marker = review.published_body;
    review.markers[0]!.candidates[0]!.position = 0;
    const fixture = await reviewedFixture(review);
    const pending = fixture.channel.reconnect(review.reviewed_hash, [{ marker: review.published_body, field: "body", position: 0 }]);
    expect(await pending).toEqual(UNAVAILABLE);
    expect(fixture.helper.requests).toHaveLength(3);
  });

  it("applies the output budget across review and final restore together", async () => {
    const review = wireReview();
    review.published_body = "x".repeat(4 * 1024 * 1024);
    review.markers = [];
    const fixture = await reviewedFixture(review);
    const reconnecting = fixture.channel.reconnect(review.reviewed_hash, []);
    fixture.helper.reply({ kind: "reconnected" });
    expect(await reconnecting).toEqual({ kind: "reconnected" });
    const restoring = fixture.channel.restore();
    fixture.helper.reply({ kind: "restored", title: "", body: "p".repeat(4 * 1024 * 1024) });
    fixture.helper.exit();
    expect(await restoring).toEqual(UNAVAILABLE);
  });

  it("does not accept final plaintext before a successful helper exit", async () => {
    const fixture = await reviewedFixture();
    const reconnecting = fixture.channel.reconnect(wireReview().reviewed_hash, []);
    fixture.helper.reply({ kind: "reconnected" });
    await reconnecting;
    const restoring = fixture.channel.restore();
    fixture.helper.reply(RESTORED);
    fixture.helper.exit(1);
    expect(await restoring).toEqual(UNAVAILABLE);
  });
});


describe("native idle helper lifetime", () => {
  it.each(["held", "review"] as const)("immediately invalidates a %s session after unexpected helper exit", async (phase) => {
    const fixture = setup();
    const states: import("../src/shared/vaultRestore").VaultRestoreViewState[] = [];
    const session = new VaultRestoreSession({
      target: TARGET, source: () => fixture.channel, profileHold: () => null,
      emit: (state) => states.push(state), sessionMs: 60_000, now: () => 0,
    });
    try {
      const opening = session.unlock();
      fixture.helper.reply({ kind: "prepared", capabilities: { reconnect: true } });
      await vi.waitFor(() => expect(fixture.helper.requests.at(-1)?.command).toBe("restore"));
      fixture.helper.reply({ kind: "held", reason: "marker_moved", can_reconnect: true });
      await opening;
      if (phase === "review") {
        const reviewing = session.requestReview();
        fixture.helper.reply({ kind: "review", review: wireReview() });
        await reviewing;
      }
      expect(states.at(-1)?.phase).toBe(phase);
      const requestCount = fixture.helper.requests.length;
      fixture.helper.exit(1);
      expect(states.at(-1)).toEqual({ phase: "held", reason: "source_unavailable" });
      await session.requestReview();
      await session.reconnect([]);
      expect(fixture.helper.requests).toHaveLength(requestCount);
    } finally { session.lock("user"); }
  });

  it("keeps the final restored reply when the helper normally closes", async () => {
    const fixture = setup();
    const closed = vi.fn();
    fixture.channel.onClosed?.(closed);
    await prepared(fixture);
    const restoring = fixture.channel.restore();
    fixture.helper.reply(RESTORED);
    fixture.helper.exit();
    expect(await restoring).toEqual(RESTORED);
    expect(closed).not.toHaveBeenCalled();
  });

  it("notifies an idle review when the native lifetime expires", async () => {
    vi.useFakeTimers();
    const fixture = await reviewedFixture();
    const closed = vi.fn();
    fixture.channel.onClosed?.(closed);
    vi.advanceTimersByTime(5 * 60_000);
    expect(closed).toHaveBeenCalledExactlyOnceWith("locked");
    expect(fixture.helper.stdin.writableEnded).toBe(true);
    expect(await fixture.channel.reconnect(wireReview().reviewed_hash, [])).toEqual(UNAVAILABLE);
    expect(fixture.helper.requests).toHaveLength(3);
  });
});
