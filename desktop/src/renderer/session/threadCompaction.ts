import { JsonRpcError } from "../rpc/jsonRpcClient";

/** 공식 app-server 요청은 접수 시 빈 객체만 돌려준다. 완료는 해당 턴의 항목 생명주기로 확인한다. */
export const COMPACTION_RESPONSE_TIMEOUT_MS = 30_000;
export const COMPACTION_COMPLETION_TIMEOUT_MS = 180_000;
export type CompactionPhase = "idle" | "requesting" | "running" | "completed" | "failed" | "unsupported" | "unknown";
export interface CompactionState {
  phase: CompactionPhase;
  threadId: string | null;
  turnId: string | null;
  checking: boolean;
  message: string;
}
export interface CompactionContext {
  threadId: string | null;
  connected: boolean;
  writable: boolean;
  loaded: boolean;
  turnInProgress: boolean;
  submissionInFlight: boolean;
  knownTurnIds: readonly string[];
}
interface CompactionRuntime {
  request<T>(method: string, params: unknown): Promise<T>;
  context(): CompactionContext;
  reload(threadId: string, turnId: string, status: string): Promise<boolean>;
}
interface Operation {
  threadId: string;
  epoch: number;
  baselineTurnId: string;
  knownTurnIds: Set<string>;
  turnId: string | null;
  itemId: string | null;
  compactionCompleted: boolean;
  reconciling: boolean;
  timer: ReturnType<typeof setTimeout> | null;
}
const IDLE: CompactionState = { phase: "idle", threadId: null, turnId: null, checking: false, message: "" };
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
const text = (value: unknown): string | null => typeof value === "string" && value.length > 0 ? value : null;

export class ThreadCompactionController {
  private readonly states = new Map<string, CompactionState>();
  private readonly operations = new Map<string, Operation>();
  /** transcript 프레임을 기다리지 않고 실제 시작 알림부터 압축과 일반 턴의 경합을 막는다. */
  private readonly activeTurns = new Map<string, string | null>();
  private readonly listeners = new Set<() => void>();
  private epoch = 0;

  constructor(private readonly runtime: CompactionRuntime) {}

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  snapshot(threadId: string | null): CompactionState { return threadId ? this.states.get(threadId) ?? IDLE : IDLE; }

  isBlocking(threadId: string): boolean {
    if (this.operations.has(threadId)) return true;
    const state = this.snapshot(threadId);
    const context = this.runtime.context();
    // terminal 알림 다음 프레임까지 옛 압축 턴에 steer를 보내지 않는다.
    return context.threadId === threadId && context.turnInProgress && context.knownTurnIds.at(-1) === state.turnId
      && (state.phase === "completed" || state.phase === "failed");
  }

  unavailableReason(threadId: string): string | null {
    const context = this.runtime.context();
    if (!context.connected) return "Gateway에 연결한 뒤 다시 시도하세요.";
    if (context.threadId !== threadId || !context.loaded) return "현재 대화의 기록을 먼저 불러오세요.";
    if (!context.writable) return "읽기 전용이거나 다른 앱이 사용 중인 대화는 압축할 수 없습니다.";
    if (context.turnInProgress || context.submissionInFlight || this.activeTurns.has(threadId)) return "진행 중이거나 전송 중인 턴이 끝난 뒤 압축할 수 있습니다.";
    if (this.isBlocking(threadId)) return "이미 요청한 압축의 결과를 먼저 확인하세요.";
    if (this.snapshot(threadId).phase === "unsupported") return "현재 엔진은 수동 문맥 압축을 지원하지 않습니다.";
    if (!context.knownTurnIds.length) return "완료된 대화가 생긴 뒤 압축할 수 있습니다.";
    return null;
  }

  start(threadId: string): void {
    const reason = this.unavailableReason(threadId);
    if (reason) throw new Error(reason);
    const context = this.runtime.context();
    const operation: Operation = { threadId, epoch: this.epoch, baselineTurnId: context.knownTurnIds.at(-1)!,
      knownTurnIds: new Set(context.knownTurnIds), turnId: null, itemId: null, compactionCompleted: false, reconciling: false, timer: null };
    this.operations.set(threadId, operation);
    this.update(operation, "requesting", "엔진에 문맥 압축을 요청하는 중입니다.");
    this.armTimeout(operation, COMPACTION_RESPONSE_TIMEOUT_MS, "요청 응답을 받지 못했습니다. 중복 요청하지 않고 결과를 확인합니다.");
    const requestEpoch = this.epoch;
    void this.runtime.request<unknown>("thread/compact/start", { threadId }).then(response => {
      if (!this.current(operation, requestEpoch)) return;
      if (!object(response) || Object.keys(response).length !== 0) {
        this.unknown(operation, "엔진 응답 형식을 확인하지 못했습니다. 압축 결과를 다시 확인하세요."); return;
      }
      this.update(operation, "running", "엔진이 요청을 접수했습니다. 압축 완료를 기다립니다.");
      this.armTimeout(operation, COMPACTION_COMPLETION_TIMEOUT_MS, "압축 완료를 아직 확인하지 못했습니다. 상태를 다시 확인하세요.");
    }).catch((error: unknown) => {
      if (!this.current(operation, requestEpoch)) return;
      if (error instanceof JsonRpcError) {
        const unsupported = error.code === -32601;
        this.finish(operation, unsupported ? "unsupported" : "failed", unsupported
          ? "현재 엔진은 수동 문맥 압축을 지원하지 않습니다."
          : `엔진이 압축 요청을 거절했습니다: ${error.message.slice(0, 400)}`);
      } else {
        this.unknown(operation, "요청 전달 결과를 확인하지 못했습니다. 연결 상태를 확인한 뒤 결과를 다시 확인하세요.");
      }
    });
  }

