import { mkdtemp, readFile, writeFile, rm, rename, link, readdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LegacyCredentialFiles, type LegacySecretBox } from "../src/main/providerLegacy";
import { ProviderCredentialMigration } from "../src/main/providerCredentialMigration";
import type { MigrationCommand, NativeMigrationResult, ProviderMigrationSource } from "../src/main/providerMigrationSource";
import type { OperationReceipt, Provider } from "../src/shared/providerCredentials";
import { deferred, PROFILE, receipt, reference, SECOND_PROFILE, SECRET_CANARY } from "./providerFixtures";

const renameBarrier = vi.hoisted(() => ({ after: undefined as undefined | (() => Promise<void>) }));
vi.mock("node:fs/promises", async importOriginal => {
  const original = await importOriginal<typeof import("node:fs/promises")>();
  return { ...original, rename: async (...args: Parameters<typeof original.rename>) => {
    await original.rename(...args);
    await renameBarrier.after?.();
  } };
});

const directories: string[] = [];
afterEach(async () => { renameBarrier.after = undefined; for (const dir of directories.splice(0)) await rm(dir, { recursive: true, force: true }); });
const encrypt = (text: string) => Buffer.from("synthetic-sealed:" + Buffer.from(text).toString("base64"));
const target = { provider: "linear" as const, profileId: PROFILE, expectedRevision: 0 };
async function fixture(provider: Provider = "linear") {
  const directory = await mkdtemp(join(tmpdir(), "akra-provider-migration-test-")); directories.push(directory);
  const path = join(directory, provider + "-token.bin");
  const encrypted = encrypt(provider === "linear" ? JSON.stringify({ token: SECRET_CANARY, viewer: { id: "viewer", name: "Fixture" } }) : SECRET_CANARY);
  await writeFile(path, encrypted);
  if (provider === "jira") await writeFile(join(directory, "jira-connection.json"), JSON.stringify({ siteUrl: "https://jira.example/jira", deployment: "server", email: null }));
  const secrets: LegacySecretBox = {
    isEncryptionAvailable: () => true,
    decryptString: vi.fn(bytes => {
      const encoded = bytes.toString();
      if (!encoded.startsWith("synthetic-sealed:")) throw new Error("bad synthetic ciphertext");
      return Buffer.from(encoded.slice("synthetic-sealed:".length), "base64").toString("utf8");
    }),
  };
  const files = new LegacyCredentialFiles(directory);
  const commands: MigrationCommand[] = [];
  const receipts = new Map<string, OperationReceipt>();
  let handle: ProviderMigrationSource["run"] = async (command, secret) => {
    if (command.command === "offer") {
      const bytes = await secret!();
      expect(bytes.toString()).toBe(SECRET_CANARY); bytes.fill(0);
      const result = receipt({ operationId: command.operationId, profileId: command.profileId, provider: command.provider,
        kind: "legacyImport", expectedRevision: command.expectedRevision, legacySnapshotId: command.snapshotId,
        legacyCleanupAllowed: true, connection: reference(command.provider) });
      receipts.set(command.operationId, result);
      return { kind: "committed", receipt: result };
    }
    if (command.command === "validateLink") return { kind: "validatedLink", url: command.url };
    const current = receipts.get(command.operationId);
    if (!current) return { kind: "held", reason: "notFound" };
    if (command.command === "cleanup") {
      const result = { ...current, legacyCleanupAllowed: false };
      receipts.set(command.operationId, result);
      return { kind: "operation", receipt: result };
    }
    return { kind: current.legacyCleanupAllowed ? "committed" : "operation", receipt: current };
  };
  const source: ProviderMigrationSource = { run: async (command, secret, signal) => {
    commands.push(structuredClone(command));
    return handle(command, secret, signal);
  } };
  const openExternal = vi.fn(async (_url: string) => undefined);
  const dependencies = { files, secrets, source, openExternal };
  return { directory, path, encrypted, files, secrets, commands, receipts, openExternal,
    service: new ProviderCredentialMigration(dependencies), reopen: () => new ProviderCredentialMigration(dependencies),
    defaultRun: handle, handle: (run: ProviderMigrationSource["run"]) => { handle = run; } };
}

