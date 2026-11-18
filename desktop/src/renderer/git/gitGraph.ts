// 커밋 그래프의 자료이다 (plan 27 G1). Gateway 의 `akra/git/graph` 가 저장소의 커밋을 쪽 단위로 준다. 레인 배치는
// Gateway 가 정한다. 여기서는 답을 읽고, 쪽을 잇고, 커밋마다 붙일 라벨을 정한다. 저장소를 고치는 일은 없다.
import type { GatewayRequest } from "../work/notionHub";
import type { RowLanes } from "./graphGeometry";

export const GIT_GRAPH_METHOD = "akra/git/graph";

export type GitRefKind = "local_branch" | "remote_branch" | "tag";

export interface GitRef {
  /** 짧은 이름이다 (`main`, `origin/main`, `v1.0`). */
  name: string;
  kind: GitRefKind;
  /** 가리키는 커밋의 전체 sha 이다. */
  target: string;
  /** 이 작업 트리의 HEAD 가 이 브랜치이다. */
  head: boolean;
}

export interface GitGraphRow extends RowLanes {
  id: string;
  parents: string[];
  authorName: string;
  /** 작성 시각이다. Unix 초이다. */
  authorTime: number;
  subject: string;
}

export interface GitGraphPage {
  repository: boolean;
  /** 보낸 자리가 낡았다. 첫 쪽부터 다시 읽어야 한다. */
  stale: boolean;
  head: string | null;
  refs: GitRef[];
  rows: GitGraphRow[];
  /** 다음 쪽의 자리이다. 내용을 보지 않고 그대로 돌려보낸다. 마지막 쪽이면 `null` 이다. */
  next: unknown;
}

export type GitGraphState =
  | { phase: "idle" }
  | { phase: "loading"; cwd: string }
  | { phase: "failed"; cwd: string; message: string }
  | {
      phase: "loaded";
      cwd: string;
      repository: boolean;
      head: string | null;
      refs: GitRef[];
      rows: GitGraphRow[];
      next: unknown;
      loadingMore: boolean;
      /** 다음 쪽을 읽다가 난 오류이다. 있으면 자동으로 더 읽지 않는다. */
      moreError: string | null;
    };

export type LoadedGitGraph = Extract<GitGraphState, { phase: "loaded" }>;

/** 커밋 옆에 붙이는 라벨 하나이다. */
export interface GitRefLabel {
  name: string;
  kind: GitRefKind | "head";
  /** HEAD 가 이 브랜치이다. */
  head: boolean;
  /** 같은 커밋에 있는 같은 이름의 원격 브랜치의 원격 이름들이다 (`origin`). 로컬 브랜치에만 있다. */
  syncedRemotes: string[];
}

const REF_KINDS: readonly GitRefKind[] = ["local_branch", "remote_branch", "tag"];

function text(value: unknown): string | undefined {
  return typeof value === "string" && value ? value : undefined;
}

function lanes(value: unknown): number[] {
  return Array.isArray(value) ? value.filter((lane): lane is number => Number.isInteger(lane) && lane >= 0) : [];
}

function refFrom(entry: unknown): GitRef[] {
  const item = (entry ?? {}) as Record<string, unknown>;
  const name = text(item.name);
  const target = text(item.target);
  const kind = REF_KINDS.find((candidate) => candidate === item.kind);
  if (!name || !target || !kind) return [];
  return [{ name, kind, target, head: item.head === true }];
}

function rowFrom(entry: unknown): GitGraphRow[] {
  const item = (entry ?? {}) as Record<string, unknown>;
  const id = text(item.id);
  if (!id || !Number.isInteger(item.lane) || (item.lane as number) < 0) return [];
  return [
    {
      id,
      parents: Array.isArray(item.parents) ? item.parents.filter((parent): parent is string => typeof parent === "string") : [],
      authorName: text(item.authorName) ?? "",
      authorTime: typeof item.authorTime === "number" ? item.authorTime : 0,
      subject: text(item.subject) ?? "",
      lane: item.lane as number,
      lanesIn: lanes(item.lanesIn),
      lanesOut: lanes(item.lanesOut),
      lanesThrough: lanes(item.lanesThrough),
    },
  ];
}

/** Gateway 응답을 읽는다. 필드가 빠진 줄과 ref 는 뺀다. */
export function graphPageFrom(result: unknown): GitGraphPage {
  const page = result as Record<string, unknown> | null;
  if (!page || typeof page.repository !== "boolean" || !Array.isArray(page.rows) || !Array.isArray(page.refs)) {
    throw new Error("커밋 그래프의 형식을 알 수 없습니다.");
  }
  return {
    repository: page.repository,
    stale: page.stale === true,
    head: text(page.head) ?? null,
    refs: page.refs.flatMap(refFrom),
    rows: page.rows.flatMap(rowFrom),
    next: page.next ?? null,
  };
}

