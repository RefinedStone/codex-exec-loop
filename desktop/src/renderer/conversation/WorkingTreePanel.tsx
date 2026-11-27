// 열린 스레드 cwd 의 작업 트리 변경이다 (R1). main 이 git 으로 바뀐 파일 목록을 읽고, 파일 diff 는 펼칠 때 하나씩
// 읽는다. 목록은 스레드를 열 때와 턴이 끝날 때 읽는다. 위 막대의 변경 줄 수와 오른쪽 `변경` 패널이 함께 쓴다 (R10).
import { useEffect, useRef, useState } from "react";

import type { GitChangedFile, GitDiffSource, GitStatusResult } from "../../shared/bridge";
import { isConflictStatus, stageState } from "../../shared/gitChange";
import { Icon } from "../design/icons";
import { useDesktopStore } from "../session/desktopStore";
import { toast } from "../session/uiStore";
import { type ActionLock, BranchCommits, type BranchState } from "./BranchCommits";
import { type DiffFile, diffFileFromChange, parseGitDiff } from "./diff";
import { DiffCounts, DiffStyleToggle } from "./DiffView";
import { ChangedFile, SOURCE_LABELS, type GitAccess } from "./WorkingTreeFile";
import { filterChangedFiles } from "./workingTreeNavigation";

type PanelState = { phase: "idle" } | { phase: "loading" } | { phase: "loaded"; result: GitStatusResult } | { phase: "failed"; message: string };

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
  const [source, setSource] = useState<Exclude<GitDiffSource, "untracked" | "worktree_file">>("head_worktree");
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
    setOpened(new Set([next.path]));
    requestAnimationFrame(() => {
      const element = Array.from(list.current?.querySelectorAll<HTMLElement>("[data-git-path]") ?? []).find((item) => item.dataset.gitPath === next.path);
      element?.scrollIntoView({ block: "nearest" });
      element?.querySelector("summary")?.focus();
    });
  };
  const counts = files.filter((file) => file.added !== null && file.removed !== null);
  return <>
    <div className="review-toolbar working-tree-navigation" style={{ flexWrap: "wrap" }}>
      <label>비교 <select aria-label="비교" value={source} onChange={(event) => {
        const value = event.target.value;
        if (value === "head_worktree" || value === "head_index" || value === "index_worktree") setSource(value);
      }}>
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
