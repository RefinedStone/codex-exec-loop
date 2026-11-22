// 대상에 묶인 native helper와만 통신한다. 평문·오류 원문을 로그/일반 IPC에 전달하지 않는다.
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomUUID } from "node:crypto";
import {
  copyVaultRestoreTarget, isVaultRestoreTarget,
  type VaultApprovedMapping, type VaultRestoreHold, type VaultRestoreTarget,
} from "../shared/vaultRestore";
import type { VaultHeldReply, VaultRestoreChannel, VaultRestoreSource } from "./vaultRestoreSession";

const MAX_INPUT_BYTES = 16 * 1024;
const MAX_OUTPUT_BYTES = 8 * 1024 * 1024;
const HELPER_LIFETIME_MS = 5 * 60_000;
const HELPER_HOLDS = new Set<VaultRestoreHold>([
  "source_unavailable", "platform_unsupported", "confirmation_cancelled", "locked",
  "latest_read_failed", "not_bound", "published_copy_changed", "marker_moved",
  "marker_duplicated", "marker_missing", "record_unavailable",
]);
type PreparedReply = { kind: "prepared" } | VaultHeldReply;
type RestoredReply = { kind: "restored"; title: string; body: string } | VaultHeldReply;
type Reply = PreparedReply | RestoredReply;
type Stage = "new" | "preparing" | "prepared" | "restoring" | "closed";

export interface VaultRestoreSourceDependencies {
  executable(): string;
  /** 시험은 메모리 stream으로 프로세스 경계를 대체한다. 제품 환경 변수로는 바꾸지 않는다. */
  spawnHelper?(executable: string, workspace: string): ChildProcessWithoutNullStreams;
}

export function createVaultRestoreSource(dependencies: VaultRestoreSourceDependencies): VaultRestoreSource {
  return (target) => isVaultRestoreTarget(target)
    ? new NativeVaultRestoreChannel(copyVaultRestoreTarget(target), dependencies)
    : null;
}

function spawnHelper(executable: string, workspace: string): ChildProcessWithoutNullStreams {
  return spawn(executable, ["desktop-restore-bridge"], {
    cwd: workspace,
    stdio: "pipe",
    windowsHide: true,
    // EOF로 native 확인을 취소하며, 응답하지 않는 helper 정리는 전용 process group으로 제한한다.
    detached: process.platform !== "win32",
  });
}

class NativeVaultRestoreChannel implements VaultRestoreChannel {
  readonly capabilities = { reconnect: false } as const;
  private readonly id = randomUUID();
  private stage: Stage = "new";
  private child: ChildProcessWithoutNullStreams | undefined;
  private bytes: Buffer<ArrayBufferLike> = Buffer.alloc(0);
  private receivedBytes = 0;
  private pending: ((reply: Reply) => void) | undefined;
  private terminalReply: Reply | undefined;
  private lifetime: NodeJS.Timeout | undefined;
  private processClosed = false;

  constructor(
    private readonly target: VaultRestoreTarget,
    private readonly dependencies: VaultRestoreSourceDependencies,
  ) {}

  prepare(): Promise<PreparedReply> {
    if (this.stage !== "new") return Promise.resolve(unavailable());
    this.stage = "preparing";
    const answer = new Promise<Reply>((resolve) => { this.pending = resolve; });
    this.lifetime = setTimeout(() => this.finish({ kind: "held", reason: "locked" }), HELPER_LIFETIME_MS);
    try {
      const child = (this.dependencies.spawnHelper ?? spawnHelper)(this.dependencies.executable(), this.target.workspace);
      this.child = child;
      child.stdout.on("data", (chunk: Buffer) => this.receive(chunk));
      child.stdout.on("error", () => this.finish(unavailable()));
      child.stderr.on("error", () => this.finish(unavailable()));
      child.stderr.resume();
      child.on("error", () => this.finish(unavailable()));
      child.stdin.on("error", () => this.finish(unavailable()));
      child.once("close", (code) => {
        this.processClosed = true;
        // 정상 종료까지 확인한 한 응답만 넘긴다. 잘린 JSON·추가 출력·비정상 종료는 보류한다.
        const reply = code === 0 ? this.terminalReply : undefined;
        this.finish(reply ?? unavailable());
      });
      this.write({
        version: 1, id: this.id, command: "prepare",
        issue_url: this.target.issueUrl, issue: this.target.issue,
      });
    } catch {
      this.finish(unavailable());
    }
    return answer.then((reply) => reply.kind === "prepared" || reply.kind === "held" ? reply : unavailable());
  }

