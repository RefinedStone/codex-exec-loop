// Jira reads and credential ownership live in the Gateway. This view never receives a token.
import { type FormEvent, useEffect, useRef, useState } from "react";
import type { JiraBridge, JiraDeployment, JiraIssue, JiraSummary } from "../../shared/bridge";
import type { ConnectionConfiguration } from "../../shared/providerCredentials";
import { useAction } from "./useAction";
import { providerConnectionKey, useProviderConnection } from "./useProviderConnection";

type ListState = { phase: "loading" } | { phase: "loaded"; summary: JiraSummary } | { phase: "failed"; message: string };
const CATEGORY_CLASS: Record<string, string> = { new: "checks-expected", indeterminate: "checks-pending", done: "checks-success" };
function message(error: unknown): string { return error instanceof Error ? error.message : "Jira를 읽을 수 없습니다."; }

function ConnectForm({ initial, busy, supported, connect }: {
  initial?: ConnectionConfiguration; busy: boolean; supported: boolean; connect(configuration: ConnectionConfiguration): void;
}) {
  const [deployment, setDeployment] = useState<JiraDeployment>(initial?.deployment ?? "cloud");
  const [siteUrl, setSiteUrl] = useState(initial?.siteUrl ?? "");
  const [email, setEmail] = useState(initial?.email ?? "");
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!busy && supported) connect({ siteUrl: siteUrl.trim(), deployment, ...(deployment === "cloud" ? { email: email.trim() } : {}) });
  };
  return (
    <form className="work-section work-form" onSubmit={submit} data-testid="jira-connect">
      <p className="muted">토큰은 별도 시스템 창에서 입력하고 OS 키 저장소에 보관합니다. 연결 전에 회사의 데이터 정책을 확인하세요.</p>
      <label>종류
        <select value={deployment} onChange={event => setDeployment(event.target.value as JiraDeployment)} disabled={busy}>
          <option value="cloud">Cloud (이메일 + API 토큰)</option>
          <option value="server">Server / Data Center (PAT)</option>
        </select>
      </label>
      <label>주소
        <input value={siteUrl} onChange={event => setSiteUrl(event.target.value)} placeholder="https://회사.atlassian.net" required disabled={busy} data-testid="jira-site" />
      </label>
      {deployment === "cloud" && <label>이메일
        <input type="email" value={email} onChange={event => setEmail(event.target.value)} autoComplete="username" required disabled={busy} data-testid="jira-email" />
      </label>}
      <button type="submit" disabled={busy || !supported} data-testid="jira-connect-submit">{busy ? "확인 중…" : "시스템 창에서 연결"}</button>
      {!supported && <p className="muted" role="status">이 환경에서는 새 자격 증명을 입력할 수 없습니다.</p>}
    </form>
  );
}

function IssueRow({ issue, jira, onEdit, onHandOff }: { issue: JiraIssue; jira: JiraBridge; onEdit?: (issue:JiraIssue) => Promise<void>; onHandOff(issue: JiraIssue): Promise<void> }) {
  const { busy, error, run } = useAction();
  return (
    <li className="work-item" data-testid="jira-issue">
      <button type="button" className="work-link" onClick={() => void run(() => jira.openExternal(issue.url))} title={issue.url}>{issue.key} {issue.summary}</button>
      <div className="work-meta">
        {issue.status && <span className={"work-badge " + (CATEGORY_CLASS[issue.statusCategory ?? ""] ?? "")}>{issue.status}</span>}
        {issue.issueType && <span className="muted">{issue.issueType}</span>}
        {issue.priority && <span className="muted">· {issue.priority}</span>}
        <button type="button" onClick={() => void run(() => onHandOff(issue))} disabled={busy} data-testid="jira-hand-off">{busy ? "읽는 중…" : "스레드로 넘기기"}</button>
        {onEdit && <button type="button" disabled={busy} onClick={() => void run(() => onEdit(issue))}>마스킹 초안 열기</button>}
      </div>
      {error && <div className="error">{error}</div>}
    </li>
  );
}

