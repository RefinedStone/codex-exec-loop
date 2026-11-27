// 열린 스레드 cwd 의 작업 트리 변경이다 (R1). main 이 git 으로 바뀐 파일 목록을 읽고, 파일 diff 는 펼칠 때 하나씩
// 읽는다. 목록은 스레드를 열 때와 턴이 끝날 때 읽는다. 위 막대의 변경 줄 수와 오른쪽 `변경` 패널이 함께 쓴다 (R10).
import { useEffect, useRef, useState } from "react";

import type { AkraBridge, GitChangedFile, GitFileAction, GitFileDiff, GitDiffSource, GitDiffSnapshot, GitStatusResult } from "../../shared/bridge";
import { isConflictStatus, stageState } from "../../shared/gitChange";
import { Icon, type IconName } from "../design/icons";
import { useDesktopStore } from "../session/desktopStore";
import { toast } from "../session/uiStore";
import { type ActionLock, BranchCommits, type BranchState } from "./BranchCommits";
import { type DiffFile, diffFileFromChange, parseGitDiff } from "./diff";
import { DiffBody, DiffCounts, DiffStyleToggle } from "./DiffView";
import { HunkChanges } from "./HunkChanges";
import { filterChangedFiles } from "./workingTreeNavigation";

/** 변경 패널이 쓰는 브리지 함수이다. 읽기와 파일별 동작이다. */
type GitAccess = Pick<AkraBridge, "gitStatus" | "gitFileDiff" | "gitFileAction" | "gitOpenFile" | "gitBranchSummary" | "gitCommitDiff" | "gitPush" | "gitCreatePullRequest" | "gitHunks" | "gitHunkAction">;

// 동작 하나가 끝날 때까지 패널의 모든 동작 버튼을 막는다 (`ActionLock`). git 은 저장소마다 한 번에 하나만 쓸 수 있다 (`index.lock`).

type PanelState = { phase: "idle" } | { phase: "loading" } | { phase: "loaded"; result: GitStatusResult } | { phase: "failed"; message: string };

type FileState = { phase: "loading" } | { phase: "loaded"; diff: DiffFile | null; note: string | null; snapshot?: GitDiffSnapshot; source?: GitDiffSource } | { phase: "failed"; message: string };

const STATUS_LABEL: Record<string, string> = { M: "수정", A: "추가", D: "삭제", R: "이동", C: "복사", U: "충돌", "?": "새 파일" };

function statusLabel(file: GitChangedFile): string {
  const letter = [...file.status].find((character) => character !== " ") ?? "?";
  return STATUS_LABEL[letter] ?? file.status;
}

function fileFromDiff(diff: GitFileDiff, path: string): { diff: DiffFile | null; note: string | null } {
  if (diff.kind === "binary") {
    return { diff: null, note: "바이너리 파일입니다." };
  }
  const note = diff.truncated ? "1 MB 까지만 보여 줍니다." : null;
  if (diff.kind === "content") {
    return { diff: diffFileFromChange({ path, kind: { type: "add" }, diff: diff.text }), note };
  }
  const file = parseGitDiff(diff.text).find((candidate) => candidate.path === path || candidate.movePath === path);
  const hasLines = file !== undefined && file.lines.length > 0;
  return { diff: hasLines ? file : null, note: hasLines ? note : (note ?? "줄 단위 변경이 없습니다 (모드나 바이너리 변경).") };
}

const ACTION_LABEL: Record<GitFileAction, string> = { stage: "스테이지", unstage: "스테이지 취소", discard: "변경 버리기" };
const ACTION_ICON: Record<GitFileAction, IconName> = { stage: "plus", unstage: "minus", discard: "undo" };

/** 파일 줄의 동작 버튼이다. 줄(`summary`) 안에 있으므로 누를 때 줄이 펼쳐지지 않게 한다. */
function FileActions({ cwd, file, git, lock, onChanged }: { cwd: string; file: GitChangedFile; git: GitAccess; lock: ActionLock; onChanged(): void }) {
  const { busy, setBusy } = lock;
  const { staged, unstaged } = stageState(file);
  // 충돌 상태의 파일은 여기서 스테이지하거나 버리지 않는다. main 도 거절한다.
  const conflict = isConflictStatus(file.status);
  const run = (action: GitFileAction) => {
    setBusy(true);
    git
      .gitFileAction(cwd, action, file.path)
      .then(
        (result) => {
          if (result.ok) onChanged();
          else if (!result.cancelled) toast(`${ACTION_LABEL[action]} 실패: ${result.message}`);
        },
        (error: unknown) => toast(`${ACTION_LABEL[action]} 실패: ${error instanceof Error ? error.message : String(error)}`),
      )
      .finally(() => setBusy(false));
  };
  const button = (action: GitFileAction) => (
    <button
      type="button"
      className="icon-button"
      title={ACTION_LABEL[action]}
      aria-label={ACTION_LABEL[action]}
      disabled={busy}
      onClick={(event) => {
        event.preventDefault();
        run(action);
      }}
      data-testid={`git-${action}`}
    >
      <Icon name={ACTION_ICON[action]} size={13} />
    </button>
  );
  return (
    <span className="file-actions">
      {!file.directory && (
        <button
          type="button"
          className="icon-button"
          title="편집기로 열기"
          aria-label="편집기로 열기"
          onClick={(event) => {
            event.preventDefault();
            git.gitOpenFile(cwd, file.path).catch((error: unknown) => toast(`열지 못했습니다: ${error instanceof Error ? error.message : String(error)}`));
          }}
          data-testid="git-open"
        >
          <Icon name="pencil" size={13} />
        </button>
      )}
      {!conflict && !file.directory && button("discard")}
      {!conflict && staged && button("unstage")}
      {!conflict && unstaged && button("stage")}
    </span>
  );
}

