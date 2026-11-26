import type { ModelListParams } from "../../shared/protocol/generated/v2/ModelListParams";

export interface ModelChoice {
  model: string;
  name: string;
  efforts: readonly { value: string; description: string }[];
  defaultEffort: string | null;
}
export interface ModelCatalog {
  phase: "offline" | "idle" | "loading" | "ready" | "failed";
  models: readonly ModelChoice[];
  error: string | null;
}
export interface ThreadModelSettings {
  /** 서버가 확인한 대화 설정이다. 실제 각 응답에서 쓴 모델의 telemetry는 아니다. */
  current: { model: string | null; effort: string | null } | null;
  currentSource: "snapshot" | "notification" | null;
  currentRevision: number;
  draft: { model: string | null; effort: string | null };
  draftRevision: number;
}
export interface PreparedTurnSettings {
  threadId: string;
  epoch: number;
  draftRevision: number;
  currentRevision: number;
  params: { model?: string; effort?: string };
}
const EMPTY: ThreadModelSettings = {
  current: null, currentSource: null, currentRevision: 0, draft: { model: null, effort: null }, draftRevision: 0,
};
export const MODEL_LIST_TIMEOUT_MS = 10_000;
const MAX_PAGES = 10;
const text = (value: unknown): string | null => typeof value === "string" && value.trim() ? value : null;
const object = (value: unknown): Record<string, unknown> | null => value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;

function modelPage(value: unknown): { models: ModelChoice[]; cursor: string | null } {
  const page = object(value);
  if (!page || !Array.isArray(page.data) || page.data.length > 1000 || !(page.nextCursor === null || text(page.nextCursor))) {
    throw new Error("모델 목록의 응답 형식을 확인하지 못했습니다.");
  }
  const models: ModelChoice[] = [];
  for (const entry of page.data) {
    const model = object(entry);
    const id = text(model?.model);
    if (!model || !id || !Array.isArray(model.supportedReasoningEfforts)) throw new Error("모델의 지원 추론 수준을 확인하지 못했습니다.");
    if (model.hidden === true) continue;
    const efforts = model.supportedReasoningEfforts.map((entry: unknown) => {
      const effort = object(entry);
      const value = text(effort?.reasoningEffort);
      if (!value) throw new Error("모델의 지원 추론 수준을 확인하지 못했습니다.");
      return { value, description: text(effort?.description) ?? value };
    });
    const defaultEffort = text(model.defaultReasoningEffort);
    if (efforts.length && !efforts.some((effort) => effort.value === defaultEffort)) {
      throw new Error("모델의 기본 추론 수준이 지원 목록에 없습니다.");
    }
    models.push({ model: id, name: text(model.displayName) ?? id, efforts, defaultEffort: efforts.length ? defaultEffort : null });
  }
  return { models, cursor: page.nextCursor as string | null };
}

/** 한 DesktopSession(한 호스트)의 목록과 대화별 미전송 설정만 메모리에 보관한다. */
export class NextTurnSettingsController {
  private catalog: ModelCatalog = { phase: "offline", models: [], error: null };
  private readonly threads = new Map<string, ThreadModelSettings>();
  private readonly listeners = new Set<() => void>();
  private epoch = 0;
  private loadGeneration = 0;
  private connected = false;

  constructor(private readonly list: (params: ModelListParams) => Promise<unknown>) {}

  catalogSnapshot = (): ModelCatalog => this.catalog;
  snapshot = (threadId: string): ThreadModelSettings => this.threads.get(threadId) ?? EMPTY;
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  connectionChanged(connected: boolean, expectedEpoch?: number): number {
    if (expectedEpoch !== undefined && expectedEpoch !== this.epoch) return this.epoch;
    this.connected = connected;
    this.epoch++;
    this.loadGeneration++;
    this.catalog = { phase: connected ? "idle" : "offline", models: [], error: null };
    for (const [id, state] of this.threads) {
      this.threads.set(id, { ...state, current: null, currentSource: null, currentRevision: state.currentRevision + 1 });
    }
    this.publish();
    return this.epoch;
  }