  connectionChanged(connected: boolean): void {
    if (!connected) {
      this.epoch++;
      this.activeTurns.clear();
      for (const operation of this.operations.values()) this.unknown(operation, "연결이 끊겨 압축 결과를 확인하지 못했습니다. 재연결 뒤 상태를 다시 확인하세요.");
    } else {
      for (const [threadId, state] of this.states) {
        if (state.phase === "unsupported") this.states.delete(threadId);
      }
      this.emit();
    }
  }

  dispose(): void {
    this.epoch++;
    for (const operation of this.operations.values()) if (operation.timer) clearTimeout(operation.timer);
    this.operations.clear();
    this.activeTurns.clear();
    this.states.clear();
    this.emit();
    this.listeners.clear();
  }

  /** 같은 thread와 접속에서 새로 시작한 턴/compact 항목만 이 요청의 관측에 사용한다. */
  observe(method: string, params: unknown): void {
    if (!object(params)) return;
    const threadId = text(params.threadId);
    if (!threadId) return;
    const turn = object(params.turn) ? params.turn : null;
    const turnId = text(params.turnId) ?? text(turn?.id);
    if (method === "turn/started" && turnId) this.activeTurns.set(threadId, turnId);
    if (method === "turn/completed" && turnId && (turn?.status === "completed" || turn?.status === "failed" || turn?.status === "interrupted")
      && (this.activeTurns.get(threadId) === turnId || this.activeTurns.get(threadId) === null)) this.activeTurns.delete(threadId);
    if (method === "thread/status/changed" && object(params.status)) {
      if (params.status.type === "active" && !this.activeTurns.has(threadId)) this.activeTurns.set(threadId, null);
      if (params.status.type === "idle") this.activeTurns.delete(threadId);
    }
    const operation = this.operations.get(threadId);
    if (!operation || operation.epoch !== this.epoch || operation.reconciling) return;
    if (!turnId || operation.knownTurnIds.has(turnId)) return;
    const item = object(params.item) ? params.item : null;
    const compactItem = item?.type === "contextCompaction" && text(item.id) !== null;
    if (!operation.turnId && (method === "turn/started" || compactItem)) operation.turnId = turnId;
    if (operation.turnId !== turnId) return;
    if (method === "item/started" && compactItem) {
      operation.itemId = text(item!.id);
      this.update(operation, "running", "엔진이 문맥을 압축하고 있습니다.");
    } else if (method === "item/completed" && compactItem && (!operation.itemId || operation.itemId === item!.id)) {
      operation.compactionCompleted = true;
      this.update(operation, "running", "압축 항목이 완료되어 턴의 최종 결과를 확인하고 있습니다.");
    } else if (method === "thread/compacted") {
      // 구형 엔진의 알림은 이미 관측한 같은 턴에만 적용한다.
      operation.compactionCompleted = true;
      this.update(operation, "running", "압축 알림을 받았습니다. 턴의 최종 결과를 확인하고 있습니다.");
    } else if (method === "error" && params.willRetry === false) {
      this.finish(operation, "failed", "엔진이 압축 중 오류를 보고했습니다.");
    } else if (method === "turn/completed" && turn) {
      this.applyTurn(operation, turn);
    }
  }

