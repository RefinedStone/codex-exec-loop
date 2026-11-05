// GitHub 업무 패널이다 (R4). 내 PR(CI, 해결 안 된 리뷰)과 나에게 온 리뷰 요청을 보여 준다. 열 때와 "새로 고침" 때만 읽는다.
// "스레드로 넘기기" 는 해결되지 않은 리뷰 코멘트로 새 스레드를 만들고 입력창만 채운다. 보내기는 사용자가 한다.
import { useEffect, useState } from "react";

import type { GitHubBridge, GitHubPullRequest, GitHubSummary } from "../../shared/bridge";

type PanelState = { phase: "loading" } | { phase: "loaded"; summary: GitHubSummary } | { phase: "failed"; message: string };

const CHECK_LABEL: Record<string, string> = { SUCCESS: "CI 통과", FAILURE: "CI 실패", ERROR: "CI 오류", PENDING: "CI 진행 중", EXPECTED: "CI 대기" };

function PullRequestRow({
  pull,
  github,
  onHandOff,
}: {
  pull: GitHubPullRequest;
  github: GitHubBridge;
  onHandOff?(pull: GitHubPullRequest): Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const handOff = async () => {
    if (!onHandOff) return;
    setBusy(true);
    setError(null);
    try {
      await onHandOff(pull);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };
  return (
    <li className="work-item" data-testid="github-pull">
      <button type="button" className="work-link" onClick={() => github.openExternal(pull.url)} title={pull.url}>
        {pull.title}
      </button>
      <div className="work-meta">
        <span className="muted">
          {pull.repository}#{pull.number}
          {pull.author ? ` · ${pull.author}` : ""}
        </span>
        {pull.isDraft && <span className="work-badge">초안</span>}
        {pull.checks && <span className={`work-badge checks-${pull.checks.toLowerCase()}`}>{CHECK_LABEL[pull.checks] ?? pull.checks}</span>}
        {pull.unresolvedThreads !== null && pull.unresolvedThreads > 0 && (
          <span className="work-badge" data-testid="github-unresolved">
            리뷰 {pull.unresolvedThreads}건
          </span>
        )}
        {onHandOff && pull.unresolvedThreads !== null && pull.unresolvedThreads > 0 && (
          <button type="button" onClick={() => void handOff()} disabled={busy} data-testid="github-hand-off">
            {busy ? "읽는 중…" : "스레드로 넘기기"}
          </button>
        )}
      </div>
      {error && <div className="error">{error}</div>}
    </li>
  );
}

export function GitHubPanel({ github, onHandOff }: { github: GitHubBridge; onHandOff(pull: GitHubPullRequest): Promise<void> }) {
  const [state, setState] = useState<PanelState>({ phase: "loading" });

  const refresh = () => {
    setState({ phase: "loading" });
    github.summary().then(
      (summary) => setState({ phase: "loaded", summary }),
      (error: unknown) => setState({ phase: "failed", message: error instanceof Error ? error.message : String(error) }),
    );
  };
  useEffect(refresh, [github]);

  return (
    <aside className="work-panel" data-testid="github-panel">
      <div className="work-header">
        <strong>GitHub</strong>
        {state.phase === "loaded" && <span className="muted">{state.summary.login}</span>}
        <button type="button" onClick={refresh} disabled={state.phase === "loading"}>
          {state.phase === "loading" ? "읽는 중…" : "새로 고침"}
        </button>
      </div>
      {state.phase === "failed" && (
        <div className="error work-section" data-testid="github-error">
          GitHub 정보를 읽을 수 없습니다. gh 설치와 `gh auth login` 을 확인하세요. {state.message}
        </div>
      )}
      {state.phase === "loaded" && (
        <div className="work-scroll">
          <section className="work-section">
            <h3>내 PR ({state.summary.myPullRequests.length})</h3>
            {state.summary.myPullRequests.length === 0 && <p className="muted">열린 PR 이 없습니다.</p>}
            <ul>
              {state.summary.myPullRequests.map((pull) => (
                <PullRequestRow key={pull.url} pull={pull} github={github} onHandOff={onHandOff} />
              ))}
            </ul>
          </section>
          <section className="work-section">
            <h3>리뷰 요청 ({state.summary.reviewRequests.length})</h3>
            {state.summary.reviewRequests.length === 0 && <p className="muted">받은 리뷰 요청이 없습니다.</p>}
            <ul>
              {state.summary.reviewRequests.map((pull) => (
                <PullRequestRow key={pull.url} pull={pull} github={github} />
              ))}
            </ul>
          </section>
        </div>
      )}
    </aside>
  );
}