const SOURCE_LABELS: Record<GitDiffSource, string> = {
  head_worktree: "HEAD ↔ 작업 트리",
  head_index: "HEAD ↔ 스테이지",
  index_worktree: "스테이지 ↔ 작업 트리",
  untracked: "추적하지 않는 새 파일",
};

function ChangedFile({ cwd, file, git, lock, onChanged, open, onOpenChange, source }: {
  cwd: string; file: GitChangedFile; git: GitAccess; lock: ActionLock; onChanged(): void;
  open: boolean; onOpenChange(open: boolean): void; source: Exclude<GitDiffSource, "untracked">;
}) {
  const [state, setState] = useState<FileState>({ phase: "loading" });
  const comparison = file.untracked ? "untracked" : source;
  const conflict = isConflictStatus(file.status);
  useEffect(() => {
    if (!open || file.directory) return;
    let active = true;
    setState({ phase: "loading" });
    git.gitFileDiff(cwd, file.path, file.untracked, conflict ? undefined : comparison).then(
      (diff) => { if (active) setState({ phase: "loaded", ...fileFromDiff(diff, file.path), snapshot: diff.snapshot, source: comparison }); },
      (error: unknown) => { if (active) setState({ phase: "failed", message: error instanceof Error ? error.message : String(error) }); },
    );
    return () => { active = false; };
  }, [cwd, file.path, file.untracked, file.directory, open, comparison, conflict, git]);

  const summary = (
    <summary>
      <span className={`diff-kind git-status-${file.untracked ? "new" : "changed"}`}>{statusLabel(file)}</span>
      {file.originalPath && (
        <>
          <code>{file.originalPath}</code>
          {" → "}
        </>
      )}
      <code title={file.path}>{file.path}</code>
      {isConflictStatus(file.status) ? <span className="badge badge-bad">충돌</span> : stageState(file).staged && <span className="badge badge-ok">스테이지</span>}
      {file.added !== null && file.removed !== null && <DiffCounts added={file.added} removed={file.removed} />}
      <FileActions cwd={cwd} file={file} git={git} lock={lock} onChanged={onChanged} />
    </summary>
  );
  if (file.directory) {
    return (
      <details className="diff-file" data-testid="git-file" open={open} onToggle={(event) => onOpenChange(event.currentTarget.open)}>
        {summary}
        <div className="muted diff-hint">새 폴더입니다. 안의 파일은 펼치지 않습니다.</div>
      </details>
    );
  }
  return (
    <details className="diff-file" data-testid="git-file" open={open} onToggle={(event) => onOpenChange(event.currentTarget.open)}>
      {summary}
      {open && state.phase === "loading" && <div className="muted diff-hint">읽는 중…</div>}
      {open && state.phase === "failed" && <div className="error diff-hint">{state.message}</div>}
      {open && state.phase === "loaded" && (() => {
        if (state.source !== comparison) return <div className="muted diff-hint">읽는 중…</div>;
        const content = <>
          {state.snapshot && <div className="muted diff-hint" data-testid="git-diff-snapshot">
            {SOURCE_LABELS[state.snapshot.source]} · {state.snapshot.headOid?.slice(0, 8) ?? "첫 커밋 전"} · snapshot {state.snapshot.snapshotId.slice(0, 12)}
            {state.snapshot.contentStatus === "patch_only" && " · 변경 문맥만 제공"}
            {state.snapshot.contentStatus === "truncated" && " · 부분 표시"}
          </div>}
          {conflict && <div className="muted diff-hint">충돌 파일 · 기본 비교</div>}
          {state.diff && <DiffBody key={state.snapshot?.snapshotId ?? file.path} file={state.diff} />}
          {state.note && <div className="muted diff-hint">{state.note}</div>}
        </>;
        return comparison === "head_worktree" && !conflict
          ? <HunkChanges cwd={cwd} path={file.path} git={git} lock={lock} onChanged={onChanged}>{content}</HunkChanges>
          : content;
      })()}
    </details>
  );
}

