import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import type { ChildProcessWithoutNullStreams } from "node:child_process";
import { describe, expect, it, vi } from "vitest";
import { createProviderMigrationSource, type MigrationCommand } from "../src/main/providerMigrationSource";
import { PROFILE, SECOND_PROFILE, receipt, SECRET_CANARY } from "./providerFixtures";

class FakeChild extends EventEmitter {
  stdin = new PassThrough(); stdout = new PassThrough(); stderr = new PassThrough();
  chunks: Buffer[] = [];
  kill = vi.fn(() => true);
  constructor() { super(); this.stdin.on("data", (chunk: Buffer) => this.chunks.push(Buffer.from(chunk))); }
  get header(): Record<string, unknown> { return JSON.parse(this.chunks[0]!.toString("utf8")); }
  reply(value: object) { this.stdout.write(JSON.stringify({ version: 1, id: this.header.id, ...value }) + "\n"); }
  close(code = 0) { this.emit("close", code); }
}
const offer: Extract<MigrationCommand, { command: "offer" }> = {
  command: "offer", profileId: PROFILE, provider: "linear", operationId: "migration-1",
  expectedRevision: 0, configuration: {}, snapshotId: "d".repeat(64),
};
function setup(lifetimeMs = 5_000) {
  const child = new FakeChild();
  const source = createProviderMigrationSource({ executable: () => "fixed-helper.exe",
    spawnHelper: () => child as unknown as ChildProcessWithoutNullStreams, lifetimeMs });
  const secret = vi.fn(() => Buffer.from(SECRET_CANARY));
  return { child, source, secret };
}
function prepared(child: FakeChild, profileId = PROFILE) {
  child.reply({ kind: "prepared", profileId, operationId: offer.operationId, expectedRevision: 0 });
}
function committed(child: FakeChild) {
  child.reply({ kind: "committed", profileId: PROFILE, operationId: offer.operationId,
    receipt: receipt({ operationId: offer.operationId, kind: "legacyImport", legacySnapshotId: offer.snapshotId, legacyCleanupAllowed: true }) });
}

