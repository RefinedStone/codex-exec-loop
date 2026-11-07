// Jira 업무 패널이다 (R4). 연결 전에는 연결 화면을, 연결 후에는 나에게 배정된 끝나지 않은 티켓을 보여 준다.
// 열 때와 "새로 고침" 때만 읽는다. "스레드로 넘기기" 는 티켓 내용으로 새 스레드를 만들고 입력창만 채운다.
import { type FormEvent, useEffect, useState } from "react";

import type { JiraBridge, JiraDeployment, JiraIssue, JiraStatus, JiraSummary } from "../../shared/bridge";

type ListState = { phase: "loading" } | { phase: "loaded"; summary: JiraSummary } | { phase: "failed"; message: string };

const CATEGORY_CLASS: Record<string, string> = { new: "checks-expected", indeterminate: "checks-pending", done: "checks-success" };

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function ConnectForm({ jira, onConnected }: { jira: JiraBridge; onConnected(status: JiraStatus): void }) {
  const [deployment, setDeployment] = useState<JiraDeployment>("cloud");
  const [siteUrl, setSiteUrl] = useState("");
  const [email, setEmail] = useState("");
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      onConnected(await jira.connect({ siteUrl, deployment, email, token }));
    } catch (cause) {
      setError(message(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form className="work-section work-form" onSubmit={(event) => void submit(event)} data-testid="jira-connect">
      <p className="muted">토큰은 이 컴퓨터의 OS 암호화로만 저장합니다. 저장소와 Akra 설정 파일에는 넣지 않습니다.</p>
      <label>
        종류
        <select value={deployment} onChange={(event) => setDeployment(event.target.value as JiraDeployment)}>
          <option value="cloud">Cloud (이메일 + API 토큰)</option>
          <option value="server">Server / Data Center (PAT)</option>
        </select>
      </label>
      <label>
        주소
        <input value={siteUrl} onChange={(event) => setSiteUrl(event.target.value)} placeholder="https://회사.atlassian.net" required />
      </label>
      {deployment === "cloud" && (
        <label>
          이메일
          <input type="email" value={email} onChange={(event) => setEmail(event.target.value)} autoComplete="username" required />
        </label>
      )}
      <label>
        {deployment === "cloud" ? "API 토큰" : "Personal Access Token"}
        <input type="password" value={token} onChange={(event) => setToken(event.target.value)} autoComplete="off" required />
      </label>
      <button type="submit" disabled={busy}>
        {busy ? "확인 중…" : "연결"}
      </button>
      {error && (
        <div className="error" data-testid="jira-error">
          {error}
        </div>
      )}
    </form>
  );
}

function IssueRow({ issue, jira, onHandOff }: { issue: JiraIssue; jira: JiraBridge; onHandOff(issue: JiraIssue): Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const handOff = async () => {
    setBusy(true);
    setError(null);
    try {
      await onHandOff(issue);
    } catch (cause) {
      setError(message(cause));
    } finally {
      setBusy(false);
    }
  };
  return (
    <li className="work-item" data-testid="jira-issue">
      <button type="button" className="work-link" onClick={() => jira.openExternal(issue.url)} title={issue.url}>
        {issue.key} {issue.summary}
      </button>
      <div className="work-meta">
        {issue.status && <span className={`work-badge ${CATEGORY_CLASS[issue.statusCategory ?? ""] ?? ""}`}>{issue.status}</span>}
        {issue.issueType && <span className="muted">{issue.issueType}</span>}
        {issue.priority && <span className="muted">· {issue.priority}</span>}
        <button type="button" onClick={() => void handOff()} disabled={busy} data-testid="jira-hand-off">
          {busy ? "읽는 중…" : "스레드로 넘기기"}
        </button>
      </div>
      {error && <div className="error">{error}</div>}
    </li>
  );
}

export function JiraPanel({ jira, onHandOff }: { jira: JiraBridge; onHandOff(issue: JiraIssue): Promise<void> }) {
  const [status, setStatus] = useState<JiraStatus | null>(null);
  const [list, setList] = useState<ListState>({ phase: "loading" });

  const refresh = () => {
    setList({ phase: "loading" });
    jira.summary().then(
      (summary) => setList({ phase: "loaded", summary }),
      (error: unknown) => setList({ phase: "failed", message: message(error) }),
    );
  };

  useEffect(() => {
    jira.status().then(setStatus, () => setStatus({ connected: false }));
  }, [jira]);
  useEffect(() => {
    if (status?.connected) refresh();
  }, [status]);

  const disconnect = async () => {
    setStatus(await jira.disconnect());
  };

  return (
    <aside className="work-panel" data-testid="jira-panel">
      <div className="work-header">
        <strong>Jira</strong>
        {status?.connected && <span className="muted">{status.email ?? new URL(status.siteUrl).host}</span>}
        {status?.connected && (
          <span className="work-actions">
            <button type="button" onClick={refresh} disabled={list.phase === "loading"}>
              {list.phase === "loading" ? "읽는 중…" : "새로 고침"}
            </button>
            <button type="button" onClick={() => void disconnect()} data-testid="jira-disconnect">
              연결 해제
            </button>
          </span>
        )}
      </div>
      {status && !status.connected && <ConnectForm jira={jira} onConnected={setStatus} />}
      {status?.connected && list.phase === "failed" && (
        <div className="error work-section" data-testid="jira-error">
          {list.message}
        </div>
      )}
      {status?.connected && list.phase === "loaded" && (
        <div className="work-scroll">
          <section className="work-section">
            <h3>나에게 배정된 티켓 ({list.summary.issues.length})</h3>
            {list.summary.issues.length === 0 && <p className="muted">끝나지 않은 티켓이 없습니다.</p>}
            <ul>
              {list.summary.issues.map((issue) => (
                <IssueRow key={issue.key} issue={issue} jira={jira} onHandOff={onHandOff} />
              ))}
            </ul>
          </section>
        </div>
      )}
    </aside>
  );
}
