// 브랜치가 기준 브랜치 뒤에 더한 커밋과 그 브랜치의 PR 이다 (R10). `변경` 패널의 작업 트리 목록 아래에 있다.
// 커밋의 변경은 펼칠 때 하나씩 읽는다.
import { useMemo, useState } from "react";

import type { AkraBridge, GitBranchSummary, GitChecksState, GitCommit, GitPublishResult, GitPullRequest } from "../../shared/bridge";
import { Icon } from "../design/icons";
import { toast } from "../session/uiStore";
import { relativeAge } from "../sidebar/threadStatus";
import { canCreatePullRequest, canPush, initialPullRequestDraft, type LoadedSummary } from "./branchPublish";
import { parseGitDiff } from "./diff";
import { DiffFileList } from "./DiffView";

export type BranchState = { phase: "idle" } | { phase: "loading" } | { phase: "loaded"; summary: GitBranchSummary } | { phase: "failed"; message: string };

type CommitReader = Pick<AkraBridge, "gitCommitDiff">;
type Publisher = Pick<AkraBridge, "gitPush" | "gitCreatePullRequest">;

/** 동작 하나가 끝날 때까지 패널의 모든 동작 버튼을 막는다. */
export interface ActionLock {
  busy: boolean;
  setBusy(busy: boolean): void;
}

/** push 버튼과 PR 만들기 양식이다. 둘 다 main 이 확인 창을 띄운 뒤에 실행한다. */
function PublishActions({ cwd, summary, git, lock, onChanged, onOpenUrl }: { cwd: string; summary: LoadedSummary; git: Publisher; lock: ActionLock; onChanged(): void; onOpenUrl(url: string): void }) {
  const [form, setForm] = useState<{ title: string; body: string; base: string; draft: boolean } | null>(null);
  const run = (label: string, action: Promise<GitPublishResult>, done: (url: string | null) => void) => {
    lock.setBusy(true);
    action
      .then(
        (result) => {
          if (result.ok) {
            done(result.url);
            onChanged();
          } else if (!result.cancelled) {
            toast(`${label} 실패: ${result.message}`);
          }
        },
        (error: unknown) => toast(`${label} 실패: ${error instanceof Error ? error.message : String(error)}`),
      )
      .finally(() => lock.setBusy(false));
  };
  const pushable = canPush(summary);
  const creatable = canCreatePullRequest(summary);
  if (!pushable && !creatable) return null;
  return (
    <div className="publish">
      <div className="publish-buttons">
        {pushable && (
          <button type="button" className="button primary" disabled={lock.busy} onClick={() => run("push", git.gitPush(cwd), () => toast("push 했습니다"))} data-testid="branch-push">
            push
          </button>
        )}
        {creatable && !form && (
          <button type="button" className="button primary" disabled={lock.busy} onClick={() => setForm({ ...initialPullRequestDraft(summary), draft: false })} data-testid="branch-pr-new">
            <Icon name="pullRequest" size={13} />
            PR 만들기
          </button>
        )}
      </div>
      {creatable && form && (
        <form
          className="work-form publish-form"
          data-testid="branch-pr-form"
          onSubmit={(event) => {
            event.preventDefault();
            run("PR 만들기", git.gitCreatePullRequest(cwd, form), (url) => {
              setForm(null);
              toast("PR 을 만들었습니다");
              if (url) onOpenUrl(url);
            });
          }}
        >
          <label>
            제목
            <input value={form.title} onChange={(event) => setForm({ ...form, title: event.target.value })} maxLength={256} required />
          </label>
          <label>
            본문
            <textarea value={form.body} onChange={(event) => setForm({ ...form, body: event.target.value })} rows={6} />
          </label>
          <label>
            대상 브랜치
            <input value={form.base} onChange={(event) => setForm({ ...form, base: event.target.value })} required />
          </label>
          <label className="publish-check">
            <input type="checkbox" checked={form.draft} onChange={(event) => setForm({ ...form, draft: event.target.checked })} />
            초안으로 만들기
          </label>
          <div className="publish-buttons">
            <button type="submit" className="button primary" disabled={lock.busy || form.title.trim().length === 0 || form.base.trim().length === 0}>
              PR 만들기
            </button>
            <button type="button" className="button" onClick={() => setForm(null)}>
              취소
            </button>
          </div>
        </form>
      )}
    </div>
  );
}

type CommitState = { phase: "loading" } | { phase: "loaded"; text: string; truncated: boolean } | { phase: "failed"; message: string };

const PR_STATE_LABEL: Record<GitPullRequest["state"], string> = { OPEN: "열림", MERGED: "머지됨", CLOSED: "닫힘" };
const CHECKS_LABEL: Record<GitChecksState, string> = { SUCCESS: "CI 통과", FAILURE: "CI 실패", PENDING: "CI 진행 중" };
const CHECKS_BADGE: Record<GitChecksState, string> = { SUCCESS: "badge-ok", FAILURE: "badge-bad", PENDING: "badge-warn" };

/** 위 막대와 패널이 함께 쓰는 PR 표시이다. 실패한 CI 는 빨간색이다. */
export function pullRequestBadgeClass(pullRequest: GitPullRequest): string {
  return pullRequest.checks === "FAILURE" ? "badge badge-bad" : pullRequest.state === "OPEN" ? "badge badge-ok" : "badge";
}

