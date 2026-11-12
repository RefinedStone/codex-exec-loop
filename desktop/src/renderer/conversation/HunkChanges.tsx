import { useEffect, useState, type ReactNode } from "react";

import type { AkraBridge } from "../../shared/bridge";
import type { GitHunkAction, GitHunkDiff } from "../../shared/gitHunks";
import { toast } from "../session/uiStore";
import type { ActionLock } from "./BranchCommits";
import { diffFileFromChange } from "./diff";
import { DiffLines } from "./DiffView";

type HunkState = { phase: "loading" } | { phase: "loaded"; diff: GitHunkDiff } | { phase: "failed"; message: string };
type HunkAccess = Pick<AkraBridge, "gitHunks" | "gitHunkAction">;

/** 전체 파일 변경과 구분해서, index 이후의 변경만 덩어리별로 조작한다. */
export function HunkChanges({
  cwd, path, git, lock, onChanged, children,
}: {
  cwd: string;
  path: string;
  git: HunkAccess;
  lock: ActionLock;
  onChanged(): void;
  children: ReactNode;
}) {
  const [state, setState] = useState<HunkState>({ phase: "loading" });
  useEffect(() => {
    let active = true;
    git.gitHunks(cwd, path).then(
      (diff) => { if (active) setState({ phase: "loaded", diff }); },
      (error: unknown) => { if (active) setState({ phase: "failed", message: error instanceof Error ? error.message : String(error) }); },
    );
    return () => { active = false; };
  }, [cwd, path, git]);

  const run = (action: GitHunkAction, id: string) => {
    lock.setBusy(true);
    git.gitHunkAction(cwd, action, path, id).then(
      (result) => {
        if (result.ok) onChanged();
        else if (!result.cancelled) toast(result.message);
      },
      (error: unknown) => toast(error instanceof Error ? error.message : String(error)),
    ).finally(() => lock.setBusy(false));
  };

  if (state.phase === "loading") return <div className="muted diff-hint">부분 변경을 읽는 중…</div>;
  if (state.phase === "failed") return <><div className="error diff-hint">{state.message}</div>{children}</>;
  if (state.diff.hunks.length === 0) {
    return <>{state.diff.message && <div className="muted diff-hint">{state.diff.message}</div>}{children}</>;
  }
  return (
    <div data-testid="git-hunks">
      <div className="muted diff-hint">스테이지 전 변경 · {state.diff.hunks.length}개</div>
      {state.diff.hunks.map((hunk, index) => (
        <section key={hunk.id} data-testid="git-hunk">
          <header className="review-toolbar">
            <span className="muted">부분 {index + 1}</span>
            <span className="bar-spacer" />
            <button type="button" className="button" disabled={lock.busy} onClick={() => run("stage", hunk.id)} data-testid="git-hunk-stage">
              이 부분 스테이지
            </button>
            <button type="button" className="button" disabled={lock.busy} onClick={() => run("discard", hunk.id)} data-testid="git-hunk-discard">
              이 부분 버리기
            </button>
          </header>
          <DiffLines lines={diffFileFromChange({ path, kind: { type: "update", move_path: null }, diff: hunk.text }).lines} />
        </section>
      ))}
      <details className="diff-file">
        <summary>전체 변경 (스테이지 포함)</summary>
        {children}
      </details>
    </div>
  );
}
