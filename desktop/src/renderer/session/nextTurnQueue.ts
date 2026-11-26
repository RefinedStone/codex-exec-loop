/** Window-memory reservations owned by one DesktopSession (one host). */
export type QueuedInputState = "queued" | "sending" | "unknown";
export interface QueuedInput { id: string; text: string; state: QueuedInputState }
export interface NextTurnQueueState {
  items: readonly QueuedInput[];
  waitingTurnId: string | null;
  paused: string | null;
}
const EMPTY: NextTurnQueueState = { items: [], waitingTurnId: null, paused: null };
export const QUEUED_INPUT_TIMEOUT_MS = 20_000;
const MAX_ITEMS = 20;
const MAX_TEXT = 32_000;
type TerminalStatus = "completed" | "failed" | "interrupted";

export class NextTurnQueue {
  private readonly states = new Map<string, NextTurnQueueState>();
  private readonly listeners = new Set<() => void>();
  private readonly inFlight = new Set<string>();
  private readonly terminals = new Map<string, TerminalStatus>();
  private serial = 0;
  private disposed = false;

  constructor(
    private readonly send: (threadId: string, text: string) => Promise<string>,
    private readonly canDispatch: (threadId: string) => boolean,
    private readonly preflight: (threadId: string) => string | null = () => null,
  ) {}

  snapshot = (threadId: string): NextTurnQueueState => this.states.get(threadId) ?? EMPTY;
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  hasInFlight(threadId: string): boolean { return this.inFlight.has(threadId); }

  started(threadId: string, turnId: string): void {
    const state = this.snapshot(threadId);
    const terminal = this.terminals.get(this.key(threadId, turnId));
    this.set(threadId, { ...state, waitingTurnId: terminal ? null : turnId,
      paused: terminal && terminal !== "completed" ? "턴이 중단되거나 실패해 예약을 멈췄습니다." : state.paused });
  }

  enqueue(threadId: string, text: string, activeTurnId: string | null): void {
    const trimmed = this.validText(text);
    const state = this.snapshot(threadId);
    if (state.items.length >= MAX_ITEMS) throw new Error("예약은 대화마다 20개까지 추가할 수 있습니다.");
    const waiting = state.waitingTurnId ?? activeTurnId;
    const completed = waiting && this.terminals.get(this.key(threadId, waiting));
    this.set(threadId, {
      ...state,
      items: [...state.items, { id: String(++this.serial), text: trimmed, state: "queued" }],
      waitingTurnId: completed ? null : waiting,
      paused: state.paused ?? (completed && completed !== "completed" ? "턴이 중단되거나 실패해 예약을 멈췄습니다." : null),
    });
    void this.drain(threadId);
  }

  edit(threadId: string, id: string, text: string): void {
    const value = this.validText(text);
    const state = this.snapshot(threadId);
    this.set(threadId, { ...state, items: state.items.map((item) => item.id === id && item.state === "queued" ? { ...item, text: value } : item) });
  }

  remove(threadId: string, id: string): void {
    const state = this.snapshot(threadId);
    if (state.items.some((item) => item.id === id && item.state === "sending")) return;
    this.set(threadId, { ...state, items: state.items.filter((item) => item.id !== id) });
  }

  move(threadId: string, id: string, direction: -1 | 1): void {
    const state = this.snapshot(threadId);
    const items = [...state.items];
    const index = items.findIndex((item) => item.id === id);
    const other = index + direction;
    if (index < 0 || other < 0 || other >= items.length || items[index]?.state !== "queued" || items[other]?.state !== "queued") return;
    [items[index], items[other]] = [items[other]!, items[index]!];
    this.set(threadId, { ...state, items });
  }

  pause(threadId: string, reason: string): void {
    const state = this.snapshot(threadId);
    if (state.items.length || state.waitingTurnId) this.set(threadId, { ...state, paused: reason });
  }

  pauseAll(reason: string): void {
    for (const threadId of this.states.keys()) this.pause(threadId, reason);
  }

