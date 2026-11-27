// 커밋 그래프 패널이다 (plan 27 G1 – G3). 열린 스레드의 저장소의 커밋을 그래프로 보여 준다. 저장소를 고치는 일은
// 고른 커밋의 작업 단추에서 시작하고 Gateway 의 작업 엔진이 한다 (`GitOperationViews.tsx`).
// 긴 이력은 보이는 줄만 그린다. 끝에 가까워지면 다음 쪽을 읽는다. 줄 높이는 고정이다: 줄마다 그린 선이 위아래 줄과
// 이어져야 한다. 저장소가 바뀌었는지 주기적으로 보고, 바뀌면 보던 자리를 지키며 다시 읽는다. 줄을 고르면 그 커밋의
// 상세를 아래에 보인다. 최대화하면 왼쪽에 ref 목록을 더한다 (plan 27 §7.2). 그래프 위의 상태 띠는 끝나지 않은 git
// 작업, 충돌, 브랜치 없는 HEAD 를 한 줄로 쓴다 (§7.4 의 6). 열린 계획은 결과의 자리를 점선으로 그린다 (§7.4 의 2).
import { type ReactNode, useCallback, useMemo, useState } from "react";
import { copyText } from "../conversation/copyText";
import { isTurnInProgress } from "../conversation/transcript";
import { parseGitDiff } from "../conversation/diff";
import { DiffFileList, DiffStyleToggle } from "../conversation/DiffView";
import { useDesktopStore } from "../session/desktopStore";
import type { GatewayRequest } from "../work/notionHub";
import { fullTime, headName, shortSha, threadIdsInWorktreeOf, unfinishedWork, type GitCommitDetail, type GitRef, type GitRefKind, type GitWorktree } from "./gitGraph";
import { CommitActionsView, OperationLogView, useGitOperationLog } from "./GitOperationViews";
import type { GitOperationPlan } from "./gitOperations";
import { GraphRows } from "./GraphRows";
import { useCommitDetail, useGitGraph, type CommitDetailState } from "./useGitGraph";
export { useCommitDetail, useGitGraph, REFS_POLL_INTERVAL_MS } from "./useGitGraph";

function CommitDetailBody({ detail, onSelect }: { detail: GitCommitDetail; onSelect(commit: string): void }) {
  const files = useMemo(() => parseGitDiff(detail.diff), [detail.diff]);
  const committer = detail.committerName !== "" && detail.committerName !== detail.authorName ? ` · 커밋 ${detail.committerName}` : "";
  return (
    <div className="work-scroll">
      <pre className="git-detail-message" data-testid="git-commit-message">
        {detail.message}
      </pre>
      <p className="muted work-section">
        {detail.authorName}
        {detail.authorEmail && ` <${detail.authorEmail}>`} · {fullTime(detail.authorTime)}
        {committer}
      </p>
      {detail.parents.length > 0 && (
        <p className="muted work-section">
          부모{" "}
          {detail.parents.map((parent) => (
            <button key={parent} type="button" title={parent} onClick={() => onSelect(parent)} data-testid="git-commit-parent">
              <code>{shortSha(parent)}</code>
            </button>
          ))}
          {detail.parents.length > 1 && " · 변경은 첫 부모와 비교한 것입니다."}
        </p>
      )}
      {detail.diffTooLarge && <p className="muted work-section">변경이 2 MiB 를 넘어서 읽지 않았습니다.</p>}
      {!detail.diffTooLarge && files.length === 0 && <p className="muted work-section">변경한 파일이 없습니다.</p>}
      {files.length > 0 && <DiffFileList files={files} />}
    </div>
  );
}

function CommitDetail({ commit, state, actions, onSelect, onClose }: { commit: string; state: CommitDetailState; actions: ReactNode; onSelect(commit: string): void; onClose(): void }) {
  return (
    <section className="git-detail" aria-label="커밋 상세" data-testid="git-commit-detail">
      <div className="work-header">
        <strong>커밋</strong>
        <code title={commit}>{shortSha(commit)}</code>
        <span className="work-actions">
          <button type="button" onClick={() => copyText(commit, "sha 를 복사했습니다")}>
            sha 복사
          </button>
          <DiffStyleToggle />
          <button type="button" onClick={onClose} data-testid="git-commit-close">
            닫기
          </button>
        </span>
      </div>
      {actions}
      {(state.phase === "loading" || state.phase === "idle") && <p className="muted work-section">읽는 중…</p>}
      {state.phase === "missing" && <p className="muted work-section">이 커밋은 저장소에 없습니다.</p>}
      {state.phase === "failed" && (
        <div className="error work-section" data-testid="git-commit-error">
          {state.message}
        </div>
      )}
      {state.phase === "loaded" && <CommitDetailBody detail={state.detail} onSelect={onSelect} />}
    </section>
  );
}

