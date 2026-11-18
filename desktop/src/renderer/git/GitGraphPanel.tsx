// 커밋 그래프 패널이다 (plan 27 G1). 열린 스레드의 저장소의 커밋을 그래프로 보여 준다. 읽기만 한다.
// 긴 이력은 보이는 줄만 그린다. 끝에 가까워지면 다음 쪽을 읽는다. 줄 높이는 고정이다: 줄마다 그린 선이 위아래 줄과
// 이어져야 한다.
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { GatewayRequest } from "../work/notionHub";
import { appendPage, fetchGraphPage, type GitGraphRow, type GitGraphState, type GitRefLabel, headName, loadedFromFirstPage, refLabels, relativeTime, shortSha } from "./gitGraph";
import { graphWidth, laneColorIndex, laneX, NODE_RADIUS, ROW_HEIGHT, rowPaths } from "./graphGeometry";

/** 읽은 줄의 끝에서 이만큼 앞에 오면 다음 쪽을 읽는다. */
const LOAD_AHEAD_ROWS = 60;

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export interface GitGraph {
  state: GitGraphState;
  reload(): void;
  loadMore(): void;
}

/** 저장소의 그래프를 읽는다. 폴더가 바뀌면 첫 쪽부터 다시 읽는다. */
export function useGitGraph(request: GatewayRequest, cwd: string | null): GitGraph {
  const [state, setState] = useState<GitGraphState>({ phase: "idle" });
  // 늦게 온 답이 새 요청의 화면을 덮지 않게 한다. 다시 읽을 때마다 올린다.
  const generation = useRef(0);
  const latest = useRef(state);
  latest.current = state;

  const reload = useCallback(() => {
    const current = ++generation.current;
    if (!cwd) {
      setState({ phase: "idle" });
      return;
    }
    setState({ phase: "loading", cwd });
    fetchGraphPage(request, cwd, null)
      .then((page) => current === generation.current && setState(loadedFromFirstPage(cwd, page)))
      .catch((error) => current === generation.current && setState({ phase: "failed", cwd, message: errorText(error) }));
  }, [request, cwd]);

  useEffect(reload, [reload]);

  const loadMore = useCallback(() => {
    const loaded = latest.current;
    if (loaded.phase !== "loaded" || loaded.next === null || loaded.loadingMore) return;
    const current = generation.current;
    setState({ ...loaded, loadingMore: true, moreError: null });
    fetchGraphPage(request, loaded.cwd, loaded.next)
      .then((page) => {
        if (current !== generation.current) return;
        // 읽는 사이에 ref 가 바뀌었다. 앞 쪽과 이을 수 없으므로 처음부터 다시 읽는다.
        if (page.stale) reload();
        else setState((now) => (now.phase === "loaded" ? appendPage(now, page) : now));
      })
      .catch((error) => current === generation.current && setState((now) => (now.phase === "loaded" ? { ...now, loadingMore: false, moreError: errorText(error) } : now)));
  }, [request, reload]);

  return { state, reload, loadMore };
}

function RefChip({ label }: { label: GitRefLabel }) {
  const title = label.syncedRemotes.length > 0 ? `${label.name} (${label.syncedRemotes.join(", ")} 와 같은 커밋)` : label.name;
  return (
    <span className={`git-ref git-ref-${label.kind}${label.head ? " git-ref-current" : ""}`} title={title} data-testid="git-graph-ref">
      {label.name}
      {label.syncedRemotes.length > 0 && <span className="git-ref-synced">{label.syncedRemotes.join(" ")}</span>}
    </span>
  );
}

const GraphCell = memo(function GraphCell({ row, width, head }: { row: GitGraphRow; width: number; head: boolean }) {
  return (
    <svg className="git-graph-cell" width={width} height={ROW_HEIGHT} aria-hidden="true">
      {rowPaths(row).map((path, index) => (
        <path key={index} d={path.d} className={`git-lane-${laneColorIndex(path.colorLane)}`} />
      ))}
      <circle cx={laneX(row.lane)} cy={ROW_HEIGHT / 2} r={NODE_RADIUS} className={`git-node git-lane-${laneColorIndex(row.lane)}${head ? " git-node-head" : ""}`} />
    </svg>
  );
});

const GraphRow = memo(function GraphRow({ row, width, labels, head, now }: { row: GitGraphRow; width: number; labels: GitRefLabel[] | undefined; head: boolean; now: number }) {
  return (
    <>
      <GraphCell row={row} width={width} head={head} />
      <span className="git-row-main">
        {labels?.map((label) => <RefChip key={`${label.kind}:${label.name}`} label={label} />)}
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

function GraphRows({ graph }: { graph: GitGraph & { state: Extract<GitGraphState, { phase: "loaded" }> } }) {
  const { state, loadMore } = graph;
  const scrollRef = useRef<HTMLDivElement>(null);
  const labels = useMemo(() => refLabels(state.refs, state.head), [state.refs, state.head]);
  const width = useMemo(() => graphWidth(state.rows), [state.rows]);
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
    if (wantsMore) loadMore();
  }, [wantsMore, loadMore]);

  return (
    <div className="work-scroll git-graph-scroll" ref={scrollRef} data-testid="git-graph-scroll">
      <div className="git-graph-rows" style={{ height: virtualizer.getTotalSize() }} role="list">
        {items.map((item) => {
          const row = state.rows[item.index]!;
          return (
            <div key={item.key} className="git-row" style={{ transform: `translateY(${item.start}px)` }} role="listitem" data-testid="git-graph-row" data-commit={row.id}>
              <GraphRow row={row} width={width} labels={labels.get(row.id)} head={row.id === state.head} now={now} />
            </div>
          );
        })}
      </div>
      {state.loadingMore && <p className="muted work-section">더 읽는 중…</p>}
      {state.moreError !== null && (
        <div className="error work-section" data-testid="git-graph-more-error">
          {state.moreError}{" "}
          <button type="button" onClick={loadMore}>
            다시 읽기
          </button>
        </div>
      )}
    </div>
  );
}

export function GitGraphPanel({ request, cwd }: { request: GatewayRequest; cwd: string | null }) {
  const graph = useGitGraph(request, cwd);
  const { state, reload } = graph;
  const current = state.phase === "loaded" ? headName(state.refs, state.head) : null;
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
      {state.phase === "idle" && <p className="muted work-section">스레드를 열면 그 스레드의 저장소의 커밋 그래프를 보여 줍니다.</p>}
      {state.phase === "failed" && (
        <div className="error work-section" data-testid="git-graph-error">
          {state.message}
        </div>
      )}
      {state.phase === "loaded" && !state.repository && <p className="muted work-section">이 폴더는 git 저장소가 아닙니다.</p>}
      {state.phase === "loaded" && state.repository && state.rows.length === 0 && <p className="muted work-section">이 저장소에는 아직 커밋이 없습니다.</p>}
      {state.phase === "loaded" && state.repository && state.rows.length > 0 && <GraphRows graph={{ ...graph, state }} />}
    </aside>
  );
}
