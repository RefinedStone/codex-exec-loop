import { useEffect, useState } from "react";
import type { AkraBridge, GitChangedFile, GitFileAction, GitFileDiff, GitDiffSource, GitDiffSnapshot } from "../../shared/bridge";
import { isConflictStatus, stageState } from "../../shared/gitChange";
import { Icon, type IconName } from "../design/icons";
import { toast } from "../session/uiStore";
import type { ActionLock } from "./BranchCommits";
import { type DiffFile, diffFileFromChange, parseGitDiff } from "./diff";
import { DiffBody, DiffCounts } from "./DiffView";
import { HunkChanges } from "./HunkChanges";
import { SnapshotDiffView } from "./SnapshotDiffView";

export type GitAccess = Pick<AkraBridge, "gitStatus" | "gitFileDiff" | "gitDiffContext" | "gitFileAction" | "gitOpenFile" | "gitBranchSummary" | "gitCommitDiff" | "gitPush" | "gitCreatePullRequest" | "gitHunks" | "gitHunkAction">;

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

export const SOURCE_LABELS: Record<GitDiffSource, string> = {
  worktree_file: "작업 파일",
  head_worktree: "HEAD ↔ 작업 트리",
  head_index: "HEAD ↔ 스테이지",
  index_worktree: "스테이지 ↔ 작업 트리",
  untracked: "추적하지 않는 새 파일",
};

export function ChangedFile({ cwd, file, git, lock, onChanged, open, onOpenChange, source }: {
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
            {state.snapshot.contentStatus === "patch_only" && (state.snapshot.contents ? " · patch 먼저 표시 · 원문 범위 조회" : " · 변경 문맥만 제공")}
            {state.snapshot.contentStatus === "truncated" && " · 부분 표시"}
          </div>}
          {conflict && <div className="muted diff-hint">충돌 파일 · 기본 비교</div>}
          {state.diff && (state.snapshot
            ? <SnapshotDiffView key={state.snapshot.snapshotId} cwd={cwd} file={state.diff} snapshot={state.snapshot} git={git} />
            : <DiffBody key={file.path} file={state.diff} />)}
          {state.note && <div className="muted diff-hint">{state.note}</div>}
        </>;
        return comparison === "head_worktree" && !conflict
          ? <>{content}<details className="diff-file"><summary>스테이지 전 부분 변경 작업</summary>
            <HunkChanges cwd={cwd} path={file.path} git={git} lock={lock} onChanged={onChanged}>{null}</HunkChanges>
          </details></>
          : content;
      })()}
    </details>
  );
}
