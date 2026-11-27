// git 작업의 화면이다 (plan 27 G3): 고른 커밋의 작업 단추, 계획, 결과 알림, 작업 기록.
// 단추는 계획을 읽는다. 저장소는 "실행"을 눌러야 바뀐다. Gateway 는 계획 뒤에 저장소가 바뀌었으면 실행하지 않고 새
// 계획을 준다. 끝난 작업은 알림과 작업 기록에서 되돌린다.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { GatewayRequest } from "../work/notionHub";
import { fullTime, type GitRef, type GitWorktree } from "./gitGraph";
import {
  blockerText,
  canUndo,
  commitActions,
  executePlan,
  fetchOperations,
  fetchPlan,
  type GitOperation,
  type GitOperationOutcome,
  type GitOperationPlan,
  type GitOperationRecord,
  noticeFrom,
  type OperationNotice,
  operationTitle,
  planSummary,
  recordStatus,
  recordTitle,
  undoOperation,
} from "./gitOperations";

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export interface GitOperationLog {
  records: GitOperationRecord[];
  notice: OperationNotice | null;
  /** 되돌리기나 실행이 도는 중이다. */
  working: boolean;
  /** 실행의 결과를 받는다: 알림을 쓰고, 기록을 다시 읽고, 그래프를 다시 읽게 한다. */
  finished(outcome: GitOperationOutcome, undo?: boolean): void;
  undo(opId: string): void;
  dismiss(): void;
}

/** 저장소의 작업 기록과 마지막 결과이다. 폴더가 바뀌면 다시 읽는다. `onChanged` 는 저장소가 바뀌었을 때 부른다. */
export function useGitOperationLog(request: GatewayRequest, cwd: string | null, onChanged: () => void): GitOperationLog {
  const [records, setRecords] = useState<GitOperationRecord[]>([]);
  const [notice, setNotice] = useState<OperationNotice | null>(null);
  const [working, setWorking] = useState(false);
  // 늦게 온 답이 다른 폴더의 화면을 덮지 않게 한다.
  const shownCwd = useRef(cwd);
  shownCwd.current = cwd;

  const reload = useCallback(() => {
    if (!cwd) return;
    // 기록을 읽지 못해도 그래프는 쓸 수 있다. 옛 Gateway 에는 이 메서드가 없다.
    fetchOperations(request, cwd).then(
      (loaded) => shownCwd.current === cwd && setRecords(loaded),
      () => undefined,
    );
  }, [request, cwd]);

  useEffect(() => {
    setRecords([]);
    setNotice(null);
    reload();
  }, [reload]);

  const finished = useCallback(
    (outcome: GitOperationOutcome, undo = false) => {
      setNotice(noticeFrom(outcome, undo));
      reload();
      if (outcome.status === "done" || outcome.status === "failed") onChanged();
    },
    [reload, onChanged],
  );

  const undo = useCallback(
    (opId: string) => {
      if (!cwd) return;
      setWorking(true);
      undoOperation(request, cwd, opId)
        .then(
          (outcome) => shownCwd.current === cwd && finished(outcome, true),
          (error: unknown) => shownCwd.current === cwd && setNotice({ text: `되돌리지 못했습니다. ${errorText(error)}`, failed: true, record: null }),
        )
        .finally(() => setWorking(false));
    },
    [request, cwd, finished],
  );

  return { records, notice, working, finished, undo, dismiss: useCallback(() => setNotice(null), []) };
}

