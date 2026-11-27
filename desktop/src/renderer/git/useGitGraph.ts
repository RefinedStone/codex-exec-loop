import { useCallback, useEffect, useRef, useState } from "react";
import type { GatewayRequest } from "../work/notionHub";
import { appendPage, fetchCommitDetail, fetchFirstGraphPage, fetchGraphPage, fetchGraphRows, fetchRefsState, loadedFromFirstPage, type GitCommitDetail, type GitGraphState, type GitWorktreeProgress, type LoadedGitGraph } from "./gitGraph";

/** 저장소가 바뀌었는지 보는 주기이다. 패널이 열려 있고 창이 보일 때만 본다. */
export const REFS_POLL_INTERVAL_MS = 3000;
/** 줄을 고른 뒤 이만큼 기다렸다가 상세를 읽는다. 키보드로 빠르게 지나간 줄의 상세는 읽지 않는다. */
const DETAIL_DELAY_MS = 120;
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