  resume(threadId: string, activeTurnId: string | null): void {
    const state = this.snapshot(threadId);
    if (state.items.some((item) => item.state === "unknown")) {
      throw new Error("결과 불명인 입력은 대화 기록을 확인하고 목록에서 지운 뒤 계속하세요. 자동으로 다시 보내지 않습니다.");
    }
    if (!this.canDispatch(threadId)) throw new Error("현재 대화가 연결되고 쓰기 가능할 때 예약을 계속할 수 있습니다.");
    // Explicit continuation may precede the frame that removes a terminal turn.
    const terminal = activeTurnId !== null && this.terminals.has(this.key(threadId, activeTurnId));
    this.set(threadId, { ...state, paused: null, waitingTurnId: terminal ? null : activeTurnId });
    void this.drain(threadId);
  }

  terminal(threadId: string, turnId: string, status: TerminalStatus): void {
    this.terminals.set(this.key(threadId, turnId), status);
    if (this.terminals.size > 100) this.terminals.delete(this.terminals.keys().next().value!);
    const state = this.snapshot(threadId);
    if (state.waitingTurnId !== turnId) return;
    this.set(threadId, { ...state, waitingTurnId: null,
      paused: status === "completed" ? state.paused : "턴이 중단되거나 실패해 예약을 멈췄습니다. 확인 후 계속하세요." });
    void this.drain(threadId);
  }

  dispose(): void {
    this.disposed = true;
    this.states.clear();
    this.terminals.clear();
    this.listeners.clear();
  }

  private async drain(threadId: string): Promise<void> {
    const state = this.snapshot(threadId);
    const item = state.items[0];
    if (this.disposed || this.inFlight.has(threadId) || state.paused || state.waitingTurnId || item?.state !== "queued" || !this.canDispatch(threadId)) return;
    const refusal = this.preflight(threadId);
    if (refusal) { this.set(threadId, { ...state, paused: refusal }); return; }
    this.inFlight.add(threadId);
    this.set(threadId, { ...state, items: state.items.map((value) => value.id === item.id ? { ...value, state: "sending" } : value) });
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const turnId = await Promise.race([
        this.send(threadId, item.text),
        new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error("응답 대기 시간이 지났습니다.")), QUEUED_INPUT_TIMEOUT_MS); }),
      ]);
      if (!turnId) throw new Error("실행한 턴을 확인하지 못했습니다.");
      if (this.disposed) return;
      const current = this.snapshot(threadId);
      const terminal = this.terminals.get(this.key(threadId, turnId));
      this.set(threadId, { ...current,
        items: current.items.filter((value) => value.id !== item.id),
        waitingTurnId: terminal ? null : turnId,
        paused: terminal && terminal !== "completed" ? "턴이 중단되거나 실패해 예약을 멈췄습니다." : current.paused,
      });
    } catch {
      if (!this.disposed) {
        const current = this.snapshot(threadId);
        this.set(threadId, { ...current,
          items: current.items.map((value) => value.id === item.id ? { ...value, state: "unknown" } : value),
          paused: "전송 결과를 확인하지 못했습니다. 대화 기록을 확인하세요. 자동으로 다시 보내지 않습니다.",
        });
      }
    } finally {
      if (timer !== undefined) clearTimeout(timer);
      this.inFlight.delete(threadId);
    }
    // A terminal notification may arrive before the start response. Only continue
    // after that response is correlated, and never on a duplicate terminal alone.
    void this.drain(threadId);
  }

  private validText(text: string): string {
    const value = text.trim();
    if (!value || value.length > MAX_TEXT) throw new Error("예약 입력은 1~32,000자로 작성하세요.");
    return value;
  }
  private key(threadId: string, turnId: string): string { return JSON.stringify([threadId, turnId]); }
  private set(threadId: string, state: NextTurnQueueState): void {
    if (this.disposed) return;
    this.states.set(threadId, state);
    for (const listener of this.listeners) listener();
  }
}