  /** 자동 재요청 없이 공식 read로만 확인한다. 기존 마지막 턴 이후의 compact 턴을 증거로 쓴다. */
  async refresh(threadId: string): Promise<void> {
    const operation = this.operations.get(threadId);
    if (!operation || this.snapshot(threadId).checking) return;
    if (!this.runtime.context().connected) throw new Error("Gateway에 다시 연결한 뒤 상태를 확인하세요.");
    const requestEpoch = this.epoch;
    this.states.set(threadId, { ...this.snapshot(threadId), checking: true }); this.emit();
    try {
      const response = await this.withDeadline(this.runtime.request<unknown>("thread/turns/list", {
        threadId, limit: 20, sortDirection: "desc", itemsView: "full",
      }));
      if (!this.current(operation, requestEpoch)) return;
      if (!object(response) || !Array.isArray(response.data)) { this.unknown(operation, "대화 기록에서 압축 결과를 확인하지 못했습니다."); return; }
      const turns = response.data.slice(0, 20).filter(object).reverse();
      let candidate = operation.turnId ? turns.find(turn => turn.id === operation.turnId) : undefined;
      if (!candidate && !operation.turnId) {
        const baseline = turns.findIndex(turn => turn.id === operation.baselineTurnId);
        const after = baseline < 0 ? [] : turns.slice(baseline + 1).filter(turn => this.hasCompaction(turn));
        // 명확한 기준 턴과 새 압축 턴 하나가 있을 때만 결과를 회복한다.
        if (after.length === 1) candidate = after[0];
      }
      if (!candidate || !text(candidate.id)) { this.unknown(operation, "요청한 압축의 턴을 특정하지 못했습니다. 중복 요청하지 않고 결과 불명으로 유지합니다."); return; }
      operation.turnId = text(candidate.id);
      if (candidate.status === "completed" || candidate.status === "failed" || candidate.status === "interrupted") {
        operation.reconciling = true;
        const confirmed = await this.withDeadline(this.runtime.reload(threadId, operation.turnId!, candidate.status));
        if (!this.current(operation, requestEpoch)) return;
        if (!confirmed) { this.unknown(operation, "현재 대화에서 압축 결과를 다시 확인하지 못했습니다. 결과 불명으로 유지합니다."); return; }
      }
      operation.epoch = this.epoch;
      this.applyTurn(operation, candidate);
    } catch {
      if (this.current(operation, requestEpoch)) this.unknown(operation, "대화 기록을 읽지 못했습니다. 결과 불명 상태로 유지합니다.");
    } finally {
      operation.reconciling = false;
      if (this.current(operation, requestEpoch) && this.snapshot(threadId).checking) {
        this.states.set(threadId, { ...this.snapshot(threadId), checking: false }); this.emit();
      }
    }
  }

  private hasCompaction(turn: Record<string, unknown>): boolean {
    return Array.isArray(turn.items) && turn.items.some(item => object(item) && item.type === "contextCompaction");
  }

  private applyTurn(operation: Operation, turn: Record<string, unknown>): void {
    if ((turn.status === "completed" || turn.status === "failed" || turn.status === "interrupted")
      && (this.activeTurns.get(operation.threadId) === null || this.activeTurns.get(operation.threadId) === operation.turnId)) {
      this.activeTurns.delete(operation.threadId);
    }
    if (turn.status === "failed" || turn.status === "interrupted") {
      this.finish(operation, "failed", turn.status === "interrupted" ? "압축 턴이 중단되었습니다." : "압축 턴이 실패했습니다.");
    } else if (turn.status === "completed" && (operation.compactionCompleted || this.hasCompaction(turn))) {
      this.finish(operation, "completed", "대화 기록에서 문맥 압축 완료를 확인했습니다.");
    } else if (turn.status === "inProgress") {
      this.update(operation, "running", "엔진의 압축 턴이 아직 진행 중입니다.");
      this.armTimeout(operation, COMPACTION_COMPLETION_TIMEOUT_MS, "압축 완료를 아직 확인하지 못했습니다. 상태를 다시 확인하세요.");
    } else this.unknown(operation, "턴은 종료됐지만 압축 완료를 확인할 항목이 없습니다.");
  }

  private current(operation: Operation, epoch: number): boolean {
    return this.operations.get(operation.threadId) === operation && this.epoch === epoch;
  }

  private async withDeadline<T>(pending: Promise<T>): Promise<T> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      return await Promise.race([pending, new Promise<never>((_resolve, reject) => {
        timer = setTimeout(() => reject(new Error("timeout")), COMPACTION_RESPONSE_TIMEOUT_MS);
      })]);
    } finally { if (timer) clearTimeout(timer); }
  }

  private armTimeout(operation: Operation, delay: number, message: string): void {
    if (operation.timer) clearTimeout(operation.timer);
    operation.timer = setTimeout(() => { if (this.operations.get(operation.threadId) === operation) this.unknown(operation, message); }, delay);
  }

  private unknown(operation: Operation, message: string): void {
    if (operation.timer) clearTimeout(operation.timer);
    operation.timer = null;
    this.update(operation, "unknown", message);
  }

  private finish(operation: Operation, phase: "completed" | "failed" | "unsupported", message: string): void {
    if (operation.timer) clearTimeout(operation.timer);
    this.operations.delete(operation.threadId);
    this.update(operation, phase, message);
  }

  private update(operation: Operation, phase: CompactionPhase, message: string): void {
    this.states.set(operation.threadId, { phase, threadId: operation.threadId, turnId: operation.turnId, checking: false, message });
    this.emit();
  }

  private emit(): void { for (const listener of this.listeners) listener(); }
}
