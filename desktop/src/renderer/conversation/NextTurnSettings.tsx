import { useEffect, useState, useSyncExternalStore } from "react";
import { APPROVAL_CHOICES, SANDBOX_CHOICES, type NextTurnSettingsController, type ThreadPolicy } from "../session/nextTurnSettings";

const APPROVAL_LABELS: Record<string, string> = { untrusted: "신뢰하지 않은 명령 승인", "on-request": "요청 시 승인", never: "승인 요청 없음" };
const SANDBOX_LABELS: Record<string, string> = { readOnly: "읽기 전용", workspaceWrite: "작업 폴더 쓰기", dangerFullAccess: "전체 접근", externalSandbox: "외부 sandbox" };
const CURRENT_SOURCE = { snapshot: "대화 열기 응답", notification: "설정 변경 알림" };

function approvalText(value: ThreadPolicy["approvalPolicy"]): string {
  if (value === null) return "미확인";
  return typeof value === "string" ? `${APPROVAL_LABELS[value] ?? value} (${value})` : "세부 승인 (granular)";
}
function sandboxText(value: ThreadPolicy["sandbox"]): string {
  if (value === null) return "미확인";
  const network = "networkAccess" in value && (value.networkAccess === true || value.networkAccess === "enabled") ? " · 네트워크 허용" : "";
  return `${SANDBOX_LABELS[value.type] ?? value.type} (${value.type})${network}`;
}

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
  const override = state.draft.model !== null || state.draft.effort !== null
    || state.policyDraft.approvalPolicy !== null || state.policyDraft.sandbox !== null;
  const needsConsent = controller.needsPolicyConsent(threadId);
  const reason = controller.validationReason(threadId);
  const run = (action: () => void) => {
    setError(null);
    try { action(); } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
  };
  return <details className="next-turn-settings" data-testid="next-turn-settings">
    <summary><strong>다음 턴 설정</strong> <span className="muted">현재 대화: {state.current?.model ?? "모델 미확인"} · {state.current?.effort ?? "추론 기본값/미확인"}</span>
      <span className="muted" data-testid="next-turn-policy-current">승인: {approvalText(state.currentPolicy.approvalPolicy)} · sandbox: {sandboxText(state.currentPolicy.sandbox)}
        {" "}(출처: {state.currentSource ? CURRENT_SOURCE[state.currentSource] : "미확인"})</span>
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
      <label>승인 정책 <select aria-label="다음 턴 승인 정책" value={state.policyDraft.approvalPolicy ?? ""} disabled={disabled}
        onChange={(event) => run(() => controller.setPolicy(threadId, "approvalPolicy", event.target.value || null))}>
        <option value="">현재 설정 상속</option>
        {APPROVAL_CHOICES.map((value) => <option key={value} value={value}>{APPROVAL_LABELS[value]} ({value})</option>)}
      </select></label>
      <label>sandbox <select aria-label="다음 턴 sandbox 정책" value={state.policyDraft.sandbox ?? ""} disabled={disabled}
        onChange={(event) => run(() => controller.setPolicy(threadId, "sandbox", event.target.value || null))}>
        <option value="">현재 설정 상속</option>
        {SANDBOX_CHOICES.map((value) => <option key={value} value={value}>{SANDBOX_LABELS[value]} ({value})</option>)}
      </select></label>
      <button type="button" className="subtle-button" disabled={disabled || catalog.phase === "loading"} onClick={() => void controller.refresh()}>목록 새로고침</button>
      <button type="button" className="subtle-button" disabled={!override} onClick={() => run(() => controller.reset(threadId))}>현재 설정 상속</button>
    </div>
    <p className="muted">다음 새 턴부터 적용되어 이후 턴에도 이어집니다. 진행 중인 턴에 추가 지시를 보내도 대기 설정은 유지됩니다.</p>
    {catalog.phase === "loading" && <p role="status">지원 모델과 추론 수준을 읽는 중…</p>}
    {catalog.error && <p role="status">목록을 확인하지 못했습니다: {catalog.error} 현재 설정을 상속하면 계속 보낼 수 있습니다.</p>}
    {(reason || error) && <p className="error" role="status">{error ?? reason}
      {needsConsent && <>{" "}<button type="button" className="subtle-button" disabled={disabled}
        onClick={() => run(() => controller.confirmWiderPolicy(threadId))}>넓은 권한 적용 확인</button></>}</p>}
  </details>;
}
