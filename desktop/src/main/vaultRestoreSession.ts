// 복원본 전용 창의 열람 수명이다. 확인은 대상에 묶인 native helper가 직접 수행한다.
// 일반 renderer는 승인 여부·키·복원 평문을 받지 않는다.
import {
  isReconnectableHold,
  type VaultApprovedMapping,
  type VaultReconnectReview,
  type VaultRelockCause,
  type VaultRestoreHold,
  type VaultRestoreTarget,
  type VaultRestoreViewState,
} from "../shared/vaultRestore";

export type VaultHeldReply = { kind: "held"; reason: VaultRestoreHold };

/** 한 대상·한 native 확인에 묶인 로컬 경로이며 일반 Gateway를 사용하지 않는다. */
export interface VaultRestoreChannel {
  readonly capabilities: { readonly reconnect: boolean };
  prepare(): Promise<{ kind: "prepared" } | VaultHeldReply>;
  restore(): Promise<{ kind: "restored"; title: string; body: string } | VaultHeldReply>;
  review(): Promise<{ kind: "review"; review: VaultReconnectReview } | VaultHeldReply>;
  reconnect(reviewedHash: string, mappings: VaultApprovedMapping[]): Promise<{ kind: "reconnected" } | VaultHeldReply>;
  close(): void;
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

export class VaultRestoreSession {
  private generation = 0;
  private channel: VaultRestoreChannel | null = null;
  private expiry: ReturnType<typeof setTimeout> | undefined;
  private expiresAt: number | undefined;
  private review: VaultReconnectReview | null = null;

  constructor(private readonly deps: VaultRestoreSessionDeps) {}

  async unlock(): Promise<void> {
    const generation = this.end();
    const profileHold = this.deps.profileHold();
    if (profileHold) return this.deps.emit({ phase: "held", reason: profileHold });
    this.deps.emit({ phase: "opening" });
    let channel: VaultRestoreChannel | null;
    try { channel = this.deps.source(this.deps.target); } catch { channel = null; }
    if (!channel) return this.hold("source_unavailable");
    // 확인이 떠 있는 동안에도 잠금/창 종료가 helper를 취소할 수 있어야 한다.
    this.channel = channel;
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
    if (!channel?.capabilities.reconnect || !this.current(generation)) return;
    const reply = await channel.review().catch((): VaultHeldReply => ({ kind: "held", reason: "source_unavailable" }));
    if (!this.current(generation)) return;
    if (reply.kind === "held") return this.hold(reply.reason);
    this.review = reply.review;
    this.deps.emit({ phase: "review", review: reply.review });
  }

  async reconnect(mappings: unknown): Promise<void> {
    const { channel, generation, review } = this;
    if (!channel?.capabilities.reconnect || !review || !this.current(generation)) return;
    const approved = approvedMappings(review, mappings);
    if (!approved) return this.hold("mapping_rejected");
    const reply = await channel.reconnect(review.reviewedHash, approved).catch((): VaultHeldReply => ({ kind: "held", reason: "source_unavailable" }));
    if (!this.current(generation)) return;
    if (reply.kind === "held") return this.hold(reply.reason);
    this.review = null;
    await this.showRestored(generation, channel);
  }

  private async showRestored(generation: number, channel: VaultRestoreChannel): Promise<void> {
    if (!this.current(generation)) return;
    const reply = await channel.restore().catch((): VaultHeldReply => ({ kind: "held", reason: "source_unavailable" }));
    // 복호화가 끝난 뒤에도 세대·프로필·열람 만료를 다시 본다.
    if (!this.current(generation)) return;
    if (reply.kind === "held") return this.hold(reply.reason);
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

  private hold(reason: VaultRestoreHold): void {
    const canReconnect = isReconnectableHold(reason) && this.channel?.capabilities.reconnect === true;
    if (!canReconnect) this.end();
    this.deps.emit({ phase: "held", reason, ...(canReconnect ? { canReconnect: true } : {}) });
  }

  private now(): number { return (this.deps.now ?? Date.now)(); }

  private end(): number {
    this.generation += 1;
    clearTimeout(this.expiry);
    this.expiry = undefined;
    this.expiresAt = undefined;
    this.review = null;
    const channel = this.channel;
    this.channel = null;
    channel?.close();
    return this.generation;
  }
}

function approvedMappings(review: VaultReconnectReview, mappings: unknown): VaultApprovedMapping[] | null {
  if (!Array.isArray(mappings)) return null;
  const approved: VaultApprovedMapping[] = [];
  for (const mapping of mappings as Partial<VaultApprovedMapping>[]) {
    const marker = review.markers.find((candidate) => candidate.marker === mapping?.marker);
    const place = marker?.candidates.find((candidate) => candidate.field === mapping.field && candidate.position === mapping.position);
    if (!marker || !place || approved.some((other) => other.marker === marker.marker)) return null;
    approved.push({ marker: marker.marker, field: place.field, position: place.position });
  }
  return approved;
}
