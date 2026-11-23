// Electron main only: bounded legacy ciphertext snapshots. No plaintext is written to disk.
import { constants, type BigIntStats } from "node:fs";
import { open, lstat, mkdir, rename, link, rm, rmdir, readdir } from "node:fs/promises";
import { createHash, randomUUID } from "node:crypto";
import { join } from "node:path";
import {
  type ConnectionConfiguration, type DisconnectRequest, type LegacyMigrationTarget, type Provider,
  isOperationId, isProfileId, isProvider, isRevision, isSnapshotId, publicRecord,
} from "../shared/providerCredentials";
import { normalizeJiraSite } from "../shared/providerLinks";

const MAX_CIPHERTEXT_BYTES = 256 * 1024;
const MAX_METADATA_BYTES = 16 * 1024;
const MAX_SECRET_BYTES = 64 * 1024;
const MAX_JOURNAL_BYTES = 16 * 1024;
const FILES = { jira: ["jira-token.bin", "jira-connection.json"], linear: ["linear-token.bin"] } as const;
export interface LegacySecretBox {
  isEncryptionAvailable(): boolean;
  decryptString(data: Buffer): string;
}
interface Identity { readonly dev: string; readonly ino: string; }
export interface LegacyFileExpectation {
  readonly name: string;
  readonly size: number;
  readonly digest: string;
  readonly identity: Identity;
}
interface CapturedFile {
  readonly expectation: LegacyFileExpectation;
  readonly bytes: Buffer;
}
export interface LegacyMigrationJournal extends LegacyMigrationTarget {
  readonly version: 1;
  readonly operationId: string;
  readonly snapshotId: string;
  readonly files: readonly LegacyFileExpectation[];
}
function unavailable(): never { throw new Error("기존 연결을 안전하게 읽거나 정리할 수 없습니다."); }
function hash(bytes: Uint8Array): string { return createHash("sha256").update(bytes).digest("hex"); }
function identity(stat: BigIntStats): Identity { return { dev: String(stat.dev), ino: String(stat.ino) }; }
function sameIdentity(left: Identity, right: Identity): boolean { return left.dev === right.dev && left.ino === right.ino; }
function sameStat(left: BigIntStats, right: BigIntStats): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.size === right.size &&
    left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}
function absent(error: unknown): boolean { return (error as NodeJS.ErrnoException)?.code === "ENOENT"; }

async function capture(path: string, name: string, maximum: number): Promise<CapturedFile | null> {
  let before: BigIntStats;
  try { before = await lstat(path, { bigint: true }); } catch (error) { if (absent(error)) return null; throw error; }
  if (!before.isFile() || before.isSymbolicLink() || before.nlink !== 1n || before.size < 1n || before.size > BigInt(maximum)) unavailable();
  const file = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const opened = await file.stat({ bigint: true });
    if (!sameStat(before, opened)) unavailable();
    // Read at most the prevalidated size; an extending file cannot allocate an unbounded buffer.
    const bytes = Buffer.alloc(Number(opened.size));
    let offset = 0;
    while (offset < bytes.length) {
      const result = await file.read(bytes, offset, bytes.length - offset, offset);
      if (!result.bytesRead) { bytes.fill(0); unavailable(); }
      offset += result.bytesRead;
    }
    const after = await file.stat({ bigint: true });
    const named = await lstat(path, { bigint: true });
    if (!sameStat(opened, after) || !sameStat(after, named) || !named.isFile() || named.nlink !== 1n) {
      bytes.fill(0); unavailable();
    }
    return { bytes, expectation: { name, size: bytes.length, digest: hash(bytes), identity: identity(opened) } };
  } finally { await file.close(); }
}

function matches(file: CapturedFile, expected: LegacyFileExpectation): boolean {
  return file.expectation.size === expected.size && file.expectation.digest === expected.digest &&
    sameIdentity(file.expectation.identity, expected.identity);
}
function jiraConfiguration(bytes: Buffer): ConnectionConfiguration {
  let raw: Record<string, unknown>;
  try { raw = publicRecord(JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes)), ["siteUrl", "deployment", "email"]); }
  catch { return unavailable(); }
  const siteUrl = normalizeJiraSite(raw.siteUrl);
  if (!siteUrl || (raw.deployment !== "cloud" && raw.deployment !== "server")) unavailable();
  const email = typeof raw.email === "string" ? raw.email.trim() : "";
  if (raw.deployment === "cloud" && (!email || new TextEncoder().encode(email).length > 320 || /[\u0000-\u001f\u007f]/.test(email))) unavailable();
  return { siteUrl, deployment: raw.deployment, ...(raw.deployment === "cloud" ? { email } : {}) };
}

