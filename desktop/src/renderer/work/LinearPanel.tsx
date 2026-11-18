import { type FormEvent, useCallback, useEffect, useRef, useState } from "react";

import type { LinearBridge, LinearFilters, LinearIssue, LinearStatus, LinearSummary } from "../../shared/linear";
import "./LinearPanel.css";

const NO_FILTERS: LinearFilters = { teamId: null, projectId: null, currentCycle: false };
const PRIORITY = ["우선순위 없음", "긴급", "높음", "보통", "낮음"];

function message(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error);
  return text.replace(/^Error invoking remote method '[^']+': (?:Error: )?/, "");
}

function IssueRow({
  issue,
  linear,
  disabled,
  onHandOff,
}: {
  issue: LinearIssue;
  linear: LinearBridge;
  disabled: boolean;
  onHandOff(issue: LinearIssue): Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const act = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (cause) {
      if (mounted.current) setError(message(cause));
    } finally {
      if (mounted.current) setBusy(false);
    }
  };

  return (
    <li className="work-item" data-testid="linear-issue">
      <button
        type="button"
        className="work-link"
        title={issue.url}
        disabled={disabled || busy}
        onClick={() => void act(() => linear.openExternal(issue.url))}
      >
        <span className="linear-identifier">{issue.identifier}</span> {issue.title}
      </button>
      <div className="work-meta">
        <span className="muted">{PRIORITY[issue.priority] ?? `우선순위 ${issue.priority}`}</span>
        {issue.project && <span className="muted">{issue.project.name}</span>}
        <button
          type="button"
          disabled={disabled || busy}
          onClick={() => void act(() => onHandOff(issue))}
          aria-label={`${issue.identifier} 스레드로 넘기기`}
          data-testid="linear-hand-off"
        >
          {busy ? "읽는 중…" : "스레드로 넘기기"}
        </button>
      </div>
      {error && <div className="error" role="alert" data-testid="linear-error">{error}</div>}
    </li>
  );
}

