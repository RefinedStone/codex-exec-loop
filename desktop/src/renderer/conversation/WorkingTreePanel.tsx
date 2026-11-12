// 열린 스레드 cwd 의 작업 트리 변경이다 (R1). main 이 git 으로 바뀐 파일 목록을 읽고, 파일 diff 는 펼칠 때 하나씩
// 읽는다. 목록은 스레드를 열 때와 턴이 끝날 때 읽는다. 위 막대의 변경 줄 수와 오른쪽 `변경` 패널이 함께 쓴다 (R10).
import { useEffect, useRef, useState } from "react";

import type { AkraBridge, GitChangedFile, GitFileDiff, GitStatusResult } from "../../shared/bridge";
import { Icon } from "../design/icons";
import { useDesktopStore } from "../session/desktopStore";
import { type DiffLine, diffFileFromChange, parseGitDiff } from "./diff";
import { DiffCounts, DiffLines } from "./DiffView";

type GitReader = Pick<AkraBridge, "gitStatus" | "gitFileDiff">;

type PanelState = { phase: "idle" } | { phase: "loading" } | { phase: "loaded"; result: GitStatusResult } | { phase: "failed"; message: string };

type FileState = { phase: "loading" } | { phase: "loaded"; lines: DiffLine[]; note: string | null } | { phase: "failed"; message: string };

const STATUS_LABEL: Record<string, string> = { M: "수정", A: "추가", D: "삭제", R: "이동", C: "복사", U: "충돌", "?": "새 파일" };

function statusLabel(file: GitChangedFile): string {
  const letter = [...file.status].find((character) => character !== " ") ?? "?";
  return STATUS_LABEL[letter] ?? file.status;
}

function linesFromDiff(diff: GitFileDiff, path: string): { lines: DiffLine[]; note: string | null } {
  if (diff.kind === "binary") {
    return { lines: [], note: "바이너리 파일입니다." };
  }
  const note = diff.truncated ? "1 MB 까지만 보여 줍니다." : null;
  if (diff.kind === "content") {
    return { lines: diffFileFromChange({ path, kind: { type: "add" }, diff: diff.text }).lines, note };
  }
  const file = parseGitDiff(diff.text)[0];
  return { lines: file?.lines ?? [], note: file && file.lines.length > 0 ? note : (note ?? "줄 단위 변경이 없습니다 (모드나 바이너리 변경).") };
}

function ChangedFile({ cwd, file, git }: { cwd: string; file: GitChangedFile; git: GitReader }) {
  const [open, setOpen] = useState(false);
  const [state, setState] = useState<FileState>({ phase: "loading" });

  const toggle = (next: boolean) => {
    setOpen(next);
    if (next && state.phase !== "loaded") {
      setState({ phase: "loading" });
      git.gitFileDiff(cwd, file.path, file.untracked).then(
        (diff) => setState({ phase: "loaded", ...linesFromDiff(diff, file.path) }),
        (error: unknown) => setState({ phase: "failed", message: error instanceof Error ? error.message : String(error) }),
      );
    }
  };

  const summary = (
    <summary>
      <span className={`diff-kind git-status-${file.untracked ? "new" : "changed"}`}>{statusLabel(file)}</span>
      {file.originalPath && (
        <>
          <code>{file.originalPath}</code>
          {" → "}
        </>
      )}
      <code>{file.path}</code>
      {file.added !== null && file.removed !== null && <DiffCounts added={file.added} removed={file.removed} />}
    </summary>
  );
  if (file.directory) {
    return (
      <details className="diff-file" data-testid="git-file">
        {summary}
        <div className="muted diff-hint">새 폴더입니다. 안의 파일은 펼치지 않습니다.</div>
      </details>
    );
  }
  return (
    <details className="diff-file" data-testid="git-file" open={open} onToggle={(event) => toggle(event.currentTarget.open)}>
      {summary}
      {open && state.phase === "loading" && <div className="muted diff-hint">읽는 중…</div>}
      {open && state.phase === "failed" && <div className="error diff-hint">{state.message}</div>}
      {open && state.phase === "loaded" && (
        <>
          {state.lines.length > 0 && <DiffLines lines={state.lines} />}
          {state.note && <div className="muted diff-hint">{state.note}</div>}
        </>
      )}
    </details>
  );
}

export interface WorkingTree {
  cwd: string | null;
  panel: PanelState;
  /** 다시 읽을 때마다 늘어난다. 파일 diff 도 새로 읽도록 목록 key 로 쓴다. */
  generation: number;
  refresh(): void;
}

/** 열린 스레드 폴더의 작업 트리를 읽는다. 폴더가 바뀔 때와 턴이 끝날 때 다시 읽는다. */
export function useWorkingTree(git: GitReader): WorkingTree {
  const cwd = useDesktopStore((state) => state.transcript?.cwd ?? null);
  const lastTurnStatus = useDesktopStore((state) => state.transcript?.turns.at(-1)?.status ?? null);
  const [panel, setPanel] = useState<PanelState>({ phase: "idle" });
  /** 늦게 끝난 이전 요청이 새 결과를 덮지 않게 한다. */
  const requestCounter = useRef(0);

  const refresh = (target: string) => {
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
  };

  // 다른 폴더의 스레드로 바뀌면 이전 결과를 버리고 새 폴더를 읽는다.
  useEffect(() => {
    requestCounter.current += 1;
    setPanel({ phase: "idle" });
    if (cwd) {
      refresh(cwd);
    }
  }, [cwd]); // 폴더가 바뀔 때만 다시 읽는다.

  // 턴이 끝나면 에이전트가 바꾼 파일을 반영해 다시 읽는다.
  const previousStatus = useRef(lastTurnStatus);
  useEffect(() => {
    const finished = previousStatus.current === "inProgress" && lastTurnStatus !== "inProgress";
    previousStatus.current = lastTurnStatus;
    if (finished && cwd) {
      refresh(cwd);
    }
  }, [lastTurnStatus]); // 턴 상태가 바뀔 때만 본다.

  return { cwd, panel, generation: requestCounter.current, refresh: () => cwd && refresh(cwd) };
}

/** 줄 수를 아는 파일의 더한 줄과 지운 줄의 합이다. 아직 읽지 못했으면 null 이다. */
export function workingTreeCounts(tree: WorkingTree): { branch: string | null; added: number; removed: number } | null {
  if (tree.panel.phase !== "loaded" || !tree.panel.result.ok) return null;
  const { branch, files } = tree.panel.result;
  return { branch, added: files.reduce((sum, file) => sum + (file.added ?? 0), 0), removed: files.reduce((sum, file) => sum + (file.removed ?? 0), 0) };
}

export function WorkingTreePanel({ tree, git }: { tree: WorkingTree; git: GitReader }) {
  const { cwd, panel } = tree;
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
        <button type="button" className="button" onClick={tree.refresh} disabled={panel.phase === "loading"}>
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
        <div className="working-tree-files diff-file-list" key={tree.generation}>
          {files.map((file) => (
            <ChangedFile key={`${file.status}:${file.path}`} cwd={cwd} file={file} git={git} />
          ))}
        </div>
      )}
    </section>
  );
}