export class LegacySnapshot {
  readonly snapshotId: string;
  readonly configuration: ConnectionConfiguration;
  constructor(readonly provider: Provider, private readonly captured: readonly CapturedFile[]) {
    const digest = createHash("sha256").update("akra-provider-legacy-v1\0").update(provider);
    for (const file of captured) {
      digest.update("\0" + file.expectation.name + "\0" + file.expectation.size + "\0");
      digest.update(file.bytes);
    }
    this.snapshotId = digest.digest("hex");
    this.configuration = provider === "jira" ? jiraConfiguration(captured[1]!.bytes) : {};
  }
  journal(target: LegacyMigrationTarget): LegacyMigrationJournal {
    return {
      version: 1, profileId: target.profileId, provider: this.provider, expectedRevision: target.expectedRevision,
      operationId: randomUUID(), snapshotId: this.snapshotId,
      files: this.captured.map(file => ({ ...file.expectation, identity: { ...file.expectation.identity } })),
    };
  }
  /** Called only after the private helper has verified owner/profile and native policy confirmation. */
  secret(secrets: LegacySecretBox): Buffer {
    if (!secrets.isEncryptionAvailable()) unavailable();
    let text = secrets.decryptString(this.captured[0]!.bytes);
    if (this.provider === "linear") {
      let raw: Record<string, unknown>;
      try { raw = publicRecord(JSON.parse(text), ["token", "viewer"]); } catch { return unavailable(); }
      if (typeof raw.token !== "string") unavailable();
      text = raw.token;
      if (text.length > 4_096) unavailable();
    }
    const normalized = text.trim();
    if (!normalized || /[\u0000-\u0020\u007f]/.test(normalized)) unavailable();
    const bytes = Buffer.from(normalized, "utf8");
    if (bytes.length > MAX_SECRET_BYTES || (this.provider === "linear" && bytes.length > 12_288)) {
      bytes.fill(0); unavailable();
    }
    return bytes;
  }
  release(): void { for (const file of this.captured) file.bytes.fill(0); }
}

/** The directory is Electron's own userData; callers cannot supply paths or filenames. */
export class LegacyCredentialFiles {
  constructor(private readonly directory: string) {}
  private journalPath(provider: Provider): string { return join(this.directory, provider + "-credential-migration.json"); }
  private disconnectPath(request: Pick<DisconnectRequest, "provider" | "operationId">, completed = false): string {
    return join(this.directory, request.provider + "-credential-disconnect-" + request.operationId + (completed ? ".completed" : "") + ".json");
  }

  async snapshot(provider: Provider): Promise<LegacySnapshot | null> {
    const files: CapturedFile[] = [];
    try {
      for (const name of FILES[provider]) {
        const captured = await capture(join(this.directory, name), name, name.endsWith(".json") ? MAX_METADATA_BYTES : MAX_CIPHERTEXT_BYTES);
        if (!captured) {
          if (files.length === 0) return null;
          unavailable();
        }
        files.push(captured);
      }
      return new LegacySnapshot(provider, files);
    } catch {
      for (const file of files) file.bytes.fill(0);
      return unavailable();
    }
  }

  async loadJournal(provider: Provider): Promise<LegacyMigrationJournal | null> {
    return this.loadRecord(provider, this.journalPath(provider), false);
  }