describe("private native credential helper channel", () => {
  it("decrypts only after verified preparation, writes one raw frame plus EOF, and waits for normal exit", async () => {
    const h = setup(); const result = h.source.run(offer, h.secret);
    expect(h.secret).not.toHaveBeenCalled();
    expect(h.child.header).toEqual({ version: 1, id: expect.any(String), ...offer });
    expect(JSON.stringify(h.child.header)).not.toContain(SECRET_CANARY);
    prepared(h.child);
    await vi.waitFor(() => expect(h.child.stdin.writableEnded).toBe(true));
    expect(h.secret).toHaveBeenCalledTimes(1);
    const frame = Buffer.concat(h.child.chunks.slice(1));
    expect(frame.readUInt32BE()).toBe(Buffer.byteLength(SECRET_CANARY));
    expect(frame.subarray(4).toString()).toBe(SECRET_CANARY);
    committed(h.child);
    let resolved = false; void result.then(() => { resolved = true; });
    await Promise.resolve();
    expect(resolved).toBe(false);
    h.child.close();
    expect(await result).toMatchObject({ kind: "committed", receipt: { operationId: offer.operationId } });
  });

  it("rejects a mismatched owner/profile preparation before decryption", async () => {
    const h = setup(); const result = h.source.run(offer, h.secret);
    prepared(h.child, SECOND_PROFILE);
    expect(await result).toEqual({ kind: "held", reason: "unavailable" });
    h.child.close();
    expect(h.secret).not.toHaveBeenCalled();
    expect(h.child.chunks).toHaveLength(1);
  });

  it.each(["exit", "truncated", "extra"])("preserves uncertain submitted credentials on %s terminal failure", async failure => {
    const h = setup(); const result = h.source.run(offer, h.secret);
    prepared(h.child);
    await vi.waitFor(() => expect(h.child.stdin.writableEnded).toBe(true));
    if (failure !== "truncated") committed(h.child);
    else h.child.stdout.write('{"version":1');
    if (failure === "extra") h.child.stdout.write("extra");
    h.child.close(failure === "exit" ? 1 : 0);
    expect(await result).toEqual({ kind: "unknown", operationId: offer.operationId });
  });

  it("fails closed on extra secret fields and never includes diagnostics in its result", async () => {
    const h = setup(); const result = h.source.run(offer, h.secret);
    h.child.stderr.write(SECRET_CANARY);
    h.child.reply({ kind: "held", reason: "unavailable", token: SECRET_CANARY });
    expect(await result).toEqual({ kind: "held", reason: "unavailable" });
    h.child.close();
    expect(h.secret).not.toHaveBeenCalled();
  });

  it.each(["failed", "cancelled", "superseded"] as const)("accepts a correlated durable %s before secret submission", async status => {
    const h = setup(); const result = h.source.run(offer, h.secret);
    h.child.reply({ kind: "operation", profileId: PROFILE, operationId: offer.operationId,
      receipt: receipt({ operationId: offer.operationId, kind: "legacyImport", status,
        connection: null, error: "cancelled", legacySnapshotId: offer.snapshotId }) });
    h.child.close();
    expect(await result).toMatchObject({ kind: "operation", receipt: { status } });
    expect(h.secret).not.toHaveBeenCalled();
  });

  it("accepts only the matching profile on the native Unknown wire response", async () => {
    const h = setup();
    const result = h.source.run({ command: "operation", provider: "linear", profileId: PROFILE, operationId: offer.operationId });
    h.child.reply({ kind: "unknown", profileId: PROFILE, operationId: offer.operationId }); h.child.close();
    expect(await result).toEqual({ kind: "unknown", operationId: offer.operationId });
    const wrong = setup();
    const rejected = wrong.source.run({ command: "operation", provider: "linear", profileId: PROFILE, operationId: offer.operationId });
    wrong.child.reply({ kind: "unknown", profileId: SECOND_PROFILE, operationId: offer.operationId }); wrong.child.close();
    expect(await rejected).toEqual({ kind: "held", reason: "unavailable" });
  });

  it("keeps operation and cleanup queries secret-free and accepts an authenticated NotFound", async () => {
    const h = setup();
    const result = h.source.run({ command: "operation", provider: "linear", profileId: PROFILE, operationId: offer.operationId });
    expect(h.child.stdin.writableEnded).toBe(true);
    h.child.reply({ kind: "held", reason: "notFound" }); h.child.close();
    expect(await result).toEqual({ kind: "held", reason: "notFound" });
    expect(h.child.chunks).toHaveLength(1);
  });

  it("requires the exact validated external URL and normal helper exit", async () => {
    const h = setup();
    const command: MigrationCommand = { command: "validateLink",
      reference: { profileId: PROFILE, provider: "jira", connectionId: "c".repeat(32), revision: 1 },
      url: "https://jira.example/browse/APP-1" };
    const result = h.source.run(command);
    h.child.reply({ kind: "validatedLink", url: "https://other.example/browse/APP-1" }); h.child.close();
    expect(await result).toEqual({ kind: "held", reason: "unavailable" });
  });

  it("does not send oversized secrets and drains only a bounded amount of stdout", async () => {
    const h = setup(); const result = h.source.run(offer, () => Buffer.alloc(65_537, 97));
    prepared(h.child);
    expect(await result).toEqual({ kind: "held", reason: "tooLarge" }); h.child.close();
    expect(h.child.chunks).toHaveLength(1);
    const next = setup(); const output = next.source.run(offer, next.secret);
    next.child.stdout.write(Buffer.alloc(16_385, 32));
    expect(await output).toEqual({ kind: "held", reason: "unavailable" }); next.child.close();
    expect(next.secret).not.toHaveBeenCalled();
  });

  it.each([false, true])("aborts only its own helper and preserves ambiguity after submitted=%s", async submitted => {
    const h = setup(); const controller = new AbortController();
    const result = h.source.run(offer, h.secret, controller.signal);
    if (submitted) { prepared(h.child); await vi.waitFor(() => expect(h.child.stdin.writableEnded).toBe(true)); }
    controller.abort();
    expect(await result).toEqual(submitted
      ? { kind: "unknown", operationId: offer.operationId } : { kind: "held", reason: "unavailable" });
    h.child.close();
  });
});
