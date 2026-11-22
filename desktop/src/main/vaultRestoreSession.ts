// 복원본 전용 창의 열람 수명이다. 확인은 대상에 묶인 native helper가 직접 수행한다.
// 일반 renderer는 승인 여부·키·복원 평문을 받지 않는다.
import {
  approvedVaultMappings, isReconnectableHold,
  type VaultApprovedMapping, type VaultReconnectReview, type VaultRelockCause,
  type VaultRestoreHold, type VaultRestoreTarget, type VaultRestoreViewState,
} from "../shared/vaultRestore";

/** 명시적인 true만 검토를 허용한다. 같은 보류 이유여도 최종 복원은 다시 연결하지 않는다. */
export type VaultHeldReply = { kind: "held"; reason: VaultRestoreHold; canReconnect?: boolean };

/** 한 대상·한 native 확인에 묶인 로컬 경로이며 일반 Gateway를 사용하지 않는다. */
export interface VaultRestoreChannel {
  readonly capabilities: { readonly reconnect: boolean };
  prepare(): Promise<{ kind: "prepared" } | VaultHeldReply>;
  restore(): Promise<{ kind: "restored"; title: string; body: string } | VaultHeldReply>;
  review(): Promise<{ kind: "review"; review: VaultReconnectReview } | VaultHeldReply>;
  reconnect(reviewedHash: string, mappings: VaultApprovedMapping[]): Promise<{ kind: "reconnected" } | VaultHeldReply>;
  close(): void;
  /** 요청 사이에 helper가 종료돼도 화면의 검토·저장 권한을 즉시 회수한다. */
  onClosed?(listener: (reason: VaultRestoreHold) => void): () => void;
}
export type VaultRestoreSource = (target: VaultRestoreTarget) => VaultRestoreChannel | null;
export interface VaultRestoreSessionDeps {
  target: VaultRestoreTarget;
  source: VaultRestoreSource;
  profileHold(): VaultRestoreHold | null;
  emit(state: VaultRestoreViewState): void;
  sessionMs: number;
  now?: () => number;
}
type Phase = "locked" | "opening" | "restoring" | "held" | "reviewing" | "review" | "reconnecting" | "restored";

export class VaultRestoreSession {
  private generation = 0;
  private channel: VaultRestoreChannel | null = null;
  private expiry: ReturnType<typeof setTimeout> | undefined;
  private expiresAt: number | undefined;
  private review: VaultReconnectReview | null = null;
  private phase: Phase = "locked";
  private canReview = false;
  private reconnectUsed = false;
  private unsubscribeChannelClose: (() => void) | undefined;

  constructor(private readonly deps: VaultRestoreSessionDeps) {}

  async unlock(): Promise<void> {
    const generation = this.end();
    const profileHold = this.deps.profileHold();
    if (profileHold) return this.deps.emit({ phase: "held", reason: profileHold });
    this.phase = "opening";
    this.deps.emit({ phase: "opening" });
    let channel: VaultRestoreChannel | null;
    try { channel = this.deps.source(this.deps.target); } catch { channel = null; }
    if (!channel) return this.hold("source_unavailable");
    this.channel = channel;
    this.unsubscribeChannelClose = channel.onClosed?.((reason) => {
      if (generation === this.generation && this.channel === channel) this.hold(reason);
    });
    const prepared = await channel.prepare().catch((): VaultHeldReply => ({ kind: "held", reason: "source_unavailable" }));
    if (!this.current(generation)) return;
    if (prepared.kind === "held") return this.hold(prepared.reason);
    this.expiresAt = this.now() + this.deps.sessionMs;
    this.expiry = setTimeout(() => this.lock("session_expired"), this.deps.sessionMs);
    await this.showRestored(generation, channel);
  }

  lock(cause: VaultRelockCause): void {
    this.end();
    this.deps.emit({ phase: "locked", cause });
  }

  async requestReview(): Promise<void> {
    const { channel, generation } = this;
    if (this.phase !== "held" || !this.canReview || !channel || !this.current(generation)) return;
    if (!channel.capabilities.reconnect) return this.hold("source_unavailable");
    this.canReview = false;
    this.phase = "reviewing";
    this.deps.emit({ phase: "reviewing" });
    const reply = await channel.review().catch((): VaultHeldReply => ({ kind: "held", reason: "source_unavailable" }));
    if (!this.current(generation)) return;
    if (reply.kind === "held") return this.hold(reply.reason);
    this.review = reply.review;
    this.phase = "review";
    this.deps.emit({ phase: "review", review: reply.review });
  }

  async reconnect(mappings: unknown): Promise<void> {
    const { channel, generation, review } = this;
    if (this.phase !== "review" || this.reconnectUsed || !channel || !review || !this.current(generation)) return;
    if (!channel.capabilities.reconnect) return this.hold("source_unavailable");
    const approved = approvedVaultMappings(review, mappings);
    if (!approved) return this.hold("mapping_rejected");
    this.review = null;
    this.reconnectUsed = true;
    this.phase = "reconnecting";
    this.deps.emit({ phase: "reconnecting" });
    const reply = await channel.reconnect(review.reviewedHash, approved)
      .catch((): VaultHeldReply => ({ kind: "held", reason: "write_outcome_unknown" }));
    if (!this.current(generation)) return;
    if (reply.kind === "held") return this.hold(reply.reason);
    await this.showRestored(generation, channel);
  }

  private async showRestored(generation: number, channel: VaultRestoreChannel): Promise<void> {
    if (!this.current(generation)) return;
    this.phase = "restoring";
    const reply = await channel.restore().catch((): VaultHeldReply => ({ kind: "held", reason: "source_unavailable" }));
    if (!this.current(generation)) return;
    if (reply.kind === "held") return this.hold(reply.reason, !this.reconnectUsed && reply.canReconnect === true);
    this.phase = "restored";
    this.deps.emit({ phase: "restored", title: reply.title, body: reply.body, expiresAt: this.expiresAt! });
  }

  private current(generation: number): boolean {
    if (generation !== this.generation) return false;
    const profileHold = this.deps.profileHold();
    if (profileHold) { this.hold(profileHold); return false; }
    if (this.expiresAt !== undefined && this.now() >= this.expiresAt) {
      this.lock("session_expired");
      return false;
    }
    return true;
  }

  private hold(reason: VaultRestoreHold, offered = false): void {
    const canReconnect = offered && isReconnectableHold(reason) && this.channel?.capabilities.reconnect === true;
    if (!canReconnect) this.end();
    else { this.phase = "held"; this.canReview = true; }
    this.deps.emit({ phase: "held", reason, ...(canReconnect ? { canReconnect: true } : {}) });
  }

  private now(): number { return (this.deps.now ?? Date.now)(); }

  private end(): number {
    this.generation += 1;
    clearTimeout(this.expiry);
    this.expiry = undefined;
    this.expiresAt = undefined;
    this.review = null;
    this.phase = "locked";
    this.canReview = false;
    this.reconnectUsed = false;
    this.unsubscribeChannelClose?.();
    this.unsubscribeChannelClose = undefined;
    const channel = this.channel;
    this.channel = null;
    channel?.close();
    return this.generation;
  }
}
