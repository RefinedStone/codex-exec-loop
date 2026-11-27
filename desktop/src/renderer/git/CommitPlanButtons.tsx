import { useLayoutEffect, useRef, useState } from "react";
import { command } from "../../shared/commands";
import type { CommitActions, GitOperation } from "./gitOperations";
import { SHOW_COMMIT_ACTIONS } from "./openCommitActions";

interface Props {
  readonly picks: readonly string[];
  readonly commit: string;
  readonly running: boolean;
  readonly actions: CommitActions;
  plan(operation: GitOperation, branch: string | null): void;
}

export function CommitPlanButtons({ picks, commit, running, actions, plan }: Props) {
  const [choosing, setChoosing] = useState(false);
  const dialog = useRef<HTMLDialogElement>(null);
  useLayoutEffect(() => {
    const show = (event: Event) => {
      event.preventDefault();
      setChoosing(true);
    };
    window.addEventListener(SHOW_COMMIT_ACTIONS, show);
    return () => window.removeEventListener(SHOW_COMMIT_ACTIONS, show);
  }, []);
  useLayoutEffect(() => {
    if (choosing) dialog.current?.showModal();
  }, [choosing]);
  const choose = (operation: GitOperation, branch: string | null) => {
    setChoosing(false);
    plan(operation, branch);
  };
  const { reset, checkoutCommit } = actions;
  const controls = (
      <div className="git-action-buttons">
        <button type="button" disabled={running || picks.length === 0 || picks.length > 32} onClick={() => choose({ kind: "cherry_pick", commits: picks }, null)}>cherry-pick 계획 ({picks.length})</button>
        {reset && (
          <button type="button" disabled={running} onClick={() => choose(reset.operation, reset.branch)} data-testid="git-action-reset">
            여기로 reset ({reset.branch})
          </button>
        )}
        {actions.checkoutBranches.map(({ operation, branch }) => (
          <button key={branch} type="button" disabled={running} onClick={() => choose(operation, null)} data-testid="git-action-checkout-branch">
            체크아웃: {branch}
          </button>
        ))}
        {checkoutCommit && (
          <button type="button" disabled={running} onClick={() => choose(checkoutCommit, null)} data-testid="git-action-checkout-commit">
            이 커밋 체크아웃
          </button>
        )}
        {actions.movableBranches.length > 0 && (
          <select
            value=""
            disabled={running}
            aria-label="이 커밋으로 옮길 브랜치"
            onChange={(event) => event.target.value && choose({ kind: "move_branch", branch: event.target.value, target: commit }, null)}
            data-testid="git-action-move"
          >
            <option value="">브랜치를 여기로 옮기기…</option>
            {actions.movableBranches.map((branch) => (
              <option key={branch} value={branch}>
                {branch}
              </option>
            ))}
          </select>
        )}
      </div>
  );
  if (!choosing) return controls;
  return (
    <dialog className="dialog" aria-label={command("graph-actions").title} ref={dialog} onCancel={(event) => { event.preventDefault(); setChoosing(false); }}>
      <header><strong>{command("graph-actions").title}</strong><code>{commit.slice(0, 8)}</code></header>
      {controls}
      <button type="button" onClick={() => setChoosing(false)}>닫기</button>
    </dialog>
  );
}