export function JiraPanel({ jira, onEdit, onHandOff }: { jira: JiraBridge; onEdit?: (issue:JiraIssue) => Promise<void>; onHandOff(issue: JiraIssue): Promise<void> }) {
  const connection = useProviderConnection(jira);
  const { state, busy, error, run } = connection;
  const active = state?.activeConnection;
  const scope = providerConnectionKey(state);
  const [editing, setEditing] = useState(false);
  const [list, setList] = useState<ListState>({ phase: "loading" });
  const request = useRef(0);
  const alive = useRef(false);
  const refresh = () => {
    const sequence = ++request.current;
    setList({ phase: "loading" });
    void jira.summary().then(
      summary => { if (alive.current && sequence === request.current) setList({ phase: "loaded", summary }); },
      cause => { if (alive.current && sequence === request.current) setList({ phase: "failed", message: message(cause) }); },
    );
  };
  useEffect(() => {
    alive.current = true;
    request.current++;
    setList({ phase: "loading" });
    if (scope) refresh();
    return () => { alive.current = false; request.current++; };
  }, [jira, scope]);

  return (
    <aside className="work-panel" data-testid="jira-panel" aria-label="Jira">
      <div className="work-header">
        <strong>Jira</strong>
        {active && <span className="muted">{active.metadata.configuration.email ?? active.metadata.accountName}</span>}
        {active && <span className="work-actions">
          <button type="button" onClick={refresh} disabled={list.phase === "loading"} data-testid="jira-refresh">{list.phase === "loading" ? "읽는 중…" : "새로 고침"}</button>
          <button type="button" onClick={() => setEditing(value => !value)} disabled={busy || !connection.canConnect}>연결 변경</button>
          <button type="button" onClick={() => void run(() => jira.disconnect())} data-testid="jira-disconnect">연결 해제</button>
        </span>}
      </div>
      {!state && !error && <p className="muted work-section" role="status">연결 상태 확인 중…</p>}
      {state && (!active || editing) && <ConnectForm key={scope || "new"} initial={active?.metadata.configuration} busy={busy} supported={connection.canConnect}
        connect={configuration => { void run(() => jira.connect(configuration)).then(done => { if (done) setEditing(false); }); }} />}
      {connection.canMigrate && <div className="work-section">
        <button type="button" disabled={busy} onClick={() => void run(() => jira.migrateLegacy())} data-testid="jira-migrate">
          {connection.legacy.kind === "pending" ? "기존 연결 이관 확인" : "기존 연결 옮기기"}
        </button>
        <p className="muted">저장한 토큰을 다시 입력하지 않고 옮깁니다.</p>
      </div>}
      {connection.canCleanLegacy && <div className="work-section" role="status">
        <p className="muted">연결은 해제되어 있습니다. 저장된 기존 연결을 정리할 수 있습니다.</p>
        <button type="button" disabled={busy} onClick={() => void run(() => jira.disconnect())} data-testid="jira-disconnect-legacy">남은 기존 연결 삭제</button>
      </div>}
      {(busy || connection.pending) && <div className="work-section" role="status">
        연결 작업을 확인하고 있습니다.
        <button type="button" onClick={() => void run(() => jira.cancel())} data-testid="jira-cancel">작업 취소</button>
      </div>}
      {state?.operation?.cleanupPending && <p className="muted work-section" role="status">{state.operation.kind === "disconnect" ? "연결은 해제했습니다. 저장소에서 자격 증명을 지우는 작업이 남아 있습니다." : "현재 연결은 유지됩니다. 이전 자격 증명 정리가 남아 있습니다."}</p>}
      {error && <div className="error work-section" role="alert" data-testid="jira-error">{error}</div>}
      {(error || connection.pending || state?.operation?.cleanupPending) && <div className="work-section">
        <button type="button" onClick={() => void run(() => jira.retry())} data-testid="jira-retry">{state?.operation?.kind === "disconnect" && state.operation.cleanupPending ? "삭제 다시 시도" : "상태 확인"}</button>
      </div>}
      {active && list.phase === "failed" && <div className="error work-section" data-testid="jira-error">{list.message}</div>}
      {active && list.phase === "loaded" && <div className="work-scroll"><section className="work-section">
        <h3>나에게 배정된 티켓 ({list.summary.issues.length})</h3>
        {list.summary.issues.length === 0 && <p className="muted">끝나지 않은 티켓이 없습니다.</p>}
        <ul>{list.summary.issues.map(issue => <IssueRow key={issue.key} issue={issue} jira={jira} onEdit={onEdit} onHandOff={onHandOff} />)}</ul>
      </section></div>}
    </aside>
  );
}
