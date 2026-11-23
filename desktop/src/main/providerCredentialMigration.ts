// Legacy migration is the sole remaining provider credential responsibility in Electron main.
import {
  isProvider, parseConnectionRef, parseDisconnectRequest, parseLegacyTarget, parseOperationRequest, publicRecord,
  type CredentialOperationRequest, type DisconnectRequest, type LegacyDisconnectPreparation, type LegacyAvailability, type LegacyMigrationResult,
  type LegacyMigrationTarget, type Provider,
} from "../shared/providerCredentials";
import { LegacyCredentialFiles, type LegacyMigrationJournal, type LegacySecretBox } from "./providerLegacy";
import type { NativeMigrationResult, ProviderMigrationSource } from "./providerMigrationSource";

export interface ProviderCredentialMigrationDependencies {
  files: LegacyCredentialFiles;
  secrets: LegacySecretBox;
  source: ProviderMigrationSource;
  openExternal(url: string): Promise<void>;
}
const unavailable = (): LegacyMigrationResult => ({ kind: "held", reason: "unavailable" });
export class ProviderCredentialMigration {
  private readonly busy = new Set<Provider>();
  constructor(private readonly dependencies: ProviderCredentialMigrationDependencies) {}

  async legacyStatus(input: unknown): Promise<LegacyAvailability> {
    if (!isProvider(input)) return { kind: "unavailable" };
    try {
      if (await this.dependencies.files.loadJournal(input)) return { kind: "pending" };
      const snapshot = await this.dependencies.files.snapshot(input);
      if (!snapshot) return { kind: "absent" };
      snapshot.release();
      return { kind: "available" };
    } catch { return { kind: "unavailable" }; }
  }

  async migrateLegacy(input: unknown, signal?: AbortSignal): Promise<LegacyMigrationResult> {
    let target: LegacyMigrationTarget;
    try { target = parseLegacyTarget(input); } catch { return { kind: "held", reason: "invalidRequest" }; }
    if (this.busy.has(target.provider)) return { kind: "held", reason: "busy" };
    this.busy.add(target.provider);
    try {
      const existing = await this.dependencies.files.loadJournal(target.provider);
      if (existing) {
        if (existing.profileId !== target.profileId) return { kind: "held", reason: "unauthorized" };
        const result = await this.dependencies.source.run({
          command: "operation", profileId: existing.profileId, provider: existing.provider, operationId: existing.operationId,
        }, undefined, signal);
        if (result.kind === "held" && result.reason === "notFound") {
          // An authenticated, normally exited helper confirmed no durable operation.
          // Remove only this journal; do not resend a secret or create a new operation here.
          await this.dependencies.files.removeJournal(existing);
          return result;
        }
        return await this.complete(existing, result, signal);
      }
      // A nonzero revision may be a durable disconnect tombstone. Only a new native connection
      // can replace it; legacy import must never silently resurrect an old token.
      if (target.expectedRevision !== 0) return { kind: "held", reason: "conflict" };
      const snapshot = await this.dependencies.files.snapshot(target.provider);
      if (!snapshot) return { kind: "held", reason: "notFound" };
      try {
        const journal = snapshot.journal(target);
        await this.dependencies.files.createJournal(journal);
        const result = await this.dependencies.source.run({
          command: "offer", profileId: target.profileId, provider: target.provider,
          operationId: journal.operationId, expectedRevision: target.expectedRevision,
          snapshotId: journal.snapshotId, configuration: snapshot.configuration,
        }, () => snapshot.secret(this.dependencies.secrets), signal);
        return await this.complete(journal, result, signal);
      } finally { snapshot.release(); }
    } catch { return unavailable(); }
    finally { this.busy.delete(target.provider); }
  }