function PullRequestLine({ pullRequest, onOpenUrl }: { pullRequest: GitPullRequest; onOpenUrl(url: string): void }) {
  return (
    <div className="branch-pr" data-testid="branch-pr">
      <Icon name="pullRequest" size={14} />
      <span className={pullRequestBadgeClass(pullRequest)}>PR #{pullRequest.number}</span>
      <span className="branch-pr-title">{pullRequest.title}</span>
      <span className="badge">{pullRequest.isDraft && pullRequest.state === "OPEN" ? "초안" : PR_STATE_LABEL[pullRequest.state]}</span>
      {pullRequest.checks && <span className={`badge ${CHECKS_BADGE[pullRequest.checks]}`}>{CHECKS_LABEL[pullRequest.checks]}</span>}
      <span className="bar-spacer" />
      <button type="button" className="button" onClick={() => onOpenUrl(pullRequest.url)}>
        GitHub 에서 열기
      </button>
    </div>
  );
}

function CommitRow({ cwd, commit, git }: { cwd: string; commit: GitCommit; git: CommitReader }) {
  const [open, setOpen] = useState(false);
  const [state, setState] = useState<CommitState>({ phase: "loading" });
  const files = useMemo(() => (state.phase === "loaded" ? parseGitDiff(state.text) : []), [state]);

  const toggle = (next: boolean) => {
    setOpen(next);
    if (next && state.phase !== "loaded") {
      setState({ phase: "loading" });
      git.gitCommitDiff(cwd, commit.sha).then(
        (diff) => setState({ phase: "loaded", ...diff }),
        (error: unknown) => setState({ phase: "failed", message: error instanceof Error ? error.message : String(error) }),
      );
    }
  };

  return (
    <details className="diff-file" data-testid="branch-commit" open={open} onToggle={(event) => toggle(event.currentTarget.open)}>
      <summary>
        <code>{commit.shortSha}</code>
        <span className="commit-subject">{commit.subject}</span>
        {commit.pushed === false && <span className="badge badge-warn">push 전</span>}
        <span className="muted commit-meta">
          {commit.author} · {relativeAge(commit.committedAt, Date.now())}
        </span>
      </summary>
      {open && state.phase === "loading" && <div className="muted diff-hint">읽는 중…</div>}
      {open && state.phase === "failed" && <div className="error diff-hint">{state.message}</div>}
      {open && state.phase === "loaded" && (
        <div className="commit-files">
          {files.length > 0 ? <DiffFileList files={files} /> : <div className="muted diff-hint">줄 단위 변경이 없습니다.</div>}
          {state.truncated && <div className="muted diff-hint">변경이 커서 앞부분만 보여 줍니다.</div>}
        </div>
      )}
    </details>
  );
}

export function BranchCommits({
  cwd,
  state,
  git,
  lock,
  onChanged,
  onOpenUrl,
}: {
  cwd: string;
  state: BranchState;
  git: CommitReader & Publisher;
  lock: ActionLock;
  /** push 나 PR 만들기가 끝났다. 목록을 다시 읽는다. */
  onChanged(): void;
  onOpenUrl(url: string): void;
}) {
  if (state.phase === "idle") {
    return null;
  }
  if (state.phase === "loading") {
    return <div className="muted diff-hint branch-commits">커밋과 PR 을 읽는 중…</div>;
  }
  const failure = state.phase === "failed" ? state.message : state.summary.ok ? null : state.summary.message;
  if (state.phase === "failed" || !state.summary.ok) {
    return (
      <div className="error diff-hint branch-commits" data-testid="branch-error">
        브랜치의 커밋을 읽을 수 없습니다. {failure}
      </div>
    );
  }
  const { summary } = state;
  const unpushed = summary.commits.filter((commit) => commit.pushed === false).length;
  return (
    <section className="branch-commits" data-testid="branch-commits">
      <header className="review-toolbar">
        <strong>커밋 {summary.commits.length}{summary.truncated ? "+" : ""}개</strong>
        {summary.base ? <span className="muted">기준 {summary.base}</span> : <span className="muted">기준 브랜치를 찾지 못했습니다.</span>}
        {summary.base && unpushed > 0 && <span className="badge badge-warn">push 전 {unpushed}</span>}
        {summary.base && !summary.remoteBranch && summary.branch && summary.pullRequest?.state !== "MERGED" && <span className="muted">원격 브랜치 없음</span>}
      </header>
      {summary.pullRequest ? (
        <PullRequestLine pullRequest={summary.pullRequest} onOpenUrl={onOpenUrl} />
      ) : (
        <div className="muted diff-hint" data-testid="branch-no-pr">
          {summary.pullRequestError ? `PR 을 읽지 못했습니다. ${summary.pullRequestError}` : summary.repository ? "이 브랜치의 PR 이 없습니다." : "GitHub 저장소가 아니어서 PR 을 찾지 않습니다."}
        </div>
      )}
      <PublishActions cwd={cwd} summary={summary} git={git} lock={lock} onChanged={onChanged} onOpenUrl={onOpenUrl} />
      {summary.commits.length > 0 && (
        <div className="working-tree-files diff-file-list">
          {summary.commits.map((commit) => (
            <CommitRow key={commit.sha} cwd={cwd} commit={commit} git={git} />
          ))}
        </div>
      )}
    </section>
  );
}
