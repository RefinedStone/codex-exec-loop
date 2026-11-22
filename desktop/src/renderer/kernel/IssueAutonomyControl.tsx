import { ISSUE_AUTONOMY_LABEL, type IssueAutonomyLevel, type IssueAutonomyView } from "./issueInteractions";

export function IssueAutonomyControl({ autonomy, busy, connected, onChange }: {
  autonomy: IssueAutonomyView | undefined;
  busy: boolean;
  connected: boolean;
  onChange(level: IssueAutonomyLevel): Promise<void>;
}) {
  if (!autonomy || autonomy.status !== "ready") {
    return <p className="issues-autonomy-unavailable muted" role="status" data-testid="issues-autonomy-unavailable">{autonomy?.reason ?? "자율 수준을 확인하고 있습니다."}</p>;
  }
  const value = autonomy.requested_level ?? autonomy.effective_level;
  const offered = autonomy.choices.some((choice) => choice.level === value);
  const disabled = !connected || busy || autonomy.pending;
  return (
    <section className="issues-autonomy" aria-label="이슈 자율 수준" data-testid="issues-autonomy">
      <label className="issues-mode-label">
        <span>자율 수준</span>
        <select value={value} disabled={disabled} aria-label="이슈 자율 수준" data-testid="issues-autonomy-level" onChange={(event) => {
          const choice = autonomy.choices.find((item) => item.level === event.target.value);
          if (choice?.enabled) void onChange(choice.level);
        }}>
          {!offered && <option value={value} disabled>{ISSUE_AUTONOMY_LABEL[value] ?? value} · 정책 설정</option>}
          {autonomy.choices.map((choice) => <option key={choice.level} value={choice.level} disabled={!choice.enabled}>{ISSUE_AUTONOMY_LABEL[choice.level]}{choice.enabled ? "" : " · 선택 불가"}</option>)}
        </select>
      </label>
      <p className="issues-autonomy-summary" data-testid="issues-autonomy-effective">
        현재 적용 <strong>{ISSUE_AUTONOMY_LABEL[autonomy.effective_level] ?? autonomy.effective_level}</strong>
        <span>상한 {ISSUE_AUTONOMY_LABEL[autonomy.cap] ?? autonomy.cap}</span>
      </p>
      {autonomy.requested_level === null && <p className="issues-autonomy-help">현재 정책의 자율 수준을 사용하고 있습니다.</p>}
      {autonomy.pending && <p className="issues-autonomy-help" role="status">실행 허가를 정리하며 변경을 적용하고 있습니다.</p>}
      {autonomy.choices.filter((choice) => !choice.enabled && choice.reason).map((choice) => <p key={choice.level} className="issues-autonomy-help" data-testid="issues-autonomy-reason">{ISSUE_AUTONOMY_LABEL[choice.level]}: {choice.reason}</p>)}
      <p className="issues-autonomy-help">수집 모드와 별개입니다. 자율 수준을 높이면 시스템 확인 창을 엽니다.</p>
    </section>
  );
}
