// Fixed native helper protocol. Secret bytes use a bounded inherited stdin pipe, never JSON/argv/logs.
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomUUID } from "node:crypto";
import {
  isCredentialError, isOperationId, parseOperationReceipt, publicRecord,
  type ConnectionConfiguration, type ConnectionRef, type CredentialError, type CredentialOperationRequest,
  type LegacyMigrationTarget, type OperationReceipt,
} from "../shared/providerCredentials";

const MAX_HEADER_BYTES = 16 * 1024;
const MAX_SECRET_BYTES = 64 * 1024;
const MAX_OUTPUT_BYTES = 16 * 1024;
const HELPER_LIFETIME_MS = 5 * 60_000;

export type MigrationCommand =
  | ({ readonly command: "offer"; readonly operationId: string; readonly snapshotId: string; readonly configuration: ConnectionConfiguration } & LegacyMigrationTarget)
  | ({ readonly command: "operation" } & CredentialOperationRequest)
  | ({ readonly command: "cleanup"; readonly snapshotId: string } & CredentialOperationRequest)
  | { readonly command: "validateLink"; readonly reference: ConnectionRef; readonly url: string };
export type NativeMigrationResult =
  | { readonly kind: "committed" | "operation"; readonly receipt: OperationReceipt }
  | { readonly kind: "held"; readonly reason: CredentialError }
  | { readonly kind: "unknown"; readonly operationId: string }
  | { readonly kind: "validatedLink"; readonly url: string };
export interface ProviderMigrationSource {
  run(command: MigrationCommand, secret?: () => Buffer | Promise<Buffer>, signal?: AbortSignal): Promise<NativeMigrationResult>;
}
export interface ProviderMigrationSourceDependencies {
  executable(): string;
  /** Test seam only; production callers cannot select an arbitrary executable or transport. */
  spawnHelper?(executable: string): ChildProcessWithoutNullStreams;
  lifetimeMs?: number;
}

function held(reason: CredentialError = "unavailable"): NativeMigrationResult { return { kind: "held", reason }; }
function spawnHelper(executable: string): ChildProcessWithoutNullStreams {
  return spawn(executable, ["provider-credential-migration"], {
    stdio: "pipe", windowsHide: true, detached: process.platform !== "win32",
  });
}
export function createProviderMigrationSource(dependencies: ProviderMigrationSourceDependencies): ProviderMigrationSource {
  return { run: (command, secret, signal) => new NativeMigrationChannel(dependencies, command, secret, signal).run() };
}

class NativeMigrationChannel {
  private readonly id = randomUUID();
  private child: ChildProcessWithoutNullStreams | undefined;
  private bytes: Buffer<ArrayBufferLike> = Buffer.alloc(0);
  private receivedBytes = 0;
  private prepared = false;
  private submitted = false;
  private closed = false;
  private processClosed = false;
  private terminal: NativeMigrationResult | undefined;
  private timer: NodeJS.Timeout | undefined;
  private resolve: ((value: NativeMigrationResult) => void) | undefined;
  private readonly abort = () => this.finish(this.failure());
  constructor(
    private readonly dependencies: ProviderMigrationSourceDependencies,
    private readonly command: MigrationCommand,
    private readonly secret: (() => Buffer | Promise<Buffer>) | undefined,
    private readonly signal: AbortSignal | undefined,
  ) {}

  run(): Promise<NativeMigrationResult> {
    if (this.signal?.aborted || (this.command.command === "offer") !== Boolean(this.secret)) return Promise.resolve(held("invalidRequest"));
    const answer = new Promise<NativeMigrationResult>(resolve => { this.resolve = resolve; });
    this.signal?.addEventListener("abort", this.abort, { once: true });
    this.timer = setTimeout(() => this.finish(this.failure()), this.dependencies.lifetimeMs ?? HELPER_LIFETIME_MS);
    try {
      const header = Buffer.from(JSON.stringify({ version: 1, id: this.id, ...this.command }) + "\n", "utf8");
      if (header.length > MAX_HEADER_BYTES) { this.finish(held("tooLarge")); return answer; }
      const child = (this.dependencies.spawnHelper ?? spawnHelper)(this.dependencies.executable());
      this.child = child;
      child.stdout.on("data", (chunk: Buffer) => this.receive(chunk));
      child.stdout.on("error", () => this.finish(this.failure()));
      // Drain raw native diagnostics without forwarding them to logs, recording, or renderer.
      child.stderr.on("error", () => this.finish(this.failure()));
      child.stderr.resume();
      child.stdin.on("error", () => this.finish(this.failure()));
      child.on("error", () => this.finish(this.failure()));
      child.once("close", code => {
        this.processClosed = true;
        this.finish(code === 0 && this.bytes.length === 0 ? this.terminal ?? this.failure() : this.failure());
      });
      if (this.command.command === "offer") child.stdin.write(header);
      else child.stdin.end(header);
    } catch { this.finish(this.failure()); }
    return answer;
  }