export function LinearPanel({ linear, onHandOff }: { linear: LinearBridge; onHandOff(issue: LinearIssue): Promise<void> }) {
  const [status, setStatus] = useState<LinearStatus | null>(null);
  const [token, setToken] = useState("");
  const [filters, setFilters] = useState<LinearFilters>(NO_FILTERS);
  const [summary, setSummary] = useState<LinearSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(false);
  const request = useRef(0);

  const load = useCallback(async (nextFilters: LinearFilters, refresh: boolean) => {
    const id = ++request.current;
    setLoading(true);
    setError(null);
    try {
      const next = await linear.summary(nextFilters, refresh);
      if (mounted.current && request.current === id) setSummary(next);
    } catch (cause) {
      if (mounted.current && request.current === id) {
        setSummary(null);
        setError(message(cause));
      }
    } finally {
      if (mounted.current && request.current === id) setLoading(false);
    }
  }, [linear]);

  useEffect(() => {
    mounted.current = true;
    const id = ++request.current;
    setStatus(null);
    setSummary(null);
    setFilters(NO_FILTERS);
    setToken("");
    setBusy(false);
    setLoading(true);
    setError(null);
    void linear.status().then(
      (next) => {
        if (!mounted.current || request.current !== id) return;
        setStatus(next);
        if (next.connected) void load(NO_FILTERS, true);
        else setLoading(false);
      },
      (cause: unknown) => {
        if (!mounted.current || request.current !== id) return;
        setStatus({ connected: false });
        setLoading(false);
        setError(message(cause));
      },
    );
    return () => {
      mounted.current = false;
      request.current++;
    };
  }, [linear, load]);

  const connect = async (event: FormEvent) => {
    event.preventDefault();
    if (busy || !token.trim()) return;
    const id = ++request.current;
    setBusy(true);
    setError(null);
    try {
      const next = await linear.connect(token.trim());
      if (!mounted.current || request.current !== id) return;
      setStatus(next);
      if (next.connected) {
        setToken("");
        setFilters(NO_FILTERS);
        setBusy(false);
        void load(NO_FILTERS, true);
      } else {
        setBusy(false);
        setError("연결되지 않았습니다. 개인 API 키를 확인하고 다시 연결하세요.");
      }
    } catch (cause) {
      if (mounted.current && request.current === id) {
        setBusy(false);
        setError(message(cause));
      }
    }
  };

  const disconnect = async () => {
    const id = ++request.current;
    setBusy(true);
    setLoading(false);
    setSummary(null);
    setError(null);
    try {
      const next = await linear.disconnect();
      if (!mounted.current || request.current !== id) return;
      setStatus(next);
      setToken("");
      setFilters(NO_FILTERS);
      setBusy(false);
      if (next.connected) void load(NO_FILTERS, false);
    } catch (cause) {
      if (mounted.current && request.current === id) {
        setBusy(false);
        setError(message(cause));
      }
    }
  };

  const filter = (next: LinearFilters) => {
    setFilters(next);
    void load(next, false);
  };
  const groups = new Map<string, { name: string; issues: LinearIssue[] }>();
  for (const issue of summary?.issues ?? []) {
    const group = groups.get(issue.state.id) ?? { name: issue.state.name, issues: [] };
    group.issues.push(issue);
    groups.set(issue.state.id, group);
  }

  return (
    <aside className="work-panel linear-panel" data-testid="linear-panel" aria-label="Linear">
      <div className="work-header">
        <strong>Linear</strong>
        {status?.connected && <span className="muted linear-viewer">{status.viewerName}</span>}
        {status?.connected && (
          <span className="work-actions">
            <button type="button" onClick={() => void load(filters, true)} disabled={busy || loading} data-testid="linear-refresh">
              새로 고침
            </button>
            <button type="button" onClick={() => void disconnect()} disabled={busy} data-testid="linear-disconnect">
              {busy ? "해제 중…" : "연결 해제"}
            </button>
          </span>
        )}
      </div>
      {status && !status.connected && (
        <form className="work-section work-form" onSubmit={(event) => void connect(event)} data-testid="linear-connect">
          <p className="muted">개인 API 키로 배정된 이슈를 읽습니다. 키는 이 컴퓨터의 OS 암호화로만 저장합니다.</p>
          <p className="muted">Linear 설정의 Security &amp; access → Personal API keys에서 키를 만드세요.</p>
          <label>
            개인 API 키
            <input
              type="password"
              value={token}
              onChange={(event) => setToken(event.target.value)}
              autoComplete="off"
              spellCheck={false}
              disabled={busy}
              required
              data-testid="linear-key"
            />
          </label>
          <button type="submit" disabled={busy || !token.trim()} data-testid="linear-connect-submit">
            {busy ? "확인 중…" : "연결"}
          </button>
        </form>
      )}
      {error && <div className="error work-section" role="alert" data-testid="linear-error">{error}</div>}
      {status?.connected && (
        <div className="work-section linear-filters" aria-label="이슈 필터">
          <label>
            팀
            <select
              value={filters.teamId ?? ""}
              disabled={busy}
              onChange={(event) => filter({ ...filters, teamId: event.target.value || null })}
              data-testid="linear-team"
            >
              <option value="">모든 팀</option>
              {summary?.teams.map((team) => <option key={team.id} value={team.id}>{team.name}</option>)}
            </select>
          </label>
          <label>
            프로젝트
            <select
              value={filters.projectId ?? ""}
              disabled={busy}
              onChange={(event) => filter({ ...filters, projectId: event.target.value || null })}
              data-testid="linear-project"
            >
              <option value="">모든 프로젝트</option>
              {summary?.projects.map((project) => <option key={project.id} value={project.id}>{project.name}</option>)}
            </select>
          </label>
          <label className="linear-cycle">
            <input
              type="checkbox"
              checked={filters.currentCycle}
              disabled={busy}
              onChange={(event) => filter({ ...filters, currentCycle: event.target.checked })}
              data-testid="linear-cycle"
            />
            현재 사이클
          </label>
        </div>
      )}
      {loading && <p className="muted work-section" role="status" data-testid="linear-loading">읽는 중…</p>}
      {status?.connected && !loading && summary && (
        <div className="work-scroll">
          <div className="work-section">
            <h3>나에게 배정된 이슈 ({summary.issues.length})</h3>
            <p className="muted">최근 수정된 미완료 이슈 최대 {summary.limit}개에서 표시합니다.</p>
            {summary.issues.length === 0 && (
              <p className="muted" role="status" data-testid={summary.totalCached === 0 ? "linear-empty" : "linear-filtered-empty"}>
                {summary.totalCached === 0 ? "배정된 미완료 이슈가 없습니다." : "필터에 맞는 이슈가 없습니다. 다른 필터를 선택하세요."}
              </p>
            )}
          </div>
          {[...groups].map(([id, group]) => (
            <section key={id} className="work-section" aria-label={group.name}>
              <h3>{group.name} ({group.issues.length})</h3>
              <ul>
                {group.issues.map((issue) => (
                  <IssueRow key={issue.id} issue={issue} linear={linear} disabled={busy} onHandOff={onHandOff} />
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}
    </aside>
  );
}
