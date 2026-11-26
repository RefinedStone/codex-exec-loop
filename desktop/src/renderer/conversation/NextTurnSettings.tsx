import { useEffect, useState, useSyncExternalStore } from "react";
import type { NextTurnSettingsController } from "../session/nextTurnSettings";

export function NextTurnSettings({ controller, threadId, disabled }: {
  controller: NextTurnSettingsController; threadId: string; disabled: boolean;
}) {
  const catalog = useSyncExternalStore(controller.subscribe, controller.catalogSnapshot);
  const state = useSyncExternalStore(controller.subscribe, () => controller.snapshot(threadId));
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!disabled && catalog.phase === "idle") void controller.refresh();
  }, [controller, catalog.phase, disabled]);
  const choice = catalog.models.find((model) => model.model === (state.draft.model ?? state.current?.model));
  const override = state.draft.model !== null || state.draft.effort !== null;
  const reason = controller.validationReason(threadId);
  const run = (action: () => void) => {
    setError(null);
    try { action(); } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
  };
  return <details className="next-turn-settings" data-testid="next-turn-settings">
    <summary><strong>다음 턴 설정</strong> <span className="muted">현재 대화: {state.current?.model ?? "모델 미확인"} · {state.current?.effort ?? "추론 기본값/미확인"}</span>
      {override && <span className="chip">변경 대기</span>}</summary>
    <div className="next-turn-settings-fields">
      <label>모델 <select aria-label="다음 턴 모델" value={state.draft.model ?? ""} disabled={disabled || catalog.phase !== "ready"}
        onChange={(event) => run(() => controller.setModel(threadId, event.target.value || null))}>
        <option value="">현재 설정 상속</option>
        {state.draft.model && !catalog.models.some((model) => model.model === state.draft.model) && <option value={state.draft.model} disabled>{state.draft.model} (지원 미확인)</option>}
        {catalog.models.map((model) => <option key={model.model} value={model.model}>{model.name}</option>)}
      </select></label>
      <label>추론 수준 <select aria-label="다음 턴 추론 수준" value={state.draft.effort ?? ""}
        disabled={disabled || catalog.phase !== "ready" || !choice?.efforts.length}
        onChange={(event) => run(() => controller.setEffort(threadId, event.target.value || null))}>
        {state.draft.model === null && <option value="">현재 설정 상속</option>}
        {!choice?.efforts.length && state.draft.model !== null && <option value="">지원 설정 없음</option>}
        {state.draft.effort && !choice?.efforts.some((effort) => effort.value === state.draft.effort) && <option value={state.draft.effort} disabled>{state.draft.effort} (지원 미확인)</option>}
        {choice?.efforts.map((effort) => <option key={effort.value} value={effort.value}>{effort.value} · {effort.description}</option>)}
      </select></label>
      <button type="button" className="subtle-button" disabled={disabled || catalog.phase === "loading"} onClick={() => void controller.refresh()}>목록 새로고침</button>
      <button type="button" className="subtle-button" disabled={!override} onClick={() => run(() => controller.reset(threadId))}>현재 설정 상속</button>
    </div>
    <p className="muted">다음 새 턴부터 적용되어 이후 턴에도 이어집니다. 진행 중인 턴에 추가 지시를 보내도 대기 설정은 유지됩니다.</p>
    {catalog.phase === "loading" && <p role="status">지원 모델과 추론 수준을 읽는 중…</p>}
    {catalog.error && <p role="status">목록을 확인하지 못했습니다: {catalog.error} 현재 설정을 상속하면 계속 보낼 수 있습니다.</p>}
    {(reason || error) && <p className="error" role="status">{error ?? reason}</p>}
  </details>;
}