  private receive(chunk: Buffer): void {
    if (this.closed) return;
    this.receivedBytes += chunk.length;
    if (this.receivedBytes > MAX_OUTPUT_BYTES || this.terminal) return this.finish(this.failure());
    const previous = this.bytes;
    this.bytes = Buffer.concat([previous, chunk]);
    previous.fill(0);
    const end = this.bytes.indexOf(10);
    if (end < 0) return;
    if (end !== this.bytes.length - 1) return this.finish(this.failure());
    try {
      const value: unknown = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(this.bytes.subarray(0, end)));
      this.bytes.fill(0); this.bytes = Buffer.alloc(0);
      const base = publicRecord(value, ["version", "id", "kind", "profileId", "operationId", "expectedRevision", "receipt", "reason", "url"]);
      if (base.version !== 1 || base.id !== this.id) return this.finish(this.failure());
      if (base.kind === "prepared") {
        publicRecord(base, ["version", "id", "kind", "profileId", "operationId", "expectedRevision"]);
        if (this.command.command !== "offer" || this.prepared ||
          base.profileId !== this.command.profileId || base.operationId !== this.command.operationId ||
          base.expectedRevision !== this.command.expectedRevision) return this.finish(this.failure());
        this.prepared = true;
        void this.submitSecret();
        return;
      }
      this.terminal = this.parseTerminal(base);
      if (!this.terminal) return this.finish(this.failure());
      this.child?.stdin.end();
    } catch { this.finish(this.failure()); }
  }

  private parseTerminal(raw: Record<string, unknown>): NativeMigrationResult | undefined {
    if (raw.kind === "held") {
      publicRecord(raw, ["version", "id", "kind", "reason"]);
      return isCredentialError(raw.reason) ? held(raw.reason) : undefined;
    }
    if (raw.kind === "unknown") {
      publicRecord(raw, ["version", "id", "kind", "profileId", "operationId"]);
      if (this.command.command === "validateLink" || raw.profileId !== this.command.profileId || raw.operationId !== this.command.operationId || !isOperationId(raw.operationId)) return;
      return { kind: "unknown", operationId: raw.operationId };
    }
    if (raw.kind === "validatedLink") {
      publicRecord(raw, ["version", "id", "kind", "url"]);
      return this.command.command === "validateLink" && raw.url === this.command.url
        ? { kind: "validatedLink", url: this.command.url } : undefined;
    }
    if (raw.kind === "committed" || raw.kind === "operation") {
      publicRecord(raw, ["version", "id", "kind", "profileId", "operationId", "receipt"]);
      if (this.command.command === "validateLink") return;
      const receipt = parseOperationReceipt(raw.receipt);
      if (raw.profileId !== this.command.profileId || raw.operationId !== this.command.operationId ||
        receipt.profileId !== this.command.profileId || receipt.operationId !== this.command.operationId ||
        receipt.provider !== this.command.provider) return;
      if (this.command.command === "offer") {
        const refusedBeforeSecret = ["failed", "cancelled", "superseded"].includes(receipt.status);
        if ((!this.submitted && !refusedBeforeSecret) || receipt.expectedRevision !== this.command.expectedRevision ||
          receipt.legacySnapshotId !== this.command.snapshotId) return;
      }
      if (raw.kind === "committed" && (receipt.kind !== "legacyImport" || receipt.status !== "committed" || !receipt.legacyCleanupAllowed)) return;
      return { kind: raw.kind, receipt };
    }
    return;
  }

  private async submitSecret(): Promise<void> {
    let bytes: Buffer | undefined;
    let frame: Buffer | undefined;
    try {
      bytes = await this.secret!();
      if (this.closed || this.terminal || this.signal?.aborted) return;
      if (!Buffer.isBuffer(bytes) || bytes.length < 1 || bytes.length > MAX_SECRET_BYTES) {
        this.finish(held("tooLarge")); return;
      }
      frame = Buffer.alloc(4 + bytes.length);
      frame.writeUInt32BE(bytes.length);
      bytes.copy(frame, 4);
      bytes.fill(0);
      this.submitted = true;
      await new Promise<void>((resolve, reject) => this.child!.stdin.end(frame!, (error?: Error | null) => error ? reject(error) : resolve()));
    } catch { this.finish(this.submitted ? this.failure() : held("invalidRequest")); }
    finally { bytes?.fill(0); frame?.fill(0); }
  }

  private failure(): NativeMigrationResult {
    return this.submitted && this.command.command !== "validateLink"
      ? { kind: "unknown", operationId: this.command.operationId } : held();
  }

  private finish(value: NativeMigrationResult): void {
    if (this.closed) return;
    this.closed = true;
    clearTimeout(this.timer);
    this.signal?.removeEventListener("abort", this.abort);
    this.bytes.fill(0); this.bytes = Buffer.alloc(0);
    this.terminal = undefined;
    this.resolve?.(value); this.resolve = undefined;
    const child = this.child;
    if (!child || this.processClosed) return;
    try { child.stdin.end(); } catch { /* closed */ }
    const stop = (signal: NodeJS.Signals) => {
      if (this.processClosed) return;
      try {
        if (process.platform !== "win32" && child.pid) process.kill(-child.pid, signal);
        else child.kill(signal);
      } catch { try { child.kill(signal); } catch { /* already reaped */ } }
    };
    const terminate = setTimeout(() => stop("SIGTERM"), 1_000);
    const kill = setTimeout(() => stop("SIGKILL"), 2_000);
    terminate.unref(); kill.unref();
    child.once("close", () => { clearTimeout(terminate); clearTimeout(kill); });
  }
}
