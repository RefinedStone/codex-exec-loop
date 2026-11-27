// 커밋 그래프 패널이다 (plan 27 G1 – G3). 열린 스레드의 저장소의 커밋을 그래프로 보여 준다. 저장소를 고치는 일은
// 고른 커밋의 작업 단추에서 시작하고 Gateway 의 작업 엔진이 한다 (`GitOperationViews.tsx`).
// 긴 이력은 보이는 줄만 그린다. 끝에 가까워지면 다음 쪽을 읽는다. 줄 높이는 고정이다: 줄마다 그린 선이 위아래 줄과
// 이어져야 한다. 저장소가 바뀌었는지 주기적으로 보고, 바뀌면 보던 자리를 지키며 다시 읽는다. 줄을 고르면 그 커밋의
// 상세를 아래에 보인다. 최대화하면 왼쪽에 ref 목록을 더한다 (plan 27 §7.2). 그래프 위의 상태 띠는 끝나지 않은 git
// 작업, 충돌, 브랜치 없는 HEAD 를 한 줄로 쓴다 (§7.4 의 6). 열린 계획은 결과의 자리를 점선으로 그린다 (§7.4 의 2).
import { useVirtualizer } from "@tanstack/react-virtual";
import { type KeyboardEvent, memo, type ReactNode, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import { copyText } from "../conversation/copyText";
import { isTurnInProgress } from "../conversation/transcript";
import { parseGitDiff } from "../conversation/diff";
import { DiffFileList, DiffStyleToggle } from "../conversation/DiffView";
import { useDesktopStore } from "../session/desktopStore";
import type { GatewayRequest } from "../work/notionHub";
import {
  appendPage,
  type BadgeFolder,
  fetchCommitDetail,
  fetchGraphPage,
  fetchFirstGraphPage,
  fetchGraphRows,
  fetchRefsState,
  findRows,
  fullTime,
  GIT_GRAPH_PAGE_LIMIT_MAX,
  type GitCommitDetail,
  type GitGraphRow,
  type GitGraphState,
  type GitRef,
  type GitRefKind,
  type GitRefLabel,
  type GitThreadBadge,
  headName,
  type LoadedGitGraph,
  loadedFromFirstPage,
  nextMatch,
  refLabels,
  relativeTime,
  scrollTopAfterRowsChange,
  shortSha,
  threadBadges,
  threadIdsInWorktreeOf,
  unfinishedWork,
  type GitWorktree,
  type GitWorktreeProgress,
} from "./gitGraph";
import { CommitActionsView, OperationLogView, useGitOperationLog } from "./GitOperationViews";
import { type GitOperationPlan, operationTitle } from "./gitOperations";
import { graphWidth, laneColorIndex, laneX, NODE_RADIUS, ROW_HEIGHT, rowPaths } from "./graphGeometry";

/** 읽은 줄의 끝에서 이만큼 앞에 오면 다음 쪽을 읽는다. */
const LOAD_AHEAD_ROWS = 60;
/** 저장소가 바뀌었는지 보는 주기이다. 패널이 열려 있고 창이 보일 때만 본다. */
export const REFS_POLL_INTERVAL_MS = 3000;
/** 줄을 고른 뒤 이만큼 기다렸다가 상세를 읽는다. 키보드로 빠르게 지나간 줄의 상세는 읽지 않는다. */
const DETAIL_DELAY_MS = 120;
/** "다음 찾기" 한 번이 더 읽는 쪽의 최댓값이다. 한 쪽은 `GIT_GRAPH_PAGE_LIMIT_MAX` 줄이다. */
const FIND_MORE_PAGES = 10;

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export interface GitGraph {
  state: GitGraphState;
  reload(): void;
  /** 다음 쪽을 읽어 잇는다. 이은 뒤의 상태를 준다. 읽지 않았거나 실패하면 `null` 이다. */
  loadMore(limit?: number): Promise<LoadedGitGraph | null>;
  /** 저장소가 바뀌었는지 지금 본다. 바뀌었으면 보던 자리를 지키며 다시 읽는다. 작업을 실행한 뒤에 부른다. */
  refresh(): void;
  /** 작업 트리의 끝나지 않은 일이다. ref 지문과 함께 읽는다. 아직 읽지 않았으면 `null` 이다. */
  progress: GitWorktreeProgress | null;
}

/** 저장소의 그래프를 읽는다. 폴더가 바뀌면 첫 쪽부터 다시 읽는다. 저장소가 바뀌면 읽은 만큼 다시 읽는다. */
export function useGitGraph(request: GatewayRequest, cwd: string | null): GitGraph {
  const [state, setState] = useState<GitGraphState>({ phase: "idle" });
  // 늦게 온 답이 새 요청의 화면을 덮지 않게 한다. 다시 읽을 때마다 올린다.
  const generation = useRef(0);
  const latest = useRef(state);
  latest.current = state;
  // 저장소가 바뀌었는지 보는 중이다. 주기가 겹치지 않게 한다.
  const polling = useRef(false);
  // 다른 폴더의 값은 쓰지 않는다.
  const [progress, setProgress] = useState<{ cwd: string; progress: GitWorktreeProgress } | null>(null);

  const reload = useCallback(() => {
    const current = ++generation.current;
    if (!cwd) {
      setState({ phase: "idle" });
      return;
    }
    setState({ phase: "loading", cwd });
    fetchFirstGraphPage(request, cwd)
      .then((page) => current === generation.current && setState(loadedFromFirstPage(cwd, page)))
      .catch((error) => current === generation.current && setState({ phase: "failed", cwd, message: errorText(error) }));
  }, [request, cwd]);

  useEffect(reload, [reload]);

  const pendingMore = useRef<Promise<LoadedGitGraph | null> | null>(null);
  const readMore = useCallback(
    async (limit?: number): Promise<LoadedGitGraph | null> => {
      const loaded = latest.current;
      if (loaded.phase !== "loaded" || loaded.next === null || loaded.loadingMore) return null;
      const current = generation.current;
      const loading = { ...loaded, loadingMore: true, moreError: null };
      latest.current = loading;
      setState(loading);
      try {
        const page = await fetchGraphPage(request, loaded.cwd, loaded.next, limit);
        if (current !== generation.current) return null;
        // 읽는 사이에 ref 가 바뀌었다. 앞 쪽과 이을 수 없으므로 처음부터 다시 읽는다.
        if (page.stale) {
          reload();
          return null;
        }
        const appended = appendPage(loaded, page);
        latest.current = appended;
        setState(appended);
        return appended;
      } catch (error) {
        if (current === generation.current) setState((now) => (now.phase === "loaded" ? { ...now, loadingMore: false, moreError: errorText(error) } : now));
        return null;
      }
    },
    [request, reload],
  );
  // 이미 읽는 중이면 그 읽기의 결과를 준다. 찾기의 "다음"이 자동 읽기와 겹쳐도 그 쪽부터 이어서 찾는다.
  const loadMore = useCallback(
    (limit?: number): Promise<LoadedGitGraph | null> => {
      if (pendingMore.current) return pendingMore.current;
      const reading = readMore(limit).finally(() => {
        pendingMore.current = null;
      });
      pendingMore.current = reading;
      return reading;
    },
    [readMore],
  );

  // 저장소를 감시한다. 지문이 다르면 읽은 만큼 다시 읽고 줄을 바꾼다. 화면은 그동안 앞의 줄을 보인다. 읽는 사이에
  // 화면의 상태가 바뀌었으면 (다음 쪽을 읽었다, 다시 읽었다) 읽은 것을 버린다: 낡은 자리와 새 줄이 섞이지 않게 한다.
  // 버렸거나 실패했으면 다음 주기에 다시 본다.
  // 끝나지 않은 일(상태 띠)은 그래프를 읽는 중에도 읽는다.
  const refresh = useCallback(async () => {
    if (!cwd || polling.current) return;
    polling.current = true;
    const current = generation.current;
    try {
      const refs = await fetchRefsState(request, cwd);
      setProgress({ cwd, progress: refs.progress });
      const shown = latest.current;
      if (shown.phase !== "loaded" || shown.loadingMore || refs.refsDigest === shown.refsDigest) return;
      const fresh = await fetchGraphRows(request, cwd, shown.rows.length);
      // `generation` 은 폴더가 바뀌거나 다시 읽기를 시작한 그 자리에서 오른다. 화면의 상태는 그리기 뒤에야 바뀐다.
      if (current === generation.current && latest.current === shown) setState(fresh);
    } catch {
      // 다음 주기에 다시 본다.
    } finally {
      polling.current = false;
    }
  }, [request, cwd]);

  useEffect(() => {
    if (!cwd) return undefined;
    void refresh();
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") void refresh();
    }, REFS_POLL_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [cwd, refresh]);

  return { state, reload, loadMore, refresh, progress: progress?.cwd === cwd ? progress.progress : null };
}

export type CommitDetailState = { phase: "idle" } | { phase: "loading" } | { phase: "loaded"; detail: GitCommitDetail } | { phase: "missing" } | { phase: "failed"; message: string };

/** 고른 커밋의 상세를 읽는다. 선택이 바뀌면 앞의 읽기를 버린다. */
export function useCommitDetail(request: GatewayRequest, cwd: string | null, commit: string | null): CommitDetailState {
  const [state, setState] = useState<CommitDetailState>({ phase: "idle" });
  useEffect(() => {
    if (!cwd || !commit) {
      setState({ phase: "idle" });
      return undefined;
    }
    let current = true;
    setState({ phase: "loading" });
    const timer = setTimeout(() => {
      fetchCommitDetail(request, cwd, commit).then(
        (detail) => current && setState(detail ? { phase: "loaded", detail } : { phase: "missing" }),
        (error: unknown) => current && setState({ phase: "failed", message: errorText(error) }),
      );
    }, DETAIL_DELAY_MS);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [request, cwd, commit]);
  return state;
}

const REF_KIND_TITLE: Partial<Record<GitRefLabel["kind"], string>> = { backup: "백업", tag: "태그" };

function RefChip({ label }: { label: GitRefLabel }) {
  const kind = REF_KIND_TITLE[label.kind];
  const title = label.syncedRemotes.length > 0 ? `${label.name} (${label.syncedRemotes.join(", ")} 와 같은 커밋)` : kind ? `${kind} ${label.name}` : label.name;
  return (
    <span className={`git-ref git-ref-${label.kind}${label.head ? " git-ref-current" : ""}`} title={title} data-testid="git-graph-ref">
      {label.name}
      {label.syncedRemotes.length > 0 && <span className="git-ref-synced">{label.syncedRemotes.join(" ")}</span>}
    </span>
  );
}

function ThreadBadge({ badge, onOpenThread }: { badge: GitThreadBadge; onOpenThread(threadId: string): void }) {
  const others = badge.threadCount > 1 ? ` 외 ${badge.threadCount - 1}개` : "";
  return (
    <button
      type="button"
      className="git-thread-badge"
      title={`스레드 열기: ${badge.title}${others} (${badge.worktreePath})`}
      data-testid="git-graph-thread"
      onClick={(event) => {
        // 줄을 고르지 않는다. 배지는 스레드로 간다.
        event.stopPropagation();
        onOpenThread(badge.threadId);
      }}
    >
      {badge.title}
      {badge.threadCount > 1 && <span className="git-ref-synced">+{badge.threadCount - 1}</span>}
    </button>
  );
}

const GraphCell = memo(function GraphCell({ row, width, head, planned }: { row: GitGraphRow; width: number; head: boolean; planned: boolean }) {
  return (
    <svg className="git-graph-cell" width={width} height={ROW_HEIGHT} aria-hidden="true">
      {rowPaths(row).map((path, index) => (
        <path key={index} d={path.d} className={`git-lane-${laneColorIndex(path.colorLane)}`} />
      ))}
      <circle cx={laneX(row.lane)} cy={ROW_HEIGHT / 2} r={NODE_RADIUS} className={`git-node git-lane-${laneColorIndex(row.lane)}${head ? " git-node-head" : ""}`} />
      {planned && <circle cx={laneX(row.lane)} cy={ROW_HEIGHT / 2} r={NODE_RADIUS + 3} className="git-node-planned" />}
    </svg>
  );
});

interface GraphRowProps {
  row: GitGraphRow;
  width: number;
  labels: GitRefLabel[] | undefined;
  badges: GitThreadBadge[] | undefined;
  head: boolean;
  /** 열린 계획이 이 커밋으로 옮기는 브랜치(또는 HEAD)이다. 점선으로 그린다. */
  planned: { name: string; title: string } | undefined;
  now: number;
  onOpenThread(threadId: string): void;
}

const GraphRow = memo(function GraphRow({ row, width, labels, badges, head, planned, now, onOpenThread }: GraphRowProps) {
  return (
    <>
      <GraphCell row={row} width={width} head={head} planned={planned !== undefined} />
      <span className="git-row-main">
        {planned && (
          <span className="git-ref git-ref-planned" title={`계획: ${planned.title}`} data-testid="git-graph-planned">
            {planned.name}
          </span>
        )}
        {labels?.map((label) => <RefChip key={`${label.kind}:${label.name}`} label={label} />)}
        {badges?.map((badge) => <ThreadBadge key={badge.worktreePath} badge={badge} onOpenThread={onOpenThread} />)}
        <span className="git-row-subject" title={row.subject}>
          {row.subject}
        </span>
      </span>
      <span className="git-row-author muted" title={row.authorName}>
        {row.authorName}
      </span>
      <span className="git-row-time muted">{relativeTime(row.authorTime, now)}</span>
      <code className="git-row-sha muted" title={row.id}>
        {shortSha(row.id)}
      </code>
    </>
  );
});

function rowElementId(commit: string): string {
  return `git-row-${commit}`;
}

interface GraphRowsProps {
  graph: GitGraph & { state: LoadedGitGraph };
  folders: readonly BadgeFolder[];
  selected: string | null;
  /** 실행 전의 계획이다. 결과의 자리를 점선으로 그린다. */
  preview: GitOperationPlan | null;
  /** 줄 목록의 맨 위에 둔다 ("커밋하지 않은 변경"). */
  top: ReactNode;
  onSelect(commit: string): void;
  onOpenThread(threadId: string): void;
}

function GraphRows({ graph, folders, selected, preview, top, onSelect, onOpenThread }: GraphRowsProps) {
  const { state, loadMore } = graph;
  const scrollRef = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState(false);
  const labels = useMemo(() => refLabels(state.refs, state.head), [state.refs, state.head]);
  const badges = useMemo(() => threadBadges(state.worktrees, folders), [state.worktrees, folders]);
  const width = useMemo(() => graphWidth(state.rows), [state.rows]);
  const matches = useMemo(() => findRows(state.rows, labels, query), [state.rows, labels, query]);
  const matched = useMemo(() => new Set(matches), [matches]);
  const planned = useMemo(() => (preview?.after ? { commit: preview.after.commit, name: preview.branch ?? "HEAD", title: operationTitle(preview.operation, preview.branch) } : null), [preview]);
  const selectedIndex = useMemo(() => (selected === null ? -1 : state.rows.findIndex((row) => row.id === selected)), [state.rows, selected]);
  // 상대 시간의 기준이다. 목록을 다시 읽을 때만 바꾼다. 줄을 그릴 때마다 시계를 읽지 않는다.
  const now = useMemo(() => Date.now(), [state.refs]);
  const virtualizer = useVirtualizer({
    count: state.rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    getItemKey: (index) => state.rows[index]?.id ?? index,
    overscan: 12,
  });
  const items = virtualizer.getVirtualItems();
  const lastVisible = items[items.length - 1]?.index ?? 0;
  const wantsMore = state.next !== null && !state.loadingMore && state.moreError === null && lastVisible >= state.rows.length - LOAD_AHEAD_ROWS;
  useEffect(() => {
    if (wantsMore) void loadMore();
  }, [wantsMore, loadMore]);

  // 저장소가 바뀌어서 줄이 바뀌면, 화면 맨 위에 있던 커밋이 같은 자리에 남게 스크롤을 옮긴다. 뒤에 줄을 이은 때는
  // 위치가 같으므로 아무 일도 없다.
  const previousRows = useRef(state.rows);
  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (element && previousRows.current !== state.rows) {
      const top = scrollTopAfterRowsChange(previousRows.current, state.rows, element.scrollTop);
      if (top !== element.scrollTop) element.scrollTop = top;
    }
    previousRows.current = state.rows;
  }, [state.rows]);

  const select = (index: number) => {
    const row = state.rows[index];
    if (!row) return;
    onSelect(row.id);
    virtualizer.scrollToIndex(index, { align: "auto" });
  };

  // 다음 찾은 줄로 간다. 읽은 줄에 더 없으면 다음 쪽들을 읽으며 찾는다. 끝까지 없으면 처음의 찾은 줄로 돌아간다.
  const findNext = async (step: 1 | -1) => {
    if (query.trim() === "") return;
    const ahead = step === 1 ? matches.find((index) => index > selectedIndex) : undefined;
    if (step === -1 || ahead !== undefined || state.next === null) {
      const target = ahead ?? nextMatch(matches, selectedIndex, step);
      if (target !== null) select(target);
      return;
    }
    setSearching(true);
    let loaded: LoadedGitGraph = state;
    for (let page = 0; page < FIND_MORE_PAGES && loaded.next !== null; page += 1) {
      const more = await loadMore(GIT_GRAPH_PAGE_LIMIT_MAX);
      if (!more) break;
      const found = findRows(more.rows.slice(loaded.rows.length), refLabels(more.refs, more.head), query)[0];
      if (found !== undefined) {
        onSelect(more.rows[loaded.rows.length + found]!.id);
        setSearching(false);
        return;
      }
      loaded = more;
    }
    setSearching(false);
    const first = loaded.next === null ? matches[0] : undefined;
    if (first !== undefined) select(first);
  };

  // 찾기로 고른 줄이 방금 읽은 쪽에 있으면, 줄이 그려진 뒤에 그 자리로 간다.
  useEffect(() => {
    if (selectedIndex >= 0) virtualizer.scrollToIndex(selectedIndex, { align: "auto" });
    // 선택이 바뀔 때만 옮긴다. 줄을 더 읽을 때마다 고른 줄로 돌아가지 않는다.
  }, [selected]);

  const onListKey = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const from = selectedIndex >= 0 ? selectedIndex : (items[0]?.index ?? 0) - 1;
    select(Math.max(0, Math.min(state.rows.length - 1, from + (event.key === "ArrowDown" ? 1 : -1))));
  };

  const position = matches.indexOf(selectedIndex);
  return (
    <>
      <div className="git-find" role="search">
        <input
          type="search"
          value={query}
          placeholder="제목, 작성자, sha, 브랜치로 찾기"
          aria-label="커밋 찾기"
          data-testid="git-graph-find"
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") void findNext(event.shiftKey ? -1 : 1);
            if (event.key === "Escape") setQuery("");
          }}
        />
        {query.trim() !== "" && (
          <>
            <span className="muted" aria-live="polite" data-testid="git-graph-find-count">
              {searching ? "찾는 중…" : matches.length === 0 ? "없음" : `${position >= 0 ? `${position + 1} / ` : ""}${matches.length}개`}
              {!searching && state.next !== null && ` (읽은 ${state.rows.length}개에서)`}
            </span>
            <button type="button" disabled={searching || matches.length === 0} onClick={() => void findNext(-1)} data-testid="git-graph-find-previous">
              이전
            </button>
            <button type="button" disabled={searching || (matches.length === 0 && state.next === null)} onClick={() => void findNext(1)} data-testid="git-graph-find-next">
              다음
            </button>
          </>
        )}
      </div>
      {top}
      <div
        className="work-scroll git-graph-scroll"
        ref={scrollRef}
        tabIndex={0}
        role="listbox"
        aria-label="커밋"
        aria-activedescendant={selected !== null && selectedIndex >= 0 ? rowElementId(selected) : undefined}
        onKeyDown={onListKey}
        data-testid="git-graph-scroll"
      >
        <div className="git-graph-rows" style={{ height: virtualizer.getTotalSize() }} role="presentation">
          {items.map((item) => {
            const row = state.rows[item.index]!;
            const dimmed = row.backupOnly || (matches.length > 0 && !matched.has(item.index));
            return (
              <div
                key={item.key}
                id={rowElementId(row.id)}
                className={`git-row${row.id === selected ? " git-row-selected" : ""}${dimmed ? " git-row-dim" : ""}`}
                style={{ transform: `translateY(${item.start}px)` }}
                role="option"
                aria-selected={row.id === selected}
                onClick={() => onSelect(row.id)}
                data-testid="git-graph-row"
                data-commit={row.id}
              >
                <GraphRow row={row} width={width} labels={labels.get(row.id)} badges={badges.get(row.id)} head={row.id === state.head} planned={planned?.commit === row.id ? planned : undefined} now={now} onOpenThread={onOpenThread} />
              </div>
            );
          })}
        </div>
        {state.loadingMore && <p className="muted work-section">더 읽는 중…</p>}
        {state.moreError !== null && (
          <div className="error work-section" data-testid="git-graph-more-error">
            {state.moreError}{" "}
            <button type="button" onClick={() => void loadMore()}>
              다시 읽기
            </button>
          </div>
        )}
      </div>
    </>
  );
}

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
