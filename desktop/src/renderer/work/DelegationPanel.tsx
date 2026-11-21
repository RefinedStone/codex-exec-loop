import { type FormEvent, useEffect, useRef, useState } from "react";

import { JsonRpcError } from "../rpc/jsonRpcClient";
import { useAction } from "./useAction";

export interface DelegationSummary {
  id: string;
  label: string;
  status: string;
  backend: string;
}

export interface DelegationDetail extends DelegationSummary {
  prompt?: string;
  canCancel: boolean;
  result?: string;
  reason?: string;
  attempts?: readonly string[];
  evidence?: readonly { source: string; observedAt: string; text?: string }[];
}

export interface DelegationActions {
  list(): Promise<readonly DelegationSummary[]>;
  detail(id: string): Promise<DelegationDetail>;
  submit(text: string, key: string): Promise<string>;
  cancel(id: string): Promise<void>;
  openLogin(): Promise<void>;
}

type ReadState<T> =
  | { phase: "loading" }
  | { phase: "ready"; value: T }
  | { phase: "failed"; message: string; unsupported: boolean };

function readError(cause: unknown): Extract<ReadState<never>, { phase: "failed" }> {
  return {
    phase: "failed",
    message: cause instanceof Error ? cause.message : String(cause),
    unsupported: cause instanceof JsonRpcError && cause.code === -32601,
  };
}

function DelegationDetailView({
  actions,
  id,
  revision,
  refresh,
}: {
  actions: DelegationActions;
  id: string;
  revision: number;
  refresh(): void;
}) {
  const [state, setState] = useState<ReadState<DelegationDetail>>({ phase: "loading" });
  const cancelAction = useAction();

  useEffect(() => {
    let active = true;
    setState({ phase: "loading" });
    actions.detail(id).then(
      (value) => { if (active) setState({ phase: "ready", value }); },
      (cause: unknown) => { if (active) setState(readError(cause)); },
    );
    return () => { active = false; };
  }, [actions, id, revision]);

  if (state.phase === "loading") return <p className="muted work-section" role="status">상세 읽는 중…</p>;
  if (state.phase === "failed") return <p className="error work-section" role="alert">{state.message}</p>;
  const task = state.value;
  return (
    <section className="work-section" data-testid="delegation-detail">
      <h3>위임 상세</h3>
      <strong>{task.label}</strong>
      <div className="work-meta muted">{task.status} · {task.backend}</div>
      {task.prompt != null && (
        <>
          <h3>지시</h3>
          <p className="delegation-text">{task.prompt}</p>
        </>
      )}
      {task.reason && <p className="muted">{task.reason}</p>}
      {!!task.attempts?.length && (
        <>
          <h3>시도</h3>
          <ul>{task.attempts.map((attempt, index) => <li key={index}>{attempt}</li>)}</ul>
        </>
      )}
      {!!task.evidence?.length && (
        <>
          <h3>진행 근거</h3>
          <ul>{task.evidence.map((item, index) => (
            <li key={index} className="work-item">
              <div className="work-meta muted">{item.source} · <time dateTime={item.observedAt}>{item.observedAt}</time></div>
              {item.text && <p className="delegation-text">{item.text}</p>}
            </li>
          ))}</ul>
        </>
      )}
      {task.result != null && (
        <>
          <h3>결과</h3>
          <p className="delegation-text" data-testid="delegation-result">{task.result}</p>
        </>
      )}
      {task.canCancel && (
        <button type="button" disabled={cancelAction.busy} onClick={() => void cancelAction.run(async () => {
          await actions.cancel(id);
          refresh();
        })}>{cancelAction.busy ? "취소 중…" : "위임 취소"}</button>
      )}
      {cancelAction.error && <p className="error" role="alert">{cancelAction.error}</p>}
    </section>
  );
}

export function DelegationPanel({ actions, ready, message }: { actions: DelegationActions; ready: boolean; message: string }) {
  const [list, setList] = useState<ReadState<readonly DelegationSummary[]>>({ phase: "loading" });
  const [draft, setDraft] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const submission = useRef<{ text: string; key: string } | null>(null);
  const submitAction = useAction();
  const loginAction = useAction();
  const refresh = () => setRevision((value) => value + 1);

  useEffect(() => {
    if (!ready) return;
    let active = true;
    setList({ phase: "loading" });
    actions.list().then(
      (value) => { if (active) setList({ phase: "ready", value }); },
      (cause: unknown) => { if (active) setList(readError(cause)); },
    );
    return () => { active = false; };
  }, [actions, ready, revision]);

  function submit(event: FormEvent) {
    event.preventDefault();
    const text = draft.trim();
    if (!text || !ready || list.phase !== "ready") return;
    void submitAction.run(async () => {
      if (submission.current?.text !== text) submission.current = { text, key: crypto.randomUUID() };
      const id = await actions.submit(text, submission.current.key);
      submission.current = null;
      setDraft("");
      setSelected(id);
      refresh();
    });
  }

  const unsupported = list.phase === "failed" && list.unsupported;
  return (
    <section className="work-panel" aria-label="ChatGPT 위임" data-testid="delegation-panel">
      <div className="work-header">
        <strong>ChatGPT 위임</strong>
        <div className="work-actions">
          <button type="button" disabled={unsupported || loginAction.busy} onClick={() => void loginAction.run(actions.openLogin)}>
            {loginAction.busy ? "여는 중…" : "ChatGPT 로그인 열기"}
          </button>
          <button type="button" disabled={!ready || list.phase === "loading"} onClick={refresh}>새로 고침</button>
        </div>
      </div>
      <div className="work-scroll">
        {loginAction.error && <p className="error work-section" role="alert">{loginAction.error}</p>}
        {!ready && <p className="muted work-section" role="status">{message}</p>}
        <form className="work-section work-form delegation-form" onSubmit={submit}>
          <label>
            위임 지시
            <textarea value={draft} rows={4} disabled={submitAction.busy || unsupported} onChange={(event) => setDraft(event.target.value)} />
          </label>
          <button type="submit" disabled={!ready || !draft.trim() || submitAction.busy || list.phase !== "ready"}>
            {submitAction.busy ? "접수 중…" : "텍스트 위임"}
          </button>
          {submitAction.error && <p className="error" role="alert">{submitAction.error}</p>}
        </form>
        {ready && <section className="work-section">
          <h3>위임 목록</h3>
          {list.phase === "loading" && <p className="muted" role="status">목록 읽는 중…</p>}
          {list.phase === "failed" && (
            <p className="error" role="alert">{unsupported ? "이 Gateway 는 위임을 지원하지 않습니다." : list.message}</p>
          )}
          {list.phase === "ready" && (list.value.length === 0
            ? <p className="muted">위임 작업이 없습니다.</p>
            : (
              <ul>
                {list.value.map((task) => (
                  <li key={task.id} className="work-item">
                    <button type="button" className="work-link delegation-text" aria-pressed={selected === task.id} onClick={() => setSelected(task.id)}>
                      {task.label}
                    </button>
                    <div className="work-meta muted">{task.status} · {task.backend}</div>
                  </li>
                ))}
              </ul>
            ))}
        </section>}
        {ready && selected && <DelegationDetailView key={selected} actions={actions} id={selected} revision={revision} refresh={refresh} />}
      </div>
    </section>
  );
}