/** 그래프의 한 쪽을 읽는다. `cursor` 가 `null` 이면 첫 쪽이다. */
export async function fetchGraphPage(request: GatewayRequest, cwd: string, cursor: unknown): Promise<GitGraphPage> {
  return graphPageFrom(await request(GIT_GRAPH_METHOD, cursor === null ? { cwd } : { cwd, cursor }));
}

export function loadedFromFirstPage(cwd: string, page: GitGraphPage): LoadedGitGraph {
  return { phase: "loaded", cwd, repository: page.repository, head: page.head, refs: page.refs, rows: page.rows, next: page.next, loadingMore: false, moreError: null };
}

/** 다음 쪽을 뒤에 잇는다. ref 와 HEAD 는 첫 쪽의 것을 지킨다. */
export function appendPage(state: LoadedGitGraph, page: GitGraphPage): LoadedGitGraph {
  return { ...state, rows: [...state.rows, ...page.rows], next: page.next, loadingMore: false, moreError: null };
}

/** `origin/main` 을 원격 이름과 브랜치 이름으로 나눈다. */
function splitRemote(name: string): { remote: string; branch: string } | null {
  const slash = name.indexOf("/");
  return slash > 0 ? { remote: name.slice(0, slash), branch: name.slice(slash + 1) } : null;
}

/**
 * 커밋마다 붙일 라벨이다. 순서는 HEAD 브랜치, 로컬 브랜치, 원격 브랜치, 태그이다. 원격 브랜치가 같은 이름의 로컬
 * 브랜치와 같은 커밋에 있으면 따로 그리지 않고 로컬 라벨에 원격 이름을 붙인다. HEAD 가 브랜치가 아니면 `HEAD`
 * 라벨을 그 커밋의 맨 앞에 둔다.
 */
export function refLabels(refs: readonly GitRef[], head: string | null): Map<string, GitRefLabel[]> {
  const byCommit = new Map<string, GitRefLabel[]>();
  const labelsAt = (target: string): GitRefLabel[] => {
    const existing = byCommit.get(target);
    if (existing) return existing;
    const created: GitRefLabel[] = [];
    byCommit.set(target, created);
    return created;
  };
  if (head && !refs.some((ref) => ref.head)) labelsAt(head).push({ name: "HEAD", kind: "head", head: true, syncedRemotes: [] });
  const locals = refs.filter((ref) => ref.kind === "local_branch");
  for (const ref of [...locals.filter((ref) => ref.head), ...locals.filter((ref) => !ref.head)]) {
    labelsAt(ref.target).push({ name: ref.name, kind: ref.kind, head: ref.head, syncedRemotes: [] });
  }
  for (const ref of refs.filter((candidate) => candidate.kind === "remote_branch")) {
    const parts = splitRemote(ref.name);
    const local = parts && labelsAt(ref.target).find((label) => label.kind === "local_branch" && label.name === parts.branch);
    if (parts && local) local.syncedRemotes.push(parts.remote);
    else labelsAt(ref.target).push({ name: ref.name, kind: ref.kind, head: false, syncedRemotes: [] });
  }
  for (const ref of refs.filter((candidate) => candidate.kind === "tag")) {
    labelsAt(ref.target).push({ name: ref.name, kind: ref.kind, head: false, syncedRemotes: [] });
  }
  return byCommit;
}

/** 지금 체크아웃한 것의 이름이다: 브랜치 이름, 또는 브랜치가 아니면 짧은 sha 이다. */
export function headName(refs: readonly GitRef[], head: string | null): string | null {
  return refs.find((ref) => ref.head)?.name ?? (head ? shortSha(head) : null);
}

export function shortSha(sha: string): string {
  return sha.slice(0, 7);
}

const MINUTE = 60;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** 작성 시각을 짧게 쓴다. 30 일이 넘으면 날짜를 쓴다. */
export function relativeTime(unixSeconds: number, nowMs: number): string {
  if (unixSeconds <= 0) return "";
  const elapsed = Math.max(0, Math.floor(nowMs / 1000) - unixSeconds);
  if (elapsed < MINUTE) return "방금";
  if (elapsed < HOUR) return `${Math.floor(elapsed / MINUTE)}분 전`;
  if (elapsed < DAY) return `${Math.floor(elapsed / HOUR)}시간 전`;
  if (elapsed < 30 * DAY) return `${Math.floor(elapsed / DAY)}일 전`;
  return new Date(unixSeconds * 1000).toISOString().slice(0, 10);
}