describe("Desktop legacy credential migration", () => {
  it("only inspects bounded encrypted files for availability and never decrypts before native preparation", async () => {
    const h = await fixture();
    expect(await h.service.legacyStatus("linear")).toEqual({ kind: "available" });
    expect(h.secrets.decryptString).not.toHaveBeenCalled();
    h.handle(async () => ({ kind: "held", reason: "unauthorized" }));
    expect(await h.service.migrateLegacy(target)).toEqual({ kind: "held", reason: "unauthorized" });
    expect(h.secrets.decryptString).not.toHaveBeenCalled();
    expect(await readFile(h.path)).toEqual(h.encrypted);
    expect(JSON.stringify(h.commands)).not.toContain(SECRET_CANARY);
  });

  it.each(["linear", "jira"] as const)("removes only the committed %s snapshot after readback and durable cleanup ACK", async provider => {
    const h = await fixture(provider);
    const result = await h.service.migrateLegacy({ ...target, provider });
    expect(result).toMatchObject({ kind: "committed", cleanupPending: false });
    expect(await readdir(h.directory)).toEqual([]);
    expect(h.secrets.decryptString).toHaveBeenCalledTimes(1);
    expect(h.commands.map(command => command.command)).toEqual(["offer", "cleanup"]);
    expect(JSON.stringify(result)).not.toContain(SECRET_CANARY);
    expect(JSON.stringify(h.commands)).not.toContain(SECRET_CANARY);
  });

  it("recovers an ACK-lost submitted import after a Desktop restart without decrypting or offering again", async () => {
    const h = await fixture();
    h.handle(async (command, secret, signal) => {
      const result = await h.defaultRun(command, secret, signal);
      return command.command === "offer" ? { kind: "unknown", operationId: command.operationId } : result;
    });
    const first = await h.service.migrateLegacy(target);
    expect(first.kind).toBe("unknown");
    expect(await readFile(h.path)).toEqual(h.encrypted);
    expect(await h.service.legacyStatus("linear")).toEqual({ kind: "pending" });
    const journal = await h.files.loadJournal("linear");
    expect(journal?.operationId).toBe("operationId" in first ? first.operationId : "");
    expect(JSON.stringify(journal)).not.toContain(SECRET_CANARY);
    const second = await h.reopen().migrateLegacy({ ...target, expectedRevision: 1 });
    expect(second).toMatchObject({ kind: "committed", cleanupPending: false });
    expect(h.commands.map(command => command.command)).toEqual(["offer", "operation", "cleanup"]);
    expect(h.secrets.decryptString).toHaveBeenCalledTimes(1);
    expect(await readdir(h.directory)).toEqual([]);
  });

  it("preserves the cleanup journal after ACK loss even when ciphertext has already been deleted", async () => {
    const h = await fixture(); let lose = true;
    h.handle(async (command, secret, signal) => {
      const result = await h.defaultRun(command, secret, signal);
      if (command.command === "cleanup" && lose) { lose = false; return { kind: "unknown", operationId: command.operationId }; }
      return result;
    });
    expect(await h.service.migrateLegacy(target)).toMatchObject({ kind: "committed", cleanupPending: true });
    await expect(readFile(h.path)).rejects.toMatchObject({ code: "ENOENT" });
    expect(await h.files.loadJournal("linear")).not.toBeNull();
    expect(await h.reopen().migrateLegacy({ ...target, expectedRevision: 1 })).toMatchObject({ kind: "committed", cleanupPending: false });
    expect(h.commands.filter(command => command.command === "offer")).toHaveLength(1);
    expect(await readdir(h.directory)).toEqual([]);
  });

  it.each(["bytes", "identity"])("keeps a concurrently replaced ciphertext when its %s changed", async kind => {
    const h = await fixture();
    h.handle(async (command, secret, signal) => {
      const result = await h.defaultRun(command, secret, signal);
      if (command.command === "offer") {
        if (kind === "identity") {
          await rename(h.path, join(h.directory, "old-copy.bin"));
          await writeFile(h.path, h.encrypted);
        } else await writeFile(h.path, encrypt("replacement"));
      }
      return result;
    });
    const expected = kind === "identity" ? h.encrypted : encrypt("replacement");
    expect(await h.service.migrateLegacy(target)).toMatchObject({ kind: "committed", cleanupPending: true });
    expect(await readFile(h.path)).toEqual(expected);
    expect(await h.files.loadJournal("linear")).not.toBeNull();
    expect(h.commands.filter(command => command.command === "cleanup")).toHaveLength(0);
  });

  it("rejects profile mismatch without asking native code or touching legacy files", async () => {
    const h = await fixture();
    h.handle(async command => ({ kind: "unknown", operationId: "operationId" in command ? command.operationId : "unused" }));
    await h.service.migrateLegacy(target);
    h.commands.length = 0;
    expect(await h.reopen().migrateLegacy({ ...target, profileId: SECOND_PROFILE })).toEqual({ kind: "held", reason: "unauthorized" });
    expect(h.commands).toEqual([]);
    expect(await readFile(h.path)).toEqual(h.encrypted);
    expect(h.secrets.decryptString).not.toHaveBeenCalled();
  });

  it("never imports through a durable revision tombstone or a renderer-supplied token", async () => {
    const h = await fixture();
    expect(await h.service.migrateLegacy({ ...target, expectedRevision: 2 })).toEqual({ kind: "held", reason: "conflict" });
    expect(await h.service.migrateLegacy({ ...target, token: SECRET_CANARY })).toEqual({ kind: "held", reason: "invalidRequest" });
    expect(h.commands).toEqual([]);
    expect(await h.files.loadJournal("linear")).toBeNull();
    expect(h.secrets.decryptString).not.toHaveBeenCalled();
  });

  it("releases a preparation-only orphan only after authenticated NotFound and waits for another explicit import", async () => {
    const h = await fixture();
    h.handle(async command => command.command === "operation" ? { kind: "held", reason: "notFound" } : { kind: "held", reason: "unavailable" });
    expect(await h.service.migrateLegacy(target)).toEqual({ kind: "held", reason: "unavailable" });
    expect(await h.files.loadJournal("linear")).not.toBeNull();
    expect(await h.reopen().migrateLegacy(target)).toEqual({ kind: "held", reason: "notFound" });
    expect(await h.files.loadJournal("linear")).toBeNull();
    expect(h.commands.map(command => command.command)).toEqual(["offer", "operation"]);
    expect(h.secrets.decryptString).not.toHaveBeenCalled();
    expect(await readFile(h.path)).toEqual(h.encrypted);
    h.handle(h.defaultRun);
    expect(await h.service.migrateLegacy(target)).toMatchObject({ kind: "committed", cleanupPending: false });
  });

  it("retains ciphertext after a superseded import and deletes it only with a committed disconnect proof", async () => {
    const h = await fixture(); const barrier = deferred<NativeMigrationResult>();
    h.handle(async () => barrier.promise);
    const importing = h.service.migrateLegacy(target);
    await vi.waitFor(() => expect(h.commands).toHaveLength(1));
    expect(await h.service.cleanupLegacy({ profileId: PROFILE, provider: "linear", operationId: "disconnect-1" })).toEqual({ kind: "held", reason: "busy" });
    const offer = h.commands[0]!;
    if (offer.command !== "offer") throw new Error("expected offer");
    barrier.resolve({ kind: "operation", receipt: receipt({ operationId: offer.operationId, kind: "legacyImport", status: "superseded",
      legacySnapshotId: offer.snapshotId, connection: null, error: "conflict" }) });
    expect(await importing).toEqual({ kind: "held", reason: "conflict" });
    expect(await readFile(h.path)).toEqual(h.encrypted);
    expect(await h.service.prepareLegacyDisconnect({ profileId: PROFILE, provider: "linear", operationId: "disconnect-1", expectedRevision: 0 })).toEqual({ kind: "prepared" });
    h.receipts.set("disconnect-1", receipt({ operationId: "disconnect-1", kind: "disconnect", expectedRevision: 0, connection: null }));
    h.handle(h.defaultRun);
    expect(await h.service.cleanupLegacy({ profileId: PROFILE, provider: "linear", operationId: "disconnect-1" })).toMatchObject({ kind: "committed", cleanupPending: false });
    await expect(readFile(h.path)).rejects.toMatchObject({ code: "ENOENT" });
    const manifest = (await h.files.disconnectSnapshot({ profileId: PROFILE, provider: "linear", operationId: "disconnect-1", expectedRevision: 0 }))!;
    expect(await h.files.disconnectSnapshotCompleted(manifest)).toBe(true);
    expect(h.secrets.decryptString).not.toHaveBeenCalled();
  });

  it("cleans a partial corrupt Jira legacy pair on confirmed disconnect without parsing or decrypting", async () => {
    const h = await fixture("jira");
    await writeFile(join(h.directory, "jira-connection.json"), "corrupt metadata");
    expect(await h.service.prepareLegacyDisconnect({ profileId: PROFILE, provider: "jira", operationId: "disconnect-1", expectedRevision: 0 })).toEqual({ kind: "prepared" });
    const operation = receipt({ provider: "jira", operationId: "disconnect-1", kind: "disconnect", connection: null, cleanupPending: true });
    h.receipts.set(operation.operationId, operation);
    expect(await h.service.cleanupLegacy({ profileId: PROFILE, provider: "jira", operationId: operation.operationId })).toMatchObject({ kind: "committed", cleanupPending: true });
    await expect(readFile(h.path)).rejects.toMatchObject({ code: "ENOENT" });
    await expect(readFile(join(h.directory, "jira-connection.json"))).rejects.toMatchObject({ code: "ENOENT" });
    expect(h.secrets.decryptString).not.toHaveBeenCalled();
  });

  it("keeps files when a public caller claims a disconnect without private proof", async () => {
    const h = await fixture();
    expect(await h.service.cleanupLegacy({ profileId: PROFILE, provider: "linear", operationId: "forged", committed: true })).toEqual({ kind: "held", reason: "invalidRequest" });
    expect(await h.service.cleanupLegacy({ profileId: PROFILE, provider: "linear", operationId: "unknown" })).toEqual({ kind: "held", reason: "notFound" });
    expect(await readFile(h.path)).toEqual(h.encrypted);
    expect(h.secrets.decryptString).not.toHaveBeenCalled();
  });

  it("never snapshots a newer legacy file from an old committed disconnect without a manifest", async () => {
    const h = await fixture();
    h.receipts.set("past", receipt({ operationId: "past", kind: "disconnect", connection: null }));
    expect(await h.service.cleanupLegacy({ profileId: PROFILE, provider: "linear", operationId: "past" })).toEqual({ kind: "held", reason: "notFound" });
    expect(await readFile(h.path)).toEqual(h.encrypted);
    expect(await h.files.disconnectSnapshot({ profileId: PROFILE, provider: "linear", operationId: "past", expectedRevision: 0 })).toBeNull();
    expect(h.secrets.decryptString).not.toHaveBeenCalled();
  });

  it("persists an absent snapshot and consumes its authority without capturing later files", async () => {
    const h = await fixture();
    await rm(h.path);
    const request = { profileId: PROFILE, provider: "linear" as const, operationId: "absent", expectedRevision: 0 };
    expect(await h.service.prepareLegacyDisconnect(request)).toEqual({ kind: "prepared" });
    const absent = (await h.files.disconnectSnapshot(request))!;
    expect(absent.files).toEqual([]);
    await writeFile(h.path, h.encrypted);
    h.receipts.set(request.operationId, receipt({ operationId: request.operationId, kind: "disconnect", connection: null }));
    const proof = { profileId: PROFILE, provider: "linear" as const, operationId: request.operationId };
    expect(await h.reopen().cleanupLegacy(proof)).toMatchObject({ kind: "committed", cleanupPending: false });
    expect(await readFile(h.path)).toEqual(h.encrypted);
    expect(await h.files.disconnectSnapshotCompleted(absent)).toBe(true);
    expect(await h.reopen().prepareLegacyDisconnect(request)).toEqual({ kind: "prepared" });
    expect((await h.files.disconnectSnapshot(request))?.files).toEqual([]);
    expect(await h.reopen().cleanupLegacy(proof)).toMatchObject({ kind: "committed", cleanupPending: false });
    expect(await readFile(h.path)).toEqual(h.encrypted);
  });

  it("keeps replacement bytes after prepare and allows only a fresh explicit operation to snapshot them", async () => {
    const h = await fixture();
    const request = { profileId: PROFILE, provider: "linear" as const, operationId: "first", expectedRevision: 0 };
    expect(await h.service.prepareLegacyDisconnect(request)).toEqual({ kind: "prepared" });
    const replacement = encrypt("newer-synthetic-token");
    await writeFile(h.path, replacement);
    h.receipts.set("first", receipt({ operationId: "first", kind: "disconnect", connection: null }));
    expect(await h.service.cleanupLegacy({ profileId: PROFILE, provider: "linear", operationId: "first" })).toMatchObject({ kind: "committed", cleanupPending: true });
    expect(await readFile(h.path)).toEqual(replacement);
    expect(await h.service.prepareLegacyDisconnect(request)).toEqual({ kind: "prepared" });
    expect(await h.service.cleanupLegacy({ profileId: PROFILE, provider: "linear", operationId: "first" })).toMatchObject({ kind: "committed", cleanupPending: true });
    expect(await readFile(h.path)).toEqual(replacement);

    const next = { ...request, operationId: "second", expectedRevision: 1 };
    expect(await h.service.prepareLegacyDisconnect(next)).toEqual({ kind: "prepared" });
    h.receipts.set("second", receipt({ operationId: "second", expectedRevision: 1, kind: "disconnect", connection: null }));
    expect(await h.service.cleanupLegacy({ profileId: PROFILE, provider: "linear", operationId: "second" })).toMatchObject({ kind: "committed", cleanupPending: false });
    await expect(readFile(h.path)).rejects.toMatchObject({ code: "ENOENT" });
    await writeFile(h.path, h.encrypted);
    expect(await h.reopen().prepareLegacyDisconnect(next)).toEqual({ kind: "prepared" });
    expect(await h.reopen().cleanupLegacy({ profileId: PROFILE, provider: "linear", operationId: "second" })).toMatchObject({ kind: "committed", cleanupPending: false });
    expect(await readFile(h.path)).toEqual(h.encrypted); // Completion evidence survives process restart.
  });

  it("preserves ciphertext when preparing its manifest fails and never invents cleanup authority", async () => {
    const h = await fixture();
    await writeFile(h.path, Buffer.alloc(256 * 1024 + 1, 97));
    expect(await h.service.prepareLegacyDisconnect({ profileId: PROFILE, provider: "linear", operationId: "failed", expectedRevision: 0 })).toEqual({ kind: "held", reason: "unavailable" });
    h.receipts.set("failed", receipt({ operationId: "failed", kind: "disconnect", connection: null }));
    expect(await h.service.cleanupLegacy({ profileId: PROFILE, provider: "linear", operationId: "failed" })).toEqual({ kind: "held", reason: "notFound" });
    expect((await readFile(h.path)).length).toBe(256 * 1024 + 1);
    expect(h.secrets.decryptString).not.toHaveBeenCalled();
  });

  it("validates links with the current Gateway connection after old Jira files are gone", async () => {
    const h = await fixture("jira");
    await h.service.migrateLegacy({ ...target, provider: "jira" });
    const url = "https://jira.example/jira/browse/APP-1";
    await h.service.openExternal({ connection: reference("jira"), url });
    expect(h.openExternal).toHaveBeenCalledWith(url);
    h.handle(async () => ({ kind: "held", reason: "conflict" }));
    await expect(h.service.openExternal({ connection: reference("jira"), url })).rejects.toThrow("현재 연결");
    expect(h.openExternal).toHaveBeenCalledTimes(1);
  });

  it("rejects oversized and multiply linked legacy files before decryption", async () => {
    const h = await fixture();
    await writeFile(h.path, Buffer.alloc(256 * 1024 + 1));
    expect(await h.service.legacyStatus("linear")).toEqual({ kind: "unavailable" });
    await writeFile(h.path, h.encrypted);
    await link(h.path, join(h.directory, "alias.bin"));
    expect(await h.service.legacyStatus("linear")).toEqual({ kind: "unavailable" });
    expect(h.secrets.decryptString).not.toHaveBeenCalled();
  });

  it.each(["import", "disconnect"] as const)("recovers staged ciphertext after %s loses its original name", async kind => {
    const h = await fixture();
    let journal;
    if (kind === "disconnect") {
      journal = await h.files.prepareDisconnectSnapshot({ profileId: PROFILE, provider: "linear", operationId: "disconnect-1", expectedRevision: 0 });
    } else {
      const snapshot = (await h.files.snapshot("linear"))!;
      journal = snapshot.journal(target); snapshot.release();
      await h.files.createJournal(journal);
    }
    const moved = deferred<void>(); const release = deferred<void>();
    renameBarrier.after = async () => { moved.resolve(); await release.promise; };
    const abandoned = h.files.cleanup(journal);
    await moved.promise;
    await expect(readFile(h.path)).rejects.toMatchObject({ code: "ENOENT" });
    // A fresh process-equivalent reader sees the persisted expectation and the completed rename.
    renameBarrier.after = undefined;
    const fresh = new LegacyCredentialFiles(h.directory);
    const recovered = kind === "disconnect"
      ? (await fresh.disconnectSnapshot({ profileId: PROFILE, provider: "linear", operationId: "disconnect-1", expectedRevision: 0 }))!
      : (await fresh.loadJournal("linear"))!;
    expect(recovered.files).toEqual(journal.files);
    expect(await fresh.cleanup(recovered)).toBe(true);
    expect((await readdir(h.directory)).some(name => name.startsWith(".akra-provider-cleanup-"))).toBe(false);
    release.resolve();
    expect(await abandoned).toBe(false);
    if (kind === "disconnect") {
      await fresh.finishDisconnectSnapshot(recovered);
      expect(await fresh.disconnectSnapshotCompleted(recovered)).toBe(true);
      expect(await fresh.disconnectSnapshot({ profileId: PROFILE, provider: "linear", operationId: "disconnect-1", expectedRevision: 0 })).toEqual(recovered);
    } else {
      await fresh.removeJournal(recovered);
      expect(await readdir(h.directory)).toEqual([]);
    }
  });

  it("publishes exactly one complete journal during two-Desktop contention", async () => {
    const h = await fixture();
    const snapshot = (await h.files.snapshot("linear"))!;
    const one = snapshot.journal(target); const two = snapshot.journal(target); snapshot.release();
    const results = await Promise.allSettled([h.files.createJournal(one), h.files.createJournal(two)]);
    expect(results.filter(result => result.status === "fulfilled")).toHaveLength(1);
    const saved = (await h.files.loadJournal("linear"))!;
    expect([one.operationId, two.operationId]).toContain(saved.operationId);
    await expect(h.files.removeJournal(saved.operationId === one.operationId ? two : one)).rejects.toThrow();
    expect((await h.files.loadJournal("linear"))?.operationId).toBe(saved.operationId);
    expect((await readdir(h.directory)).some(name => name.startsWith(".akra-provider-journal"))).toBe(false);
  });
});