export interface WorkingTree {
  cwd: string | null;
  panel: PanelState;
  /** 브랜치의 커밋과 PR 이다. 작업 트리 목록과 같은 때에 읽는다. */
  branch: BranchState;
  /** 다시 읽을 때마다 늘어난다. 파일 diff 도 새로 읽도록 목록 key 로 쓴다. */
  generation: number;
  /** 다시 읽는다. `fresh` 가 false 이면 잠깐 저장해 둔 PR 을 쓴다 (파일 동작 뒤). 기본은 PR 도 다시 묻는다. */
  refresh(fresh?: boolean): void;
}

/** 열린 스레드 폴더의 작업 트리를 읽는다. 폴더가 바뀔 때와 턴이 끝날 때 다시 읽는다. */
export function useWorkingTree(git: GitAccess): WorkingTree {
  const cwd = useDesktopStore((state) => state.transcript?.cwd ?? null);
  const lastTurnStatus = useDesktopStore((state) => state.transcript?.turns.at(-1)?.status ?? null);
  const [panel, setPanel] = useState<PanelState>({ phase: "idle" });
  const [branch, setBranch] = useState<BranchState>({ phase: "idle" });
  /** 늦게 끝난 이전 요청이 새 결과를 덮지 않게 한다. */
  const requestCounter = useRef(0);

  const refresh = (target: string, fresh: boolean) => {
    const request = ++requestCounter.current;
    setPanel({ phase: "loading" });
    git.gitStatus(target).then(
      (result) => {
        if (request === requestCounter.current) setPanel({ phase: "loaded", result });
      },
      (error: unknown) => {
        if (request === requestCounter.current) {
          setPanel({ phase: "failed", message: error instanceof Error ? error.message : String(error) });
        }
      },
    );
    // PR 을 읽는 gh 는 느릴 수 있다. 그동안 이전 커밋 목록을 그대로 보여 준다.
    setBranch((previous) => (previous.phase === "loaded" ? previous : { phase: "loading" }));
    git.gitBranchSummary(target, fresh).then(
      (summary) => {
        if (request === requestCounter.current) setBranch({ phase: "loaded", summary });
      },
      (error: unknown) => {
        if (request === requestCounter.current) {
          setBranch({ phase: "failed", message: error instanceof Error ? error.message : String(error) });
        }
      },
    );
  };

  // 다른 폴더의 스레드로 바뀌면 이전 결과를 버리고 새 폴더를 읽는다.
  useEffect(() => {
    requestCounter.current += 1;
    setPanel({ phase: "idle" });
    setBranch({ phase: "idle" });
    if (cwd) {
      refresh(cwd, false);
    }
  }, [cwd]); // 폴더가 바뀔 때만 다시 읽는다.

  // 턴이 끝나면 에이전트가 바꾼 파일을 반영해 다시 읽는다.
  const previousStatus = useRef(lastTurnStatus);
  useEffect(() => {
    const finished = previousStatus.current === "inProgress" && lastTurnStatus !== "inProgress";
    previousStatus.current = lastTurnStatus;
    // 에이전트가 턴 안에서 push 하거나 PR 을 만들었을 수 있다. PR 도 다시 묻는다.
    if (finished && cwd) {
      refresh(cwd, true);
    }
  }, [lastTurnStatus]); // 턴 상태가 바뀔 때만 본다.

  return { cwd, panel, branch, generation: requestCounter.current, refresh: (fresh = true) => cwd && refresh(cwd, fresh) };
}

/** 줄 수를 아는 파일의 더한 줄과 지운 줄의 합이다. 아직 읽지 못했으면 null 이다. */
export function workingTreeCounts(tree: WorkingTree): { branch: string | null; added: number; removed: number } | null {
  if (tree.panel.phase !== "loaded" || !tree.panel.result.ok) return null;
  const { branch, files } = tree.panel.result;
  return { branch, added: files.reduce((sum, file) => sum + (file.added ?? 0), 0), removed: files.reduce((sum, file) => sum + (file.removed ?? 0), 0) };
}