  private async complete(journal: LegacyMigrationJournal, result: NativeMigrationResult, signal?: AbortSignal): Promise<LegacyMigrationResult> {
    if (result.kind === "held" || result.kind === "unknown") return result;
    if (result.kind === "validatedLink") return unavailable();
    const receipt = result.receipt;
    if (receipt.operationId !== journal.operationId || receipt.profileId !== journal.profileId ||
      receipt.provider !== journal.provider || receipt.expectedRevision !== journal.expectedRevision ||
      receipt.kind !== "legacyImport" || receipt.legacySnapshotId !== journal.snapshotId) return unavailable();
    if (["failed", "cancelled", "superseded"].includes(receipt.status)) {
      await this.dependencies.files.removeJournal(journal);
      return { kind: "held", reason: receipt.error ?? (receipt.status === "cancelled" ? "cancelled" : "conflict") };
    }
    if (receipt.status !== "committed") return { kind: "unknown", operationId: journal.operationId };
    if (!receipt.legacyCleanupAllowed) {
      // Acknowledgement from a previous cleanup is authoritative, but never delete newly
      // appeared files merely because an old operation was once committed.
      await this.dependencies.files.removeJournal(journal);
      return { kind: "committed", operationId: journal.operationId, cleanupPending: receipt.cleanupPending };
    }
    if (signal?.aborted || !await this.dependencies.files.cleanup(journal)) {
      return { kind: "committed", operationId: journal.operationId, cleanupPending: true };
    }
    const acknowledged = await this.dependencies.source.run({
      command: "cleanup", profileId: journal.profileId, provider: journal.provider,
      operationId: journal.operationId, snapshotId: journal.snapshotId,
    }, undefined, signal);
    if ((acknowledged.kind === "committed" || acknowledged.kind === "operation") &&
      acknowledged.receipt.operationId === journal.operationId &&
      acknowledged.receipt.profileId === journal.profileId &&
      acknowledged.receipt.provider === journal.provider &&
      acknowledged.receipt.kind === "legacyImport" &&
      acknowledged.receipt.status === "committed" &&
      acknowledged.receipt.legacySnapshotId === journal.snapshotId &&
      !acknowledged.receipt.legacyCleanupAllowed) {
      await this.dependencies.files.removeJournal(journal);
      return { kind: "committed", operationId: journal.operationId, cleanupPending: acknowledged.receipt.cleanupPending };
    }
    // The encrypted files are gone, but the journal stays until the same operation confirms ACK.
    return { kind: "committed", operationId: journal.operationId, cleanupPending: true };
  }

  async prepareLegacyDisconnect(input: unknown, signal?: AbortSignal): Promise<LegacyDisconnectPreparation> {
    let request: DisconnectRequest;
    try { request = parseDisconnectRequest(input); } catch { return { kind: "held", reason: "invalidRequest" }; }
    if (this.busy.has(request.provider)) return { kind: "held", reason: "busy" };
    this.busy.add(request.provider);
    try {
      if (signal?.aborted) return { kind: "held", reason: "unavailable" };
      const pending = await this.dependencies.files.loadJournal(request.provider);
      if (pending && pending.profileId !== request.profileId) return { kind: "held", reason: "unauthorized" };
      await this.dependencies.files.prepareDisconnectSnapshot(request);
      return { kind: "prepared" };
    } catch { return { kind: "held", reason: "unavailable" }; }
    finally { this.busy.delete(request.provider); }
  }

  async cleanupLegacy(input: unknown, signal?: AbortSignal): Promise<LegacyMigrationResult> {
    let request: CredentialOperationRequest;
    try { request = parseOperationRequest(input); } catch { return { kind: "held", reason: "invalidRequest" }; }
    if (this.busy.has(request.provider)) return { kind: "held", reason: "busy" };
    this.busy.add(request.provider);
    try {
      const journal = await this.dependencies.files.loadJournal(request.provider);
      if (journal && journal.profileId !== request.profileId) return { kind: "held", reason: "unauthorized" };
      const result = await this.dependencies.source.run({ command: "operation", ...request }, undefined, signal);
      if (result.kind === "held" || result.kind === "unknown") return result;
      if (result.kind === "validatedLink" || result.receipt.operationId !== request.operationId ||
        result.receipt.profileId !== request.profileId || result.receipt.provider !== request.provider ||
        result.receipt.kind !== "disconnect" || result.receipt.status !== "committed" || result.receipt.connection !== null) return unavailable();
      if (signal?.aborted) return unavailable();
      const snapshot = await this.dependencies.files.disconnectSnapshot({ ...request, expectedRevision: result.receipt.expectedRevision });
      if (!snapshot) return { kind: "held", reason: "notFound" };
      const consumed = await this.dependencies.files.disconnectSnapshotCompleted(snapshot);
      const cleaned = consumed || await this.dependencies.files.cleanup(snapshot);
      if (cleaned && !consumed) await this.dependencies.files.finishDisconnectSnapshot(snapshot);
      // A separate import journal retains its own staging/receipt recovery path.
      return { kind: "committed", operationId: request.operationId, cleanupPending: !cleaned || result.receipt.cleanupPending || journal !== null };
    } catch { return unavailable(); }
    finally { this.busy.delete(request.provider); }
  }

  async openExternal(input: unknown, signal?: AbortSignal): Promise<void> {
    try {
      const raw = publicRecord(input, ["connection", "url"]);
      const reference = parseConnectionRef(raw.connection);
      if (typeof raw.url !== "string" || Buffer.byteLength(raw.url, "utf8") > 4_096 ||
        /[\s\\\u0000-\u001f\u007f]/.test(raw.url)) throw new Error();
      const url = new URL(raw.url);
      if (url.protocol !== "https:" || url.username || url.password) throw new Error();
      const result = await this.dependencies.source.run({ command: "validateLink", reference, url: raw.url }, undefined, signal);
      if (signal?.aborted || result.kind !== "validatedLink" || result.url !== raw.url) throw new Error();
      await this.dependencies.openExternal(result.url);
    } catch { throw new Error("현재 연결에서 확인한 이슈 주소만 열 수 있습니다."); }
  }
}