const REF_GROUPS: readonly { kind: GitRefKind; title: string }[] = [
  { kind: "local_branch", title: "로컬" },
  { kind: "remote_branch", title: "원격" },
  { kind: "tag", title: "태그" },
  { kind: "backup", title: "백업" },
];

/** 최대화한 패널의 왼쪽 ref 목록이다. 누르면 그 커밋을 고른다. */
function RefList({ refs, worktrees, selected, onSelect }: { refs: readonly GitRef[]; worktrees: readonly GitWorktree[]; selected: string | null; onSelect(commit: string): void }) {
  const item = (key: string, commit: string | null, name: string, title: string, head: boolean) => (
    <button key={key} type="button" className={`git-ref-item${commit !== null && commit === selected ? " selected" : ""}`} title={title} disabled={commit === null} onClick={() => commit && onSelect(commit)} data-testid="git-ref-item">
      <span className="git-ref-item-name">{name}</span>
      {head && <span className="git-ref-item-head">HEAD</span>}
    </button>
  );
  return (
    <nav className="git-ref-list" aria-label="ref 목록" data-testid="git-ref-list">
      {REF_GROUPS.map(({ kind, title }) => {
        const group = refs.filter((ref) => ref.kind === kind);
        return (
          group.length > 0 && (
            <section key={kind}>
              <h3>
                {title} <span className="muted">{group.length}</span>
              </h3>
              {group.map((ref) => item(ref.name, ref.target, ref.name, ref.name, ref.head))}
            </section>
          )
        );
      })}
      {worktrees.length > 0 && (
        <section>
          <h3>
            worktree <span className="muted">{worktrees.length}</span>
          </h3>
          {worktrees.map((worktree) => item(worktree.path, worktree.head, worktree.branch ?? (worktree.head ? shortSha(worktree.head) : worktree.path), worktree.path, false))}
        </section>
      )}
    </nav>
  );
}

export interface GitGraphPanelProps {
  request: GatewayRequest;
  cwd: string | null;
  /** 패널을 최대화했다. 왼쪽에 ref 목록을 더한다. */
  maximized: boolean;
  /** 이 폴더의 커밋하지 않은 변경의 파일 수이다. 모르면 `null` 이다. */
  changes: number | null;
  /** "커밋하지 않은 변경" 줄을 누르면 부른다. */
  onOpenChanges(): void;
  onOpenThread(threadId: string): void;
}

