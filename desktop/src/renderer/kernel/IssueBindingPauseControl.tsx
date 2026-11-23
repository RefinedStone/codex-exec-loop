import type { IssueBindingPauseState } from "./protocol";
import { bindingPauseControlAction, bindingPauseControlLabel } from "./issueBindingPause";

export function IssueBindingPauseControl({ state, connected, busy, onChange }: {
  state: IssueBindingPauseState | null | undefined;
  connected: boolean;
  busy: boolean;
  onChange?: (paused: boolean) => Promise<void>;
}) {
  const current = state?.status === "ready" ? state : null;
  const pending = current?.pending;
  const label = bindingPauseControlLabel(state);
  const action = bindingPauseControlAction(state, connected, busy, onChange != null);
  return <section className="issues-collection-status" aria-label="이 연결의 이슈 자동화 일시정지" data-testid="issues-binding-pause">
    <strong role="status">{label}</strong>
    <p>{pending?.paused ? "새 자동 작업을 막았습니다. 이미 시작한 요청의 처리가 정리되면 일시정지가 완료됩니다."
      : pending ? "현재 정책과 권한을 다시 확인하고 있습니다."
        : current?.paused ? "진행 기록과 사람에게 대기 중인 질문은 보존됩니다. 재개할 때 현재 기준으로 다시 확인합니다."
          : current ? "이 연결의 자동 작업을 일시정지할 수 있습니다. 수집 설정과 선택한 자율 수준은 유지됩니다."
            : "현재 상태를 읽은 뒤 일시정지하거나 재개할 수 있습니다."}</p>
    <button type="button" data-testid="issues-binding-pause-toggle"
      disabled={action.disabled}
      onClick={() => { if (current && onChange) void onChange(!current.paused); }}>
      {action.label}
    </button>
  </section>;
}
