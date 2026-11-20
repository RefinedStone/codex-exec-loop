// 로컬 복원본 창 하나의 열람 세션이다 (#2427). Electron 을 쓰지 않는다: 시험이 가짜로 돌린다.
//
// 규칙:
// - 열 때마다 실행 프로필을 보고, 사용자 확인을 받고, 그 뒤에만 로컬 복원 경로를 연다.
// - 잠그면 세대(generation)를 올린다. 잠근 뒤에 도착한 답은 세대가 달라서 버린다.
// - 복원 평문을 이 객체에 보관하지 않는다. 받은 값은 `emit` 으로 한 번 넘기고 끝이다.
import {
  isReconnectableHold,
  type VaultApprovedMapping,
  type VaultReconnectReview,
  type VaultRelockCause,
  type VaultRestoreHold,
  type VaultRestoreViewState,
} from "../shared/vaultRestore";

export type VaultHeldReply = { kind: "held"; reason: VaultRestoreHold };

/**
 * 이슈 한 건의 열람 세션 하나에 묶인 로컬 복원 경로이다. Akra 의 Vault 복원 서비스 (#2425) 쪽 끝이다.
 * 구현은 일반 Gateway 와 Kernel 프로토콜을 쓰지 않는다.
 */
export interface VaultRestoreChannel {
  restore(): Promise<{ kind: "restored"; title: string; body: string } | VaultHeldReply>;
  review(): Promise<{ kind: "review"; review: VaultReconnectReview } | VaultHeldReply>;
  reconnect(reviewedHash: string, mappings: VaultApprovedMapping[]): Promise<{ kind: "reconnected" } | VaultHeldReply>;
  /** 열람 세션을 끝낸다. 그 뒤의 요청은 답을 주지 않는다. */
  close(): void;
}

/** 사용자가 확인한 뒤에만 부른다. 로컬 복원 경로가 없으면 null 이다. */
export type VaultRestoreSource = (issueUrl: string) => VaultRestoreChannel | null;

export interface VaultRestoreSessionDeps {
  issueUrl: string;
  source: VaultRestoreSource;
  /** 신뢰된 로컬 확인이다 (main 의 시스템 대화상자). renderer 가 대신 답할 수 없다. */
  confirm(): Promise<boolean>;
  /** 지금 실행 프로필에서 열람을 보류해야 하면 그 이유를 준다. */
  profileHold(): VaultRestoreHold | null;
  emit(state: VaultRestoreViewState): void;
  /** 복원본을 연 뒤 다시 잠글 때까지의 시간이다. */
  sessionMs: number;
  now?: () => number;
}

export class VaultRestoreSession {
  private generation = 0;
  private channel: VaultRestoreChannel | null = null;
  private expiry: ReturnType<typeof setTimeout> | undefined;
  /** 검토 중인 값이다. 평문은 없다. 승인을 이 값에 맞춰 본다. */
  private review: VaultReconnectReview | null = null;

  constructor(private readonly deps: VaultRestoreSessionDeps) {}

  /** 사용자 확인을 거쳐 복원본을 연다. 이미 열려 있었으면 먼저 닫는다. */
  async unlock(): Promise<void> {
    const generation = this.end();
    const profileHold = this.deps.profileHold();
    if (profileHold) return this.deps.emit({ phase: "held", reason: profileHold });
    this.deps.emit({ phase: "opening" });
    const confirmed = await this.deps.confirm().catch(() => false);
    if (generation !== this.generation) return;
    if (!confirmed) return this.deps.emit({ phase: "held", reason: "confirmation_cancelled" });
    // 확인 창이 떠 있는 동안 실행 프로필이 바뀌었을 수 있다.
    const lateHold = this.deps.profileHold();
    if (lateHold) return this.deps.emit({ phase: "held", reason: lateHold });
    const channel = this.deps.source(this.deps.issueUrl);
    if (!channel) return this.deps.emit({ phase: "held", reason: "source_unavailable" });
    this.channel = channel;
    this.expiry = setTimeout(() => this.lock("session_expired"), this.deps.sessionMs);
    await this.showRestored(generation, channel);
  }

  /** 다시 잠근다. 로컬 복원 경로를 닫고, 진행 중인 요청의 답을 버리고, 화면에 잠금을 알린다. */
  lock(cause: VaultRelockCause): void {
    this.end();
    this.deps.emit({ phase: "locked", cause });
  }

  /** 재연결 검토의 값을 읽는다. 열린 세션이 없으면 아무것도 하지 않는다. */
  async requestReview(): Promise<void> {
    const { channel, generation } = this;
    if (!channel) return;
    const reply = await channel.review().catch((): VaultHeldReply => ({ kind: "held", reason: "source_unavailable" }));
    if (generation !== this.generation) return;
    if (reply.kind === "held") return this.hold(reply.reason);
    this.review = reply.review;
    this.deps.emit({ phase: "review", review: reply.review });
  }

  /** 사용자가 승인한 연결을 확정하고 복원본을 다시 읽는다. 검토한 값에 없는 표식이나 자리는 거절한다. */
  async reconnect(mappings: unknown): Promise<void> {
    const { channel, generation, review } = this;
    if (!channel || !review) return;
    const approved = approvedMappings(review, mappings);
    if (!approved) return this.hold("mapping_rejected");
    const reply = await channel.reconnect(review.reviewedHash, approved).catch((): VaultHeldReply => ({ kind: "held", reason: "source_unavailable" }));
    if (generation !== this.generation) return;
    if (reply.kind === "held") return this.hold(reply.reason);
    this.review = null;
    await this.showRestored(generation, channel);
  }

  private async showRestored(generation: number, channel: VaultRestoreChannel): Promise<void> {
    const reply = await channel.restore().catch((): VaultHeldReply => ({ kind: "held", reason: "source_unavailable" }));
    // 원격 읽기와 복호화 동안 잠겼으면 늦은 결과를 화면에 주지 않는다.
    if (generation !== this.generation) return;
    if (reply.kind === "held") return this.hold(reply.reason);
    this.deps.emit({ phase: "restored", title: reply.title, body: reply.body, expiresAt: (this.deps.now ?? Date.now)() + this.deps.sessionMs });
  }

  /** 보류를 알린다. 재연결 검토를 할 수 없는 보류이면 세션도 끝낸다. */
  private hold(reason: VaultRestoreHold): void {
    if (!isReconnectableHold(reason)) this.end();
    this.deps.emit({ phase: "held", reason });
  }

  private end(): number {
    this.generation += 1;
    clearTimeout(this.expiry);
    this.expiry = undefined;
    this.review = null;
    const channel = this.channel;
    this.channel = null;
    channel?.close();
    return this.generation;
  }
}

/** renderer 가 보낸 승인을 검토한 값과 맞춰 본다. 표식은 한 번씩만, 자리는 검토에 나온 후보만 받는다. */
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
