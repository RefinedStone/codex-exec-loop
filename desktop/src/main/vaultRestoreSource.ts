// 대상에 묶인 native helper와만 통신한다. 평문·오류 원문을 로그/일반 IPC에 전달하지 않는다.
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomUUID } from "node:crypto";
import {
  copyVaultRestoreTarget, isVaultRestoreTarget, approvedVaultMappings,
  type VaultApprovedMapping, type VaultReconnectReview, type VaultRestoreHold, type VaultRestoreTarget,
} from "../shared/vaultRestore";
import type { VaultHeldReply, VaultRestoreChannel, VaultRestoreSource } from "./vaultRestoreSession";
import { parseNativeVaultReply, type NativeVaultReply } from "./vaultRestoreProtocol";

const MAX_INPUT_BYTES = 16 * 1024;
const MAX_OUTPUT_BYTES = 8 * 1024 * 1024;
const HELPER_LIFETIME_MS = 5 * 60_000;
type PreparedReply = { kind: "prepared" } | VaultHeldReply;
type RestoredReply = { kind: "restored"; title: string; body: string } | VaultHeldReply;
type ReviewReply = { kind: "review"; review: VaultReconnectReview } | VaultHeldReply;
type ReconnectedReply = { kind: "reconnected" } | VaultHeldReply;
type Reply = PreparedReply | RestoredReply | ReviewReply | ReconnectedReply;
type Stage = "new" | "preparing" | "prepared" | "restoring" | "held" | "reviewing" |
  "reviewed" | "reconnecting" | "reconnected" | "final_restoring" | "closed";

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
    cwd: workspace, stdio: "pipe", windowsHide: true,
    // EOF로 native 확인을 취소하며, 응답하지 않는 helper 정리는 전용 process group으로 제한한다.
    detached: process.platform !== "win32",
  });
}

class NativeVaultRestoreChannel implements VaultRestoreChannel {
  private reconnectSupported = false;
  get capabilities(): { readonly reconnect: boolean } {
    return { reconnect: this.reconnectSupported && this.stage !== "closed" };
  }
  private readonly id = randomUUID();
  private stage: Stage = "new";
  private child: ChildProcessWithoutNullStreams | undefined;
  private bytes: Buffer<ArrayBufferLike> = Buffer.alloc(0);
  private receivedBytes = 0;
  private pending: ((reply: Reply) => void) | undefined;
  private terminalReply: Reply | undefined;
  private reviewSnapshot: VaultReconnectReview | undefined;
  private reconnectInFlight = false;
  private lifetime: NodeJS.Timeout | undefined;
  private processClosed = false;
  private readonly closedListeners = new Set<(reason: VaultRestoreHold) => void>();

  constructor(private readonly target: VaultRestoreTarget, private readonly dependencies: VaultRestoreSourceDependencies) {}

  prepare(): Promise<PreparedReply> {
    if (this.stage !== "new") return Promise.resolve(unavailable());
    this.stage = "preparing";
    const answer = this.waiting();
    this.lifetime = setTimeout(() => this.finish(this.reconnectInFlight
      ? outcomeUnknown() : { kind: "held", reason: "locked" }), HELPER_LIFETIME_MS);
    try {
      const child = (this.dependencies.spawnHelper ?? spawnHelper)(this.dependencies.executable(), this.target.workspace);
      this.child = child;
      child.stdout.on("data", (chunk: Buffer) => this.receive(chunk));
      child.stdout.on("error", () => this.finish(this.failure()));
      child.stderr.on("error", () => this.finish(this.failure()));
      child.stderr.resume();
      child.on("error", () => this.finish(this.failure()));
      child.stdin.on("error", () => this.finish(this.failure()));
      child.once("close", (code) => {
        this.processClosed = true;
        // 최종 응답만 정상 종료까지 확인한다. 잘린 JSON·추가 출력·비정상 종료는 보류한다.
        const reply = code === 0 ? this.terminalReply : undefined;
        this.finish(reply ?? this.failure());
      });
      this.write({ version: 2, id: this.id, command: "prepare", issue_url: this.target.issueUrl, issue: this.target.issue });
    } catch { this.finish(this.failure()); }
    return answer.then((reply) => reply.kind === "prepared" || reply.kind === "held" ? reply : unavailable());
  }

  restore(): Promise<RestoredReply> {
    if (this.stage !== "prepared" && this.stage !== "reconnected") return Promise.resolve(unavailable());
    this.stage = this.stage === "prepared" ? "restoring" : "final_restoring";
    const answer = this.waiting();
    this.write({ version: 2, id: this.id, command: "restore" });
    return answer.then((reply) => reply.kind === "restored" || reply.kind === "held" ? reply : unavailable());
  }

  review(): Promise<ReviewReply> {
    if (this.stage !== "held") return Promise.resolve(unavailable());
    this.stage = "reviewing";
    const answer = this.waiting();
    this.write({ version: 2, id: this.id, command: "review" });
    return answer.then((reply) => reply.kind === "review" || reply.kind === "held" ? reply : unavailable());
  }