  restore(): Promise<RestoredReply> {
    if (this.stage !== "prepared") return Promise.resolve(unavailable());
    this.stage = "restoring";
    const answer = new Promise<Reply>((resolve) => { this.pending = resolve; });
    // 대상과 승인값을 다시 받지 않는다. native helper 안에 남아 있는 같은 대상만 복원한다.
    this.write({ version: 1, id: this.id, command: "restore" });
    return answer.then((reply) => reply.kind === "restored" || reply.kind === "held" ? reply : unavailable());
  }

  review(): Promise<VaultHeldReply> { return Promise.resolve(unavailable()); }
  reconnect(_hash: string, _mappings: VaultApprovedMapping[]): Promise<VaultHeldReply> { return Promise.resolve(unavailable()); }
  close(): void { this.finish(unavailable()); }

  private write(value: unknown): void {
    const line = JSON.stringify(value) + "\n";
    if (!this.child || Buffer.byteLength(line, "utf8") > MAX_INPUT_BYTES ||
      this.child.stdin.writableLength > MAX_INPUT_BYTES) return this.finish(unavailable());
    try { this.child.stdin.write(line); } catch { this.finish(unavailable()); }
  }

  private receive(chunk: Buffer): void {
    if (this.stage === "closed") return;
    this.receivedBytes += chunk.length;
    if (this.receivedBytes > MAX_OUTPUT_BYTES || this.terminalReply || !this.pending) return this.finish(unavailable());
    this.bytes = Buffer.concat([this.bytes, chunk]);
    const end = this.bytes.indexOf(10);
    if (end < 0) return;
    // 다음 응답은 다음 요청 뒤에만 온다. 같은 chunk의 여분 frame도 허용하지 않는다.
    if (end !== this.bytes.length - 1) return this.finish(unavailable());
    try {
      const value: unknown = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(this.bytes.subarray(0, end)));
      this.bytes = Buffer.alloc(0);
      if (!record(value) || !fields(value, ["version", "id", "result"]) ||
        value.version !== 1 || value.id !== this.id || !record(value.result)) return this.finish(unavailable());
      const result = value.result;
      if (result.kind === "held" && fields(result, ["kind", "reason"]) &&
        typeof result.reason === "string" && HELPER_HOLDS.has(result.reason as VaultRestoreHold)) {
        this.terminalReply = { kind: "held", reason: result.reason as VaultRestoreHold };
        this.child?.stdin.end();
      } else if (this.stage === "preparing" && result.kind === "prepared" && fields(result, ["kind"])) {
        this.stage = "prepared";
        const resolve = this.pending;
        this.pending = undefined;
        resolve?.({ kind: "prepared" });
      } else if (this.stage === "restoring" && result.kind === "restored" &&
        fields(result, ["kind", "title", "body"]) && typeof result.title === "string" && typeof result.body === "string") {
        this.terminalReply = { kind: "restored", title: result.title, body: result.body };
        this.child?.stdin.end();
      } else {
        this.finish(unavailable());
      }
    } catch {
      this.finish(unavailable());
    }
  }

  private finish(reply: Reply): void {
    if (this.stage === "closed") return;
    this.stage = "closed";
    clearTimeout(this.lifetime);
    this.lifetime = undefined;
    this.bytes = Buffer.alloc(0);
    this.terminalReply = undefined;
    const resolve = this.pending;
    this.pending = undefined;
    resolve?.(reply);
    const child = this.child;
    if (!child || this.processClosed) return;
    try { child.stdin.end(); } catch { /* 이미 닫힌 pipe이다. 오류 원문은 넘기지 않는다. */ }
    const stop = (signal: NodeJS.Signals) => {
      if (this.processClosed) return;
      try {
        if (process.platform !== "win32" && child.pid) process.kill(-child.pid, signal);
        else child.kill(signal);
      } catch { try { child.kill(signal); } catch { /* 종료 경합 */ } }
    };
    const terminate = setTimeout(() => stop("SIGTERM"), 1_000);
    const kill = setTimeout(() => stop("SIGKILL"), 2_000);
    terminate.unref();
    kill.unref();
    child.once("close", () => { clearTimeout(terminate); clearTimeout(kill); });
  }
}

function unavailable(): VaultHeldReply { return { kind: "held", reason: "source_unavailable" }; }
function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function fields(value: Record<string, unknown>, allowed: readonly string[]): boolean {
  return Object.keys(value).every((key) => allowed.includes(key));
}