  private async loadRecord(provider: Provider, path: string, partial: boolean): Promise<LegacyMigrationJournal | null> {
    const file = await capture(path, provider + "-credential-record.json", MAX_JOURNAL_BYTES);
    if (!file) return null;
    try {
      const raw = publicRecord(JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(file.bytes)),
        ["version", "profileId", "provider", "expectedRevision", "operationId", "snapshotId", "files"]);
      if (raw.version !== 1 || raw.provider !== provider || !isProvider(raw.provider) || !isProfileId(raw.profileId) ||
        !isRevision(raw.expectedRevision) || !isOperationId(raw.operationId) || !isSnapshotId(raw.snapshotId) ||
        !Array.isArray(raw.files) || (!partial && raw.files.length < 1) || raw.files.length > FILES[provider].length || (!partial && raw.files.length !== FILES[provider].length)) unavailable();
      const files = raw.files.map((item: unknown, index: number): LegacyFileExpectation => {
        const entry = publicRecord(item, ["name", "size", "digest", "identity"]);
        const id = publicRecord(entry.identity, ["dev", "ino"]);
        if (typeof entry.name !== "string" || !(FILES[provider] as readonly string[]).includes(entry.name) || (!partial && entry.name !== FILES[provider][index]) || !isRevision(entry.size) || entry.size < 1 || entry.size > MAX_CIPHERTEXT_BYTES ||
          !isSnapshotId(entry.digest) || typeof id.dev !== "string" || typeof id.ino !== "string" ||
          !/^\d{1,24}$/.test(id.dev) || !/^\d{1,24}$/.test(id.ino)) unavailable();
        return { name: entry.name, size: entry.size, digest: entry.digest, identity: { dev: id.dev, ino: id.ino } };
      });
      if (new Set(files.map(file => file.name)).size !== files.length) unavailable();
      return { version: 1, profileId: raw.profileId, provider, expectedRevision: raw.expectedRevision,
        operationId: raw.operationId, snapshotId: raw.snapshotId, files };
    } catch { return unavailable(); } finally { file.bytes.fill(0); }
  }

  async createJournal(journal: LegacyMigrationJournal): Promise<void> {
    return this.createRecord(journal, this.journalPath(journal.provider));
  }

  private async createRecord(journal: LegacyMigrationJournal, path: string): Promise<void> {
    const bytes = Buffer.from(JSON.stringify(journal) + "\n", "utf8");
    if (bytes.length > MAX_JOURNAL_BYTES) unavailable();
    // Publish the fully flushed nonsecret journal without replacing another Desktop's operation.
    const temporary = join(this.directory, ".akra-provider-journal-" + randomUUID());
    const file = await open(temporary, "wx", 0o600);
    try { await file.writeFile(bytes); await file.sync(); } finally { bytes.fill(0); await file.close(); }
    try { await link(temporary, path); }
    finally { await rm(temporary); }
  }

  async removeJournal(expected: LegacyMigrationJournal): Promise<void> {
    return this.removeRecord(expected, this.journalPath(expected.provider), false);
  }

  private async removeRecord(expected: LegacyMigrationJournal, path: string, partial: boolean): Promise<void> {
    const current = await this.loadRecord(expected.provider, path, partial);
    if (!current) return;
    if (current.profileId !== expected.profileId || current.operationId !== expected.operationId ||
      current.snapshotId !== expected.snapshotId) unavailable();
    const file = await capture(path, path.slice(Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")) + 1), MAX_JOURNAL_BYTES);
    if (!file) return;
    try {
      // Conditional quarantine verifies the exact inode and bytes again after the move.
      const raw = JSON.parse(file.bytes.toString("utf8")) as LegacyMigrationJournal;
      if (raw.profileId !== expected.profileId || raw.operationId !== expected.operationId ||
        raw.snapshotId !== expected.snapshotId || !await this.cleanup({ ...expected, files: [file.expectation] })) unavailable();
    } finally { file.bytes.fill(0); }
  }

  async cleanup(journal: LegacyMigrationJournal): Promise<boolean> {
    const moved: Array<{ original: string; staged: string }> = [];
    // A durable operation/snapshot determines the staging root. Each attempt reserves a new,
    // exclusive subdirectory, so rename never replaces another attempt's ciphertext.
    const scope = hash(Buffer.from(JSON.stringify([journal.provider, journal.profileId, journal.operationId, journal.snapshotId, journal.files])));
    const quarantine = join(this.directory, ".akra-provider-cleanup-" + scope);
    let attempt: string | undefined;
    try {
      try { await mkdir(quarantine, { mode: 0o700 }); }
      catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
        const stat = await lstat(quarantine);
        if (!stat.isDirectory() || stat.isSymbolicLink()) return false;
      }
      // Recover completed renames left by a crashed process before interpreting missing originals.
      const attempts = await readdir(quarantine, { withFileTypes: true });
      if (attempts.length > 32) return false;
      for (const entry of attempts) {
        if (!/^attempt-[a-f0-9-]{36}$/.test(entry.name) || !entry.isDirectory() || entry.isSymbolicLink()) return false;
        const directory = join(quarantine, entry.name);
        const names = await readdir(directory);
        if (names.some(name => !journal.files.some(file => file.name === name))) return false;
        for (const name of names) {
          const expected = journal.files.find(file => file.name === name)!;
          const current = await capture(join(directory, name), name, MAX_CIPHERTEXT_BYTES);
          const valid = current !== null && matches(current, expected);
          current?.bytes.fill(0);
          if (!valid) return false;
        }
        for (const name of names) await rm(join(directory, name));
        await rmdir(directory);
      }
      for (const expected of journal.files) {
        const current = await capture(join(this.directory, expected.name), expected.name, MAX_CIPHERTEXT_BYTES);
        if (!current) continue;
        const valid = matches(current, expected);
        current.bytes.fill(0);
        if (!valid) return false;
      }
      attempt = join(quarantine, "attempt-" + randomUUID());
      await mkdir(attempt, { mode: 0o700 });
      for (const expected of journal.files) {
        const original = join(this.directory, expected.name);
        const staged = join(attempt, expected.name);
        try { await rename(original, staged); } catch (error) { if (absent(error)) continue; throw error; }
        moved.push({ original, staged });
        const current = await capture(staged, expected.name, MAX_CIPHERTEXT_BYTES);
        const valid = current !== null && matches(current, expected);
        current?.bytes.fill(0);
        if (!valid) throw new Error("changed");
      }
      for (const entry of moved) await rm(entry.staged);
      await rmdir(attempt);
      await rmdir(quarantine); // ACK requires all known staging to be gone, not just original ENOENT.
      return true;
    } catch {
      for (const entry of moved.reverse()) {
        try { await link(entry.staged, entry.original); await rm(entry.staged); } catch { /* preserve on collision */ }
      }
      if (attempt) try { await rmdir(attempt); } catch { /* staged ciphertext is recoverable by this journal */ }
      try { await rmdir(quarantine); } catch { /* preserve the operation's staging */ }
      return false;
    }
  }

  private async disconnectRecord(request: DisconnectRequest, completed = false): Promise<LegacyMigrationJournal | null> {
    const record = await this.loadRecord(request.provider, this.disconnectPath(request, completed), true);
    if (record && (record.profileId !== request.profileId || record.operationId !== request.operationId ||
      record.expectedRevision !== request.expectedRevision)) unavailable();
    return record;
  }

  async prepareDisconnectSnapshot(request: DisconnectRequest): Promise<LegacyMigrationJournal> {
    const completed = await this.disconnectRecord(request, true);
    const existing = await this.disconnectRecord(request);
    if (existing) {
      if (completed && completed.snapshotId !== existing.snapshotId) unavailable();
      return existing; // Includes already consumed operations. Never capture a new file for an old ID.
    }
    if (completed) unavailable(); // A missing original manifest cannot reactivate consumed authority.
    const prefix = request.provider + "-credential-disconnect-";
    const records = (await readdir(this.directory)).filter(name => name.startsWith(prefix) &&
      name.endsWith(".json") && !name.endsWith(".completed.json"));
    if (records.length >= 256) unavailable();
    const files: LegacyFileExpectation[] = [];
    for (const name of FILES[request.provider]) {
      const captured = await capture(join(this.directory, name), name, MAX_CIPHERTEXT_BYTES);
      if (!captured) continue;
      files.push(captured.expectation);
      captured.bytes.fill(0);
    }
    const journal: LegacyMigrationJournal = {
      version: 1, ...request, snapshotId: hash(Buffer.from(JSON.stringify(files))), files,
    };
    // Even an absent source has durable meaning. This manifest predates the public disconnect.
    await this.createRecord(journal, this.disconnectPath(request));
    return journal;
  }

  async disconnectSnapshot(request: DisconnectRequest): Promise<LegacyMigrationJournal | null> {
    return this.disconnectRecord(request); // Cleanup/replay must never capture a new source.
  }

  async disconnectSnapshotCompleted(journal: LegacyMigrationJournal): Promise<boolean> {
    const completed = await this.disconnectRecord(journal, true);
    if (!completed) return false;
    if (completed.snapshotId !== journal.snapshotId || JSON.stringify(completed.files) !== JSON.stringify(journal.files)) unavailable();
    return true;
  }

  async finishDisconnectSnapshot(journal: LegacyMigrationJournal): Promise<void> {
    if (await this.disconnectSnapshotCompleted(journal)) return;
    try { await this.createRecord(journal, this.disconnectPath(journal, true)); }
    catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST" || !await this.disconnectSnapshotCompleted(journal)) throw error;
    }
    // Retain immutable manifest + completion evidence so retries cannot bind newer ciphertext.
  }
}
