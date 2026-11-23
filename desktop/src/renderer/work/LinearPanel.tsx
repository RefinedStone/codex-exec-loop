import { useCallback, useEffect, useRef, useState } from "react";

import type { LinearBridge, LinearFilters, LinearIssue, LinearSummary } from "../../shared/linear";
import { providerConnectionKey, useProviderConnection } from "./useProviderConnection";
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
  const connection = useProviderConnection(linear);
  const { state, busy, run } = connection;
  const active = state?.activeConnection;
  const scope = providerConnectionKey(state);
  const [filters, setFilters] = useState<LinearFilters>(NO_FILTERS);
  const [summary, setSummary] = useState<LinearSummary | null>(null);
  const [loading, setLoading] = useState(false);
  const [listError, setListError] = useState<string | null>(null);
  const mounted = useRef(false);
  const request = useRef(0);

  const load = useCallback(async (nextFilters: LinearFilters, refresh: boolean) => {
    const id = ++request.current;
    setLoading(true);
    setListError(null);
    try {
      const next = await linear.summary(nextFilters, refresh);
      if (mounted.current && request.current === id) setSummary(next);
    } catch (cause) {
      if (mounted.current && request.current === id) {
        setSummary(null);
        setListError(message(cause));
      }
    } finally {
      if (mounted.current && request.current === id) setLoading(false);
    }
  }, [linear]);

  useEffect(() => {
    mounted.current = true;
    request.current++;
    setSummary(null); setFilters(NO_FILTERS); setListError(null);
    setLoading(Boolean(scope));
    if (scope) void load(NO_FILTERS, true);
    return () => { mounted.current = false; request.current++; };
  }, [linear, load, scope]);

  const filter = (next: LinearFilters) => { setFilters(next); void load(next, false); };
  const groups = new Map<string, { name: string; issues: LinearIssue[] }>();
  for (const issue of summary?.issues ?? []) {
    const group = groups.get(issue.state.id) ?? { name: issue.state.name, issues: [] };
    group.issues.push(issue);
    groups.set(issue.state.id, group);
  }
  const error = connection.error ?? listError;

  return (
    <aside className="work-panel linear-panel" data-testid="linear-panel" aria-label="Linear">
      <div className="work-header">
        <strong>Linear</strong>
        {active && <span className="muted linear-viewer">{active.metadata.accountName}</span>}
        {active && <span className="work-actions">
          <button type="button" onClick={() => void load(filters, true)} disabled={busy || loading} data-testid="linear-refresh">새로 고침</button>
          <button type="button" onClick={() => void run(() => linear.connect())} disabled={busy || !connection.canConnect} data-testid="linear-reconnect">연결 변경</button>
          <button type="button" onClick={() => void run(() => linear.disconnect())} data-testid="linear-disconnect">연결 해제</button>
        </span>}
      </div>
      {!state && !error && <p className="muted work-section" role="status">연결 상태 확인 중…</p>}
      {state && !active && <div className="work-section work-form" data-testid="linear-connect">
        <p className="muted">개인 API 키로 배정된 이슈를 읽습니다. 키는 별도 시스템 창에서 입력하고 OS 키 저장소에 보관합니다.</p>
        <p className="muted">회사 데이터 정책을 확인한 뒤 Linear 설정의 Security &amp; access → Personal API keys에서 만든 키로 연결하세요.</p>
        <button type="button" disabled={busy || !connection.canConnect} onClick={() => void run(() => linear.connect())} data-testid="linear-connect-submit">
          {busy ? "확인 중…" : "시스템 창에서 연결"}
        </button>
        {!connection.canConnect && <p className="muted" role="status">이 환경에서는 새 자격 증명을 입력할 수 없습니다.</p>}
      </div>}
      {connection.canMigrate && <div className="work-section">
        <button type="button" disabled={busy} onClick={() => void run(() => linear.migrateLegacy())} data-testid="linear-migrate">
          {connection.legacy.kind === "pending" ? "기존 연결 이관 확인" : "기존 연결 옮기기"}
        </button>
        <p className="muted">저장한 키를 다시 입력하지 않고 옮깁니다.</p>
      </div>}
      {connection.canCleanLegacy && <div className="work-section" role="status">
        <p className="muted">연결은 해제되어 있습니다. 저장된 기존 연결을 정리할 수 있습니다.</p>
        <button type="button" disabled={busy} onClick={() => void run(() => linear.disconnect())} data-testid="linear-disconnect-legacy">남은 기존 연결 삭제</button>
      </div>}
      {(busy || connection.pending) && <div className="work-section" role="status">
        연결 작업을 확인하고 있습니다.
        <button type="button" onClick={() => void run(() => linear.cancel())} data-testid="linear-cancel">작업 취소</button>
      </div>}
      {state?.operation?.cleanupPending && <p className="muted work-section" role="status">{state.operation.kind === "disconnect" ? "연결은 해제했습니다. 저장소에서 자격 증명을 지우는 작업이 남아 있습니다." : "현재 연결은 유지됩니다. 이전 자격 증명 정리가 남아 있습니다."}</p>}
      {error && <div className="error work-section" role="alert" data-testid="linear-error">{error}</div>}
      {(error || connection.pending || state?.operation?.cleanupPending) && <div className="work-section">
        <button type="button" onClick={() => void run(() => linear.retry())} data-testid="linear-retry">{state?.operation?.kind === "disconnect" && state.operation.cleanupPending ? "삭제 다시 시도" : "상태 확인"}</button>
      </div>}
      {active && (
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
      {active && !loading && summary && (
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