  reconnect(hash: string, mappings: VaultApprovedMapping[]): Promise<ReconnectedReply> {
    if (this.stage !== "reviewed" || !this.reviewSnapshot) return Promise.resolve(unavailable());
    const approved = approvedVaultMappings(this.reviewSnapshot, mappings);
    if (hash !== this.reviewSnapshot.reviewedHash || !approved) {
      const refused: VaultHeldReply = { kind: "held", reason: hash !== this.reviewSnapshot.reviewedHash ? "changed_during_review" : "mapping_rejected" };
      this.finish(refused);
      return Promise.resolve(refused);
    }
    this.reviewSnapshot = undefined;
    this.stage = "reconnecting";
    const answer = this.waiting();
    this.write({ version: 2, id: this.id, command: "reconnect", reviewed_hash: hash, mappings: approved }, true);
    return answer.then((reply) => reply.kind === "reconnected" || reply.kind === "held" ? reply : unavailable());
  }

  close(): void { this.finish(this.failure()); }

  onClosed(listener: (reason: VaultRestoreHold) => void): () => void {
    this.closedListeners.add(listener);
    return () => { this.closedListeners.delete(listener); };
  }

  private waiting(): Promise<Reply> {
    return new Promise<Reply>((resolve) => { this.pending = resolve; });
  }

  private write(value: unknown, reconnect = false): void {
    const line = JSON.stringify(value) + "\n";
    if (!this.child || Buffer.byteLength(line, "utf8") > MAX_INPUT_BYTES ||
      this.child.stdin.writableLength > MAX_INPUT_BYTES) return this.finish(unavailable());
    // 요청을 pipe에 쓴 뒤 응답이 없으면 저장 여부를 추정하거나 자동 재전송하지 않는다.
    this.reconnectInFlight = reconnect;
    try { this.child.stdin.write(line); } catch { this.finish(this.failure()); }
  }

  private receive(chunk: Buffer): void {
    if (this.stage === "closed") return;
    this.receivedBytes += chunk.length;
    if (this.receivedBytes > MAX_OUTPUT_BYTES || this.terminalReply || !this.pending) return this.finish(this.failure());
    const previous = this.bytes;
    this.bytes = Buffer.concat([previous, chunk]);
    previous.fill(0);
    const end = this.bytes.indexOf(10);
    if (end < 0) return;
    // 다음 frame은 다음 요청 뒤에만 허용한다. 누적 출력 예산은 세션 전체에 적용한다.
    if (end !== this.bytes.length - 1) return this.finish(this.failure());
    try {
      const value: unknown = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(this.bytes.subarray(0, end)));
      this.bytes.fill(0);
      this.bytes = Buffer.alloc(0);
      const reply = parseNativeVaultReply(value, this.id);
      if (!reply) return this.finish(this.failure());
      this.accept(reply);
    } catch { this.finish(this.failure()); }
  }

  private accept(reply: NativeVaultReply): void {
    if (reply.kind === "held") {
      if (reply.canReconnect === true) {
        if (this.stage !== "restoring" || !this.reconnectSupported) return this.finish(this.failure());
        this.stage = "held";
        return this.resolve(reply);
      }
      this.terminalReply = reply;
      this.child?.stdin.end();
    } else if (reply.kind === "prepared" && this.stage === "preparing") {
      this.reconnectSupported = reply.capabilities.reconnect;
      this.stage = "prepared";
      this.resolve({ kind: "prepared" });
    } else if (reply.kind === "review" && this.stage === "reviewing") {
      this.reviewSnapshot = structuredClone(reply.review);
      this.stage = "reviewed";
      this.resolve(reply);
    } else if (reply.kind === "reconnected" && this.stage === "reconnecting") {
      this.reconnectInFlight = false;
      this.stage = "reconnected";
      this.resolve(reply);
    } else if (reply.kind === "restored" && (this.stage === "restoring" || this.stage === "final_restoring")) {
      this.terminalReply = reply;
      this.child?.stdin.end();
    } else { this.finish(this.failure()); }
  }

  private resolve(reply: Reply): void {
    const resolve = this.pending;
    this.pending = undefined;
    resolve?.(reply);
  }

  private failure(): VaultHeldReply { return this.reconnectInFlight ? outcomeUnknown() : unavailable(); }

  private finish(reply: Reply): void {
    if (this.stage === "closed") return;
    const idle = !this.pending && !this.terminalReply;
    this.stage = "closed";
    clearTimeout(this.lifetime);
    this.lifetime = undefined;
    this.bytes.fill(0);
    this.bytes = Buffer.alloc(0);
    this.terminalReply = undefined;
    this.reviewSnapshot = undefined;
    this.reconnectInFlight = false;
    this.resolve(reply);
    const listeners = [...this.closedListeners];
    this.closedListeners.clear();
    if (idle) for (const listener of listeners) listener(reply.kind === "held" ? reply.reason : "source_unavailable");
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
function outcomeUnknown(): VaultHeldReply { return { kind: "held", reason: "write_outcome_unknown" }; }