export function GitGraphPanel({ request, cwd, maximized, changes, onOpenChanges, onOpenThread }: GitGraphPanelProps) {
  // 스레드 목록의 폴더들이다. 작업 트리의 커밋에 스레드 배지를 붙이는 데 쓴다. 패널이 열려 있을 때만 구독한다.
  const folders = useDesktopStore((state) => state.folders);
  const graph = useGitGraph(request, cwd);
  const { state, reload } = graph;
  const [selection, setSelection] = useState<{ cwd: string; commit: string } | null>(null);
  // 다른 폴더의 선택은 쓰지 않는다. sha 는 저장소마다 뜻이 다르다.
  const selected = selection !== null && selection.cwd === cwd ? selection.commit : null;
  const select = useCallback((commit: string) => cwd && setSelection({ cwd, commit }), [cwd]);
  const detail = useCommitDetail(request, cwd, selected);
  const [preview, setPreview] = useState<GitOperationPlan | null>(null);
  // 저장소가 아닌 폴더의 일지는 읽지 않는다. Gateway 는 일지를 읽을 때 그 폴더의 runtime 디렉터리를 만든다.
  const repositoryCwd = state.phase === "loaded" && state.repository ? cwd : null;
  const log = useGitOperationLog(request, repositoryCwd, graph.refresh);
  // 턴이 도는 동안 에이전트가 작업 트리의 파일을 쓴다. 그 사이에 reset 이나 체크아웃을 하면 에이전트의 일이 섞인다.
  // Gateway 는 Desktop 의 턴을 알지 못한다. 그래서 화면이 막는다 (plan 27 §9.3): 열린 스레드의 턴과, 같은 작업
  // 트리에 폴더가 있는 다른 스레드의 턴이다.
  const worktrees = state.phase === "loaded" ? state.worktrees : null;
  const turnRunning = useDesktopStore((store) => {
    if (isTurnInProgress(store.transcript)) return true;
    if (!cwd || !worktrees) return false;
    const neighbours = threadIdsInWorktreeOf(cwd, worktrees, store.folders);
    return store.activeThreadIds.some((id) => neighbours.includes(id));
  });
  const busyReason = turnRunning ? "이 작업 트리에서 턴이 실행 중입니다. 턴이 끝난 뒤에 실행하십시오." : null;
  const current = state.phase === "loaded" ? headName(state.refs, state.head) : null;
  const unfinished = state.phase === "loaded" ? unfinishedWork(graph.progress, state.refs, state.head) : [];
  // 첫 커밋 전에도 보인다. 커밋이 있으면 줄 목록의 맨 위에 둔다.
  const uncommitted = state.phase === "loaded" && state.repository && changes !== null && changes > 0 && (
    <button type="button" className="git-uncommitted" onClick={onOpenChanges} data-testid="git-graph-uncommitted">
      커밋하지 않은 변경 · 파일 {changes}개
    </button>
  );
  return (
    <aside className="work-panel git-graph-panel" data-testid="git-graph-panel">
      <div className="work-header">
        <strong>커밋 그래프</strong>
        {current && (
          <span className="muted" data-testid="git-graph-head">
            {current}
          </span>
        )}
        {state.phase === "loaded" && state.repository && (
          <span className="muted" data-testid="git-graph-count">
            {state.rows.length}개{state.next !== null ? " 읽음" : ""}
          </span>
        )}
        <span className="work-actions">
          <button type="button" onClick={reload} disabled={state.phase === "idle" || state.phase === "loading"} data-testid="git-graph-refresh">
            {state.phase === "loading" ? "읽는 중…" : "새로 고침"}
          </button>
        </span>
      </div>
      {unfinished.length > 0 && (
        <div className="git-status-strip" role="status" data-testid="git-status-strip">
          {unfinished.join(" · ")}
        </div>
      )}
      {state.phase === "idle" && <p className="muted work-section">스레드를 열면 그 스레드의 저장소의 커밋 그래프를 보여 줍니다.</p>}
      {state.phase === "failed" && (
        <div className="error work-section" data-testid="git-graph-error">
          {state.message}
        </div>
      )}
      {/* 좁은 패널과 최대화가 같은 틀을 쓴다. 최대화를 바꿔도 그래프의 스크롤과 찾기가 남는다. */}
      <div className="git-graph-columns">
        {maximized && state.phase === "loaded" && state.repository && <RefList refs={state.refs} worktrees={state.worktrees} selected={selected} onSelect={select} />}
        <div className="git-graph-main">
          {repositoryCwd !== null && <OperationLogView log={log} cwd={repositoryCwd} busyReason={busyReason} />}
          {state.phase === "loaded" && !state.repository && <p className="muted work-section">이 폴더는 git 저장소가 아닙니다.</p>}
          {state.phase === "loaded" && state.repository && state.rows.length === 0 && (
            <>
              {uncommitted}
              <p className="muted work-section">이 저장소에는 아직 커밋이 없습니다.</p>
            </>
          )}
          {state.phase === "loaded" && state.repository && state.rows.length > 0 && (
            <GraphRows
              graph={{ ...graph, state }}
              folders={folders}
              selected={selected}
              preview={preview}
              top={uncommitted}
              onSelect={select}
              onOpenThread={onOpenThread}
            />
          )}
          {selected !== null && (
            <CommitDetail
              commit={selected}
              state={detail}
              actions={state.phase === "loaded" && cwd && <CommitActionsView key={selected} request={request} cwd={cwd} commit={selected} refs={state.refs} head={state.head} worktrees={state.worktrees} busyReason={busyReason} log={log} onPreview={setPreview} />}
              onSelect={select}
              onClose={() => setSelection(null)}
            />
          )}
        </div>
      </div>
    </aside>
  );
}
