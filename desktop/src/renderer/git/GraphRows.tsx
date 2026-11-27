import { useVirtualizer } from "@tanstack/react-virtual";
import { type KeyboardEvent, type ReactNode, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { type BadgeFolder, findRows, GIT_GRAPH_PAGE_LIMIT_MAX, type LoadedGitGraph, nextMatch, refLabels, scrollTopAfterRowsChange, threadBadges } from "./gitGraph";
import { type GitOperationPlan, operationTitle } from "./gitOperations";
import { graphWidth, ROW_HEIGHT } from "./graphGeometry";
import { DRAG_BRANCH,GROUP_DROP } from "./GroupPlanControls";
import { GraphRow } from "./GraphRow";
import { openCommitActions } from "./openCommitActions";
import type { GitGraph } from "./useGitGraph";

/** 읽은 줄의 끝에서 이만큼 앞에 오면 다음 쪽을 읽는다. */
const LOAD_AHEAD_ROWS = 60;
/** "다음 찾기" 한 번이 더 읽는 쪽의 최댓값이다. 한 쪽은 `GIT_GRAPH_PAGE_LIMIT_MAX` 줄이다. */
const FIND_MORE_PAGES = 10;
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
  picks: readonly string[];
  togglePick(commit: string): void;
  onSelect(commit: string): void;
  onOpenThread(threadId: string): void;
}

export function GraphRows({ graph, folders, selected, preview, top, picks, togglePick, onSelect, onOpenThread }: GraphRowsProps) {
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
            const dimmed = row.outsideFilter || row.backupOnly || (matches.length > 0 && !matched.has(item.index));
            return (
              <div
                key={item.key}
                id={rowElementId(row.id)}
                className={`git-row${row.id === selected ? " git-row-selected" : ""}${dimmed ? " git-row-dim" : ""}`}
                style={{ transform: `translateY(${item.start}px)` }}
                role="option"
                aria-selected={row.id === selected}
                onDragOver={event=>{if(event.dataTransfer.types.includes(DRAG_BRANCH))event.preventDefault();}}
                onDrop={event=>{const current=state.refs.find(r=>r.kind==="local_branch"&&r.head);if(!current||row.id===state.head||event.dataTransfer.getData(DRAG_BRANCH)!==current.name)return;event.preventDefault();flushSync(()=>onSelect(row.id));window.dispatchEvent(new Event(GROUP_DROP));}}
                onClick={() => onSelect(row.id)}
                onContextMenu={(event) => {
                  event.preventDefault();
                  flushSync(() => onSelect(row.id));
                  openCommitActions();
                }}
                data-testid="git-graph-row"
                data-commit={row.id}
              >
                <input type="checkbox" aria-label={"cherry-pick: " + row.subject} checked={picks.includes(row.id)} onClick={(event) => event.stopPropagation()} onChange={() => { togglePick(row.id); onSelect(row.id); }} />
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