/** 마지막 결과의 알림과 작업 기록이다. 패널의 머리 아래에 있다. */
export function OperationLogView({ log, cwd, busyReason }: { log: GitOperationLog; cwd: string; busyReason: string | null }) {
  const { records, notice, working } = log;
  const blocked = working || busyReason !== null;
  if (notice === null && records.length === 0) return null;
  return (
    <div className="git-operations">
      {notice !== null && (
        <div className={`git-operation-notice${notice.failed ? " error" : ""}`} role="status" data-testid="git-operation-notice">
          <span>{notice.text}</span>
          {notice.record !== null && canUndo(notice.record, cwd) && (
            <button type="button" disabled={blocked} title={busyReason ?? undefined} onClick={() => log.undo(notice.record!.opId)} data-testid="git-operation-undo">
              되돌리기
            </button>
          )}
          <button type="button" onClick={log.dismiss} aria-label="알림 닫기">
            닫기
          </button>
        </div>
      )}
      {records.length > 0 && (
        <details className="git-operation-log" data-testid="git-operation-log">
          <summary>작업 기록 {records.length}개</summary>
          <ul>
            {records.map((record) => (
              <li key={record.opId} data-testid="git-operation-entry">
                <span className="git-operation-title" title={record.error ?? record.worktree}>
                  {recordTitle(record)}
                </span>
                <span className="muted">
                  {recordStatus(record)} · {fullTime(record.startedAt)}
                </span>
                {canUndo(record, cwd) && (
                  <button type="button" disabled={blocked} title={busyReason ?? undefined} onClick={() => log.undo(record.opId)}>
                    되돌리기
                  </button>
                )}
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

type PlanState = { phase: "idle" } | { phase: "planning"; title: string } | { phase: "planned"; plan: GitOperationPlan; stale: boolean; running: boolean } | { phase: "failed"; message: string };

export interface CommitActionsProps {
  request: GatewayRequest;
  cwd: string;
  commit: string;
  refs: readonly GitRef[];
  head: string | null;
  worktrees: readonly GitWorktree[];
  /** 지금 실행하면 안 되는 이유이다 (이 스레드의 턴이 실행 중이다). 계획은 볼 수 있다. */
  busyReason: string | null;
  log: GitOperationLog;
  /** 보이는 계획이 바뀌면 부른다. 계획이 닫히면 `null` 이다. 그래프가 결과를 점선으로 그린다. */
  onPreview(plan: GitOperationPlan | null): void;
}

/** 고른 커밋의 작업 단추와 계획이다. 커밋이 바뀌면 부모가 `key` 로 새로 만든다. */
export function CommitActionsView({ request, cwd, commit, refs, head, worktrees, busyReason, log, onPreview }: CommitActionsProps) {
  const actions = useMemo(() => commitActions(commit, refs, head, worktrees), [commit, refs, head, worktrees]);
  const [state, setState] = useState<PlanState>({ phase: "idle" });
  const previewed = state.phase === "planned" ? state.plan : null;
  useEffect(() => {
    onPreview(previewed);
    return () => onPreview(null);
  }, [previewed, onPreview]);
  // 계획을 읽는 사이에 다른 작업을 고르면 앞의 답을 버린다.
  const asked = useRef(0);

  const plan = (operation: GitOperation, branch: string | null) => {
    const current = ++asked.current;
    setState({ phase: "planning", title: operationTitle(operation, branch) });
    fetchPlan(request, cwd, operation).then(
      (loaded) => current === asked.current && setState({ phase: "planned", plan: loaded, stale: false, running: false }),
      (error: unknown) => current === asked.current && setState({ phase: "failed", message: errorText(error) }),
    );
  };

  const run = (planned: GitOperationPlan) => {
    const current = ++asked.current;
    setState({ phase: "planned", plan: planned, stale: false, running: true });
    executePlan(request, cwd, planned).then(
      (outcome) => {
        if (current !== asked.current) return;
        // 저장소가 계획과 다르다. 고치지 않았다. 새 계획을 보이고 다시 정하게 한다.
        if (outcome.status === "stale") {
          setState({ phase: "planned", plan: outcome.plan, stale: true, running: false });
          return;
        }
        setState({ phase: "idle" });
        log.finished(outcome);
      },
      (error: unknown) => current === asked.current && setState({ phase: "failed", message: errorText(error) }),
    );
  };

  // 실행 중에는 다른 작업을 고르지 못한다. 고르면 실행의 답을 버리게 되고, 저장소는 이미 바뀌었다.
  const running = state.phase === "planned" && state.running;
  const nothing = actions.reset === null && actions.checkoutBranches.length === 0 && actions.checkoutCommit === null && actions.movableBranches.length === 0;
  if (nothing) return null;
  return (
    <div className="git-actions">
      <div className="git-action-buttons">
        {actions.reset && (
          <button type="button" disabled={running} onClick={() => plan(actions.reset!.operation, actions.reset!.branch)} data-testid="git-action-reset">
            여기로 reset ({actions.reset.branch})
          </button>
        )}
        {actions.checkoutBranches.map(({ operation, branch }) => (
          <button key={branch} type="button" disabled={running} onClick={() => plan(operation, null)} data-testid="git-action-checkout-branch">
            체크아웃: {branch}
          </button>
        ))}
        {actions.checkoutCommit && (
          <button type="button" disabled={running} onClick={() => plan(actions.checkoutCommit!, null)} data-testid="git-action-checkout-commit">
            이 커밋 체크아웃
          </button>
        )}
        {actions.movableBranches.length > 0 && (
          <select
            value=""
            disabled={running}
            aria-label="이 커밋으로 옮길 브랜치"
            onChange={(event) => event.target.value && plan({ kind: "move_branch", branch: event.target.value, target: commit }, null)}
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
      {state.phase === "planning" && <p className="muted">{state.title}: 계획을 읽는 중…</p>}
      {state.phase === "failed" && (
        <p className="error" data-testid="git-plan-error">
          {state.message}
        </p>
      )}
      {state.phase === "planned" && (
        <div className="git-plan" data-testid="git-plan">
          <strong>{operationTitle(state.plan.operation, state.plan.branch)}</strong>
          {state.stale && <p className="error">저장소가 바뀌어서 실행하지 않았습니다. 아래는 지금 상태의 계획입니다.</p>}
          <ul>
            {planSummary(state.plan).map((line) => (
              <li key={line} data-testid="git-plan-line">
                {line}
              </li>
            ))}
            {state.plan.blockers.map((blocker) => (
              <li key={blocker.code} className="error" title={blocker.message} data-testid="git-plan-blocker">
                {blockerText(blocker)}
              </li>
            ))}
            {state.plan.blockers.length === 0 && busyReason !== null && <li className="error">{busyReason}</li>}
          </ul>
          <div className="git-action-buttons">
            <button type="button" className="primary" disabled={state.running || log.working || busyReason !== null || state.plan.blockers.length > 0} onClick={() => run(state.plan)} data-testid="git-plan-run">
              {state.running ? "실행 중…" : "실행"}
            </button>
            <button type="button" disabled={state.running} onClick={() => setState({ phase: "idle" })} data-testid="git-plan-cancel">
              취소
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