function WorkingTreeFiles({ files, cwd, git, lock, onChanged }: {
  files: GitChangedFile[]; cwd: string; git: GitAccess; lock: ActionLock; onChanged(): void;
}) {
  const [source, setSource] = useState<Exclude<GitDiffSource, "untracked">>("head_worktree");
  const [query, setQuery] = useState("");
  const [opened, setOpened] = useState<Set<string>>(new Set());
  const [selected, setSelected] = useState<string | null>(null);
  const list = useRef<HTMLDivElement>(null);
  const visible = filterChangedFiles(files, query);
  const navigable = visible.filter((file) => !file.directory);
  const move = (step: number) => {
    const current = navigable.findIndex((file) => file.path === selected);
    const next = navigable[current < 0 ? (step > 0 ? 0 : navigable.length - 1) : current + step];
    if (!next) return;
    setSelected(next.path);
    setOpened((previous) => new Set([...previous, next.path]));
    requestAnimationFrame(() => {
      const element = Array.from(list.current?.querySelectorAll<HTMLElement>("[data-git-path]") ?? []).find((item) => item.dataset.gitPath === next.path);
      element?.scrollIntoView({ block: "nearest" });
      element?.querySelector("summary")?.focus();
    });
  };
  const counts = files.filter((file) => file.added !== null && file.removed !== null);
  return <>
    <div className="review-toolbar" style={{ flexWrap: "wrap" }}>
      <label>비교 <select value={source} onChange={(event) => setSource(event.target.value as Exclude<GitDiffSource, "untracked">)}>
        <option value="head_worktree">{SOURCE_LABELS.head_worktree}</option>
        <option value="head_index">{SOURCE_LABELS.head_index}</option>
        <option value="index_worktree">{SOURCE_LABELS.index_worktree}</option>
      </select></label>
      <span className="muted">HEAD 기준 확인된 합계 <DiffCounts added={counts.reduce((sum, file) => sum + (file.added ?? 0), 0)} removed={counts.reduce((sum, file) => sum + (file.removed ?? 0), 0)} /> · {counts.length}/{files.length}개</span>
      <input aria-label="변경 파일 경로 찾기" placeholder="파일 경로 찾기" value={query} onChange={(event) => { setQuery(event.target.value); setSelected(null); }} />
      <button type="button" className="button" onClick={() => move(-1)} disabled={navigable.length === 0}>이전 파일</button>
      <button type="button" className="button" onClick={() => move(1)} disabled={navigable.length === 0}>다음 파일</button>
      <button type="button" className="button" onClick={() => setOpened(new Set())}>전체 접기</button>
      <span className="muted">{visible.length}/{files.length}개 · 경로만 검색</span>
    </div>
    <div className="working-tree-files diff-file-list" ref={list}>
      {visible.map((file) => <div key={file.status + ":" + file.path} data-git-path={file.path}>
        <ChangedFile cwd={cwd} file={file} git={git} lock={lock} onChanged={onChanged}
          open={opened.has(file.path)} source={source}
          onOpenChange={(open) => { if (open) setSelected(file.path); setOpened((previous) => {
            if (previous.has(file.path) === open) return previous;
            const next = new Set(previous); if (open) next.add(file.path); else next.delete(file.path); return next;
          }); }} />
      </div>)}
    </div>
  </>;
}

export function WorkingTreePanel({ tree, git, onOpenUrl, children }: {
  tree: WorkingTree; git: GitAccess; onOpenUrl(url: string): void; children?: React.ReactNode;
}) {
  const { cwd, panel } = tree;
  const [busy, setBusy] = useState(false);
  if (!cwd) {
    return <p className="muted empty">스레드를 열면 그 폴더의 변경을 보여 줍니다.</p>;
  }
  const result = panel.phase === "loaded" ? panel.result : null;
  const failure = panel.phase === "failed" ? panel.message : result && !result.ok ? result.message : null;
  const files = result?.ok ? result.files : null;
  return (
    <section className="review working-tree" data-testid="working-tree">
      <div className="review-toolbar">
        {result?.ok && (
          <span className="muted" data-testid="working-tree-summary">
            <Icon name="branch" size={12} /> {result.branch ?? "브랜치 없음"} · 파일 {result.files.length}개{result.truncated ? " (목록이 길어 일부만)" : ""}
          </span>
        )}
        <span className="bar-spacer" />
        <DiffStyleToggle />
        <button type="button" className="button" onClick={() => tree.refresh()} disabled={panel.phase === "loading"}>
          {panel.phase === "loading" ? "읽는 중…" : "새로 고침"}
        </button>
      </div>
      {failure && (
        <div className="error diff-hint" data-testid="working-tree-error">
          작업 트리를 읽을 수 없습니다. {failure}
        </div>
      )}
      {files && files.length === 0 && <div className="muted diff-hint">바뀐 파일이 없습니다.</div>}
      {files && files.length > 0 && (
        <WorkingTreeFiles key={tree.generation} files={files} cwd={cwd} git={git} lock={{ busy, setBusy }} onChanged={() => tree.refresh(false)} />
      )}
      <BranchCommits cwd={cwd} state={tree.branch} git={git} lock={{ busy, setBusy }} onChanged={() => tree.refresh()} onOpenUrl={onOpenUrl} />
      {children}
    </section>
  );
}
