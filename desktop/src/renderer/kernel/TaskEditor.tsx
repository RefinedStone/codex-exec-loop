import { useEffect, useRef, useState } from "react";
import { TaskCriteriaEditor } from "./TaskCriteriaEditor";
import { TaskResultView } from "./TaskResultView";
import { taskCriteriaError, type TaskAuthoringController, type TaskListAnswer, type TaskSaveOutcome } from "./taskAuthoring";
import type { TaskFields } from "./protocol";
import "./taskEditor.css";

export interface TaskDraftSource {
  text: string;
  threadId: string;
  turnIds: string[];
}

function draftFields(source?: TaskDraftSource): TaskFields {
  return {
    title: source?.text.trim().split("\n")[0]?.slice(0, 120) ?? "",
    description: source?.text ?? "",
    acceptance: [],
    status: "awaiting_user",
  };
}

const STATUS_LABEL = {
  ready: "대기", blocked: "막힘", in_progress: "진행 중", done: "완료",
  cancelled: "취소", awaiting_user: "사용자 응답 대기", proposed: "제안",
};

export function TaskEditor({ workspace, source, controller, onClose, onSaved }: {
  workspace: string;
  source?: TaskDraftSource;
  controller: TaskAuthoringController;
  onClose(): void;
  onSaved(): void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const alive = useRef(true);
  const saving = useRef(false);
  const [snapshot, setSnapshot] = useState<TaskListAnswer | null>(null);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [lookupId, setLookupId] = useState("");
  const [fields, setFields] = useState<TaskFields>(() => draftFields(source));
  const [busy, setBusy] = useState(false);
  const [reading, setReading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [uncertain, setUncertain] = useState(() => controller.pending(workspace)?.kind === "uncertain");
  const selected = snapshot?.tasks.find((task) => task.task_id === taskId);
  const locked = fields.status === "in_progress";
  const disabled = busy || reading || uncertain || locked || (!!taskId && !selected);
  const validation = taskCriteriaError(fields);

  useEffect(() => {
    alive.current = true;
    dialog.current?.showModal();
    void controller.read(workspace).then((answer) => {
      if (!alive.current) return;
      setSnapshot(answer);
    }).catch((failure: unknown) => {
      if (alive.current) setError(failure instanceof Error ? failure.message : String(failure));
    }).finally(() => { if (alive.current) setReading(false); });
    return () => { alive.current = false; };
  }, [controller, workspace]);

  async function readTask(id: string | null): Promise<void> {
    setReading(true);
    try {
      const answer = await controller.read(workspace);
      if (!alive.current) return;
      // 편집 대상 ID는 조회가 성공한 뒤에만 snapshot revision·필드와 함께 바꾼다.
      setSnapshot(answer);
      if (id) {
        setTaskId(id);
        const task = answer.tasks.find((candidate) => candidate.task_id === id);
        if (!task) throw new Error("이 작업을 현재 원본에서 찾을 수 없습니다. 작업 ID: " + id);
        setFields(structuredClone(task.fields));
      }
      setError(null);
    } catch (failure) {
      if (alive.current) setError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      if (alive.current) setReading(false);
    }
  }

  async function showOutcome(outcome: TaskSaveOutcome): Promise<void> {
    if (!alive.current) return;
    setUncertain(outcome.kind === "uncertain");
    if (outcome.kind === "saved") {
      setTaskId(outcome.taskId);
      setNotice("저장했습니다. 작업 ID: " + outcome.taskId);
      setError(null);
      onSaved();
      await readTask(outcome.taskId);
    } else {
      setError(outcome.message);
    }
  }

  async function save(recover = false): Promise<void> {
    if (saving.current) return;
    saving.current = true;
    setBusy(true);
    setNotice(null);
    setError(null);
    try {
      const outcome = recover ? await controller.recover(workspace)
        : await controller.save(workspace, taskId, fields, snapshot?.planning_revision ?? null);
      await showOutcome(outcome);
    } catch (failure) {
      if (alive.current) {
        setUncertain(controller.pending(workspace)?.kind === "uncertain");
        setError(failure instanceof Error ? failure.message : String(failure));
      }
    } finally {
      saving.current = false;
      if (alive.current) setBusy(false);
    }
  }

  return (
    <dialog ref={dialog} className="task-editor-dialog" aria-labelledby="task-editor-title" data-testid="task-editor"
      onCancel={(event) => { event.preventDefault(); onClose(); }}>
      <header className="task-editor-header">
        <h2 id="task-editor-title">{taskId ? "작업 편집" : "작업 초안"}</h2>
        <button type="button" onClick={onClose} aria-label="작업 편집 닫기">닫기</button>
      </header>
      <p className="muted task-editor-workspace">{workspace}</p>
      <label>기존 작업 열기
        <select value={taskId ?? ""} disabled={busy || reading || uncertain} data-testid="task-editor-select"
          onChange={(event) => {
            const id = event.target.value || null;
            if (!id) controller.beginDraft(workspace);
            setTaskId(id);
            const task = snapshot?.tasks.find((candidate) => candidate.task_id === id);
            setFields(task ? structuredClone(task.fields) : draftFields(source));
            setError(null); setNotice(null);
          }}>
          <option value="">새 작업 초안</option>
          {snapshot?.tasks.map((task) => <option key={task.task_id} value={task.task_id}>
            {task.fields.title} · {STATUS_LABEL[task.fields.status ?? "awaiting_user"]} · {task.task_id}
          </option>)}
        </select>
      </label>
      <form className="task-editor-id-lookup" onSubmit={(event) => {
        event.preventDefault();
        if (!lookupId.trim() || busy || reading || uncertain) return;
        void readTask(lookupId.trim());
      }}>
        <label htmlFor="task-editor-lookup">작업 ID로 열기</label>
        <input id="task-editor-lookup" value={lookupId} onChange={(event) => setLookupId(event.target.value)} disabled={busy || reading || uncertain} />
        <button type="submit" disabled={!lookupId.trim() || busy || reading || uncertain}>ID로 열기</button>
      </form>
      {source && !taskId && <details className="task-editor-source">
        <summary>선택한 대화 내용 확인</summary>
        <p className="muted">스레드 {source.threadId} · 턴 {source.turnIds.join(", ")}</p>
        <blockquote>{source.text}</blockquote>
      </details>}
      {taskId && <p className="muted" data-testid="task-editor-identity">
        {taskId} · 기준 revision {selected?.acceptance_revision ?? "확인 중"}
      </p>}
      {locked && <p className="warning">진행 중인 작업입니다. 실행이 끝난 뒤 최신 작업을 다시 읽고 수정하세요.</p>}
      <form onSubmit={(event) => { event.preventDefault(); void save(); }}>
        <fieldset disabled={disabled} className="task-editor-fields">
          <label htmlFor="task-editor-task-title">작업 제목</label>
          <input id="task-editor-task-title" value={fields.title} required
            onChange={(event) => setFields({ ...fields, title: event.target.value })} />
          <label htmlFor="task-editor-description">작업 설명</label>
          <textarea id="task-editor-description" value={fields.description ?? ""} rows={5}
            onChange={(event) => setFields({ ...fields, description: event.target.value })} />
          <TaskCriteriaEditor value={fields.acceptance ?? []} onChange={(acceptance) => setFields({ ...fields, acceptance })} disabled={disabled} />
        </fieldset>
        {!taskId && <p className="muted">확정하면 사용자 응답 대기 상태로 저장합니다. 작업 실행은 별도로 결정할 수 있습니다.</p>}
        {uncertain && <p className="warning">이 저장소의 이전 저장 요청을 확인해야 합니다. 아래 버튼은 같은 요청의 결과만 다시 확인합니다.</p>}
        {error && <p role="alert" className="error" data-testid="task-editor-error">{error}</p>}
        {notice && <p role="status" data-testid="task-editor-notice">{notice}</p>}
        <footer className="task-editor-actions">
          <button type="button" disabled={busy || reading} onClick={() => { controller.beginDraft(workspace); void readTask(taskId); }}>
            {reading ? "읽는 중…" : "최신 작업 다시 읽기"}
          </button>
          {uncertain ? <button type="button" disabled={busy || reading} onClick={() => void save(true)}
            data-testid="task-editor-recover">같은 요청으로 결과 확인</button>
            : <button type="submit" className="button" disabled={disabled || !snapshot || !!validation}
              data-testid="task-editor-save">{busy ? "저장 중…" : taskId ? "변경 확정" : "작업 확정"}</button>}
        </footer>
      </form>
      {taskId && <TaskResultView workspace={workspace} taskId={taskId} revision={snapshot?.planning_revision} controller={controller}
        onOpenTask={busy || reading || uncertain ? undefined : (id) => void readTask(id)} />}
    </dialog>
  );
}
