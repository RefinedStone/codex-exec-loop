import type { TaskAcceptanceChannel, TaskAcceptanceCriterion } from "./protocol";
import "./taskCriteria.css";
import { isTaskVerificationRecipe } from "./taskVerificationRecipe";

const CHANNEL_LABEL: Record<TaskAcceptanceChannel, string> = {
  command: "명령", data: "데이터", http: "HTTP", screen: "화면",
};

export interface TaskCriteriaEditorProps {
  value: TaskAcceptanceCriterion[];
  onChange(next: TaskAcceptanceCriterion[]): void;
  disabled?: boolean;
  idPrefix?: string;
}

/** 기준 내용은 사용자 입력이다. 검증 채널을 고른 뒤에만 새 기준을 만든다. */
export function TaskCriteriaEditor({ value, onChange, disabled = false, idPrefix = "task-criteria" }: TaskCriteriaEditorProps) {
  const update = (index: number, patch: Partial<TaskAcceptanceCriterion>) =>
    onChange(value.map((criterion, at) => at === index ? { ...criterion, ...patch } : criterion));
  return (
    <fieldset className="task-criteria" disabled={disabled}>
      <legend>완료 기준</legend>
      {value.length === 0 && <p className="muted">확인할 결과와 검증 채널을 추가하세요.</p>}
      {value.map((criterion, index) => (
        <div className="task-criterion" key={criterion.id} data-testid="task-criterion">
          <label htmlFor={idPrefix + "-scenario-" + index}>기준 {index + 1}</label>
          <textarea id={idPrefix + "-scenario-" + index} value={criterion.scenario} rows={2}
            onChange={(event) => update(index, { scenario: event.target.value })} required />
          <div className="task-criterion-options">
            <label>검증 채널
              <select value={criterion.channel} onChange={(event) => update(index, {
                channel: event.target.value as TaskAcceptanceChannel,
                verification_recipe: null,
              })}>
                {Object.entries(CHANNEL_LABEL).map(([channel, label]) => <option value={channel} key={channel}>{label}</option>)}
              </select>
            </label>
            <label><input type="checkbox" checked={criterion.essential}
              onChange={(event) => update(index, { essential: event.target.checked })} />필수</label>
            <button type="button" onClick={() => onChange(value.filter((_, at) => at !== index))}>기준 삭제</button>
          </div>
          {criterion.channel === "command" && <label>호스트 검증
            <select value={criterion.verification_recipe ?? ""} onChange={(event) => update(index, {
              verification_recipe: isTaskVerificationRecipe(event.target.value) ? event.target.value : null,
            })}>
              <option value="">지정하지 않음</option>
              <option value="git_diff_check_v1">Git diff 공백·충돌 표시 검사</option>
              <option value="repo_git_grep_v1">저장소 선언 문자열 검사 (매 실행 확인)</option>
            </select>
          </label>}
          <small className="muted">기준 ID: {criterion.id}</small>
        </div>
      ))}
      <label>완료 기준 추가
        <select value="" aria-label="완료 기준 추가" onChange={(event) => {
          const channel = event.target.value as TaskAcceptanceChannel;
          if (!Object.hasOwn(CHANNEL_LABEL, channel)) return;
          onChange([...value, { id: "criterion-" + crypto.randomUUID(), scenario: "", channel, essential: true }]);
        }}>
          <option value="">검증 채널 선택…</option>
          {Object.entries(CHANNEL_LABEL).map(([channel, label]) => <option value={channel} key={channel}>{label}</option>)}
        </select>
      </label>
    </fieldset>
  );
}