  async refresh(): Promise<void> {
    if (!this.connected) return;
    const generation = ++this.loadGeneration;
    this.catalog = { ...this.catalog, phase: "loading", error: null };
    this.publish();
    const models = new Map<string, ModelChoice>();
    const cursors = new Set<string>();
    let cursor: string | null = null;
    try {
      for (let page = 0; page < MAX_PAGES; page++) {
        let timer: ReturnType<typeof setTimeout> | undefined;
        const response = await Promise.race([
          this.list({ limit: 100, includeHidden: false, ...(cursor ? { cursor } : {}) }),
          new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error("모델 목록 응답을 기다리는 시간이 지났습니다.")), MODEL_LIST_TIMEOUT_MS); }),
        ]).finally(() => { if (timer !== undefined) clearTimeout(timer); });
        if (generation !== this.loadGeneration || !this.connected) return;
        const next = modelPage(response);
        for (const model of next.models) models.set(model.model, model);
        cursor = next.cursor;
        if (!cursor) {
          this.catalog = { phase: "ready", models: [...models.values()], error: null };
          this.publish();
          return;
        }
        if (cursors.has(cursor)) throw new Error("모델 목록의 다음 페이지가 반복되어 확인을 멈췄습니다.");
        cursors.add(cursor);
      }
      throw new Error("모델 목록이 너무 길어 전체 지원 범위를 확인하지 못했습니다.");
    } catch (error) {
      if (generation !== this.loadGeneration || !this.connected) return;
      this.catalog = { phase: "failed", models: [], error: error instanceof Error ? error.message : "모델 목록을 읽지 못했습니다." };
      this.publish();
    }
  }

  observeSnapshot(threadId: string, model: unknown, effort: unknown, expectedRevision?: number): void {
    this.observeCurrent(threadId, model, effort, "snapshot", expectedRevision);
  }

  observeNotification(threadId: string, model: unknown, effort: unknown): void {
    this.observeCurrent(threadId, model, effort, "notification");
  }

  private observeCurrent(threadId: string, model: unknown, effort: unknown,
    currentSource: "snapshot" | "notification", expectedRevision?: number): void {
    if (!this.connected) return;
    const state = this.snapshot(threadId);
    if (expectedRevision !== undefined && state.currentRevision !== expectedRevision) return;
    this.set(threadId, { ...state, current: { model: text(model), effort: text(effort) }, currentSource, currentRevision: state.currentRevision + 1 });
  }

  setModel(threadId: string, model: string | null): void {
    if (model === null) { this.reset(threadId); return; }
    const choice = this.catalog.phase === "ready" && this.catalog.models.find((entry) => entry.model === model);
    if (!choice) throw new Error("현재 지원 목록에 있는 모델을 선택하세요.");
    const state = this.snapshot(threadId);
    this.set(threadId, { ...state, draft: { model, effort: choice.defaultEffort }, draftRevision: state.draftRevision + 1 });
  }

  setEffort(threadId: string, effort: string | null): void {
    const state = this.snapshot(threadId);
    const model = state.draft.model ?? state.current?.model;
    const choice = this.catalog.phase === "ready" && this.catalog.models.find((entry) => entry.model === model);
    if (effort !== null && (!choice || !choice.efforts.some((entry) => entry.value === effort))) {
      throw new Error("선택한 모델이 지원하는 추론 수준을 선택하세요.");
    }
    if (effort === null && state.draft.model !== null && choice && choice.efforts.length) {
      throw new Error("새 모델의 지원 추론 수준을 선택하세요.");
    }
    this.set(threadId, { ...state, draft: { ...state.draft, effort }, draftRevision: state.draftRevision + 1 });
  }

  reset(threadId: string): void {
    const state = this.snapshot(threadId);
    this.set(threadId, { ...state, draft: { model: null, effort: null }, draftRevision: state.draftRevision + 1 });
  }

  validationReason(threadId: string): string | null {
    const state = this.snapshot(threadId);
    if (state.draft.model === null && state.draft.effort === null) return null;
    if (!this.connected || this.catalog.phase !== "ready") return "모델 목록을 확인한 뒤 다음 턴 설정을 적용하세요. 현재 설정 상속으로 되돌릴 수도 있습니다.";
    const model = state.draft.model ?? state.current?.model;
    const choice = this.catalog.models.find((entry) => entry.model === model);
    if (!choice) return "다음 턴의 모델이 현재 지원 목록에 없습니다. 모델을 다시 선택하거나 현재 설정을 상속하세요.";
    if (state.draft.effort !== null && !choice.efforts.some((entry) => entry.value === state.draft.effort)) return "다음 턴의 추론 수준이 현재 모델의 지원 목록에 없습니다. 다시 선택하세요.";
    if (state.draft.model !== null && choice.efforts.length && state.draft.effort === null) return "새 모델의 지원 추론 수준을 선택하세요.";
    return null;
  }

  prepare(threadId: string): PreparedTurnSettings {
    const reason = this.validationReason(threadId);
    if (reason) throw new Error(reason);
    const state = this.snapshot(threadId);
    return { threadId, epoch: this.epoch, draftRevision: state.draftRevision, currentRevision: state.currentRevision,
      params: { ...(state.draft.model !== null ? { model: state.draft.model } : {}), ...(state.draft.effort !== null ? { effort: state.draft.effort } : {}) } };
  }

  accepted(prepared: PreparedTurnSettings): void {
    if (prepared.epoch !== this.epoch || !Object.keys(prepared.params).length) return;
    const state = this.snapshot(prepared.threadId);
    const invalidateCurrent = state.currentSource !== "notification" || state.currentRevision <= prepared.currentRevision;
    // 성공한 전송의 설정은 이후 턴에 상속된다. 겹친 조회 응답은 적용 전 설정일 수 있으므로,
    // 전송 이후의 실제 설정 알림만 보존하고 다른 조회 토큰도 revision 증가로 폐기한다.
    this.set(prepared.threadId, { ...state,
      current: invalidateCurrent ? null : state.current,
      currentSource: invalidateCurrent ? null : state.currentSource,
      currentRevision: state.currentRevision + (invalidateCurrent ? 1 : 0),
      draft: state.draftRevision === prepared.draftRevision ? { model: null, effort: null } : state.draft,
    });
  }

  dispose(): void {
    this.connectionChanged(false);
    this.threads.clear();
    this.listeners.clear();
  }
  private set(threadId: string, state: ThreadModelSettings): void { this.threads.set(threadId, state); this.publish(); }
  private publish(): void { for (const listener of this.listeners) listener(); }
}
