// 커밋 그래프의 자료이다 (plan 27 G1, G2). Gateway 의 `akra/git/graph` 가 저장소의 커밋을 쪽 단위로 준다. 레인 배치는
// Gateway 가 정한다. 여기서는 답을 읽고, 쪽을 잇고, 커밋마다 붙일 라벨과 스레드 배지를 정하고, 읽은 줄에서 찾는다.
// 저장소를 고치는 일은 없다.
import type { GatewayRequest } from "../work/notionHub";
import { ROW_HEIGHT, type RowLanes } from "./graphGeometry";

export const GIT_GRAPH_METHOD = "akra/git/graph";
export const GIT_REFS_METHOD = "akra/git/refs";
export const GIT_COMMIT_METHOD = "akra/git/commit";
/** Gateway 가 한 번에 주는 줄 수의 최댓값이다. */
export const GIT_GRAPH_PAGE_LIMIT_MAX = 1000;

export type GitRefKind = "local_branch" | "remote_branch" | "tag" | "backup";

export interface GitRef {
  /** 짧은 이름이다 (`main`, `origin/main`, `v1.0`). */
  name: string;
  kind: GitRefKind;
  /** 가리키는 커밋의 전체 sha 이다. */
  target: string;
  /** 이 작업 트리의 HEAD 가 이 브랜치이다. */
  head: boolean;
}

/** 저장소의 작업 트리 하나이다. */
export interface GitWorktree {
  /** git 이 쓴 폴더이다. Windows 에서도 `/` 이다. */
  path: string;
  head: string | null;
  branch: string | null;
}

export interface GitGraphRow extends RowLanes {
  id: string;
  parents: string[];
  authorName: string;
  /** 작성 시각이다. Unix 초이다. */
  authorTime: number;
  subject: string;
  /** 백업 ref 에서만 닿는 커밋이다. 흐리게 그린다. */
  backupOnly: boolean;
}

export interface GitGraphPage {
  repository: boolean;
  /** 보낸 자리가 낡았다. 첫 쪽부터 다시 읽어야 한다. */
  stale: boolean;
  head: string | null;
  /** 이 쪽을 읽을 때의 ref 들의 지문이다. `akra/git/refs` 의 값과 다르면 저장소가 바뀌었다. */
  refsDigest: string | null;
  refs: GitRef[];
  worktrees: GitWorktree[];
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
      refsDigest: string | null;
      refs: GitRef[];
      worktrees: GitWorktree[];
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

/** 커밋 하나의 상세이다. */
export interface GitCommitDetail {
  id: string;
  parents: string[];
  authorName: string;
  authorEmail: string;
  /** Unix 초이다. */
  authorTime: number;
  committerName: string;
  /** Unix 초이다. */
  committerTime: number;
  /** 커밋 메시지 전체이다. */
  message: string;
  /** git diff 형식이다. 병합 커밋은 첫 부모와 비교한 것이다. */
  diff: string;
  /** 변경이 한도를 넘어서 Gateway 가 읽지 않았다. */
  diffTooLarge: boolean;
}

const REF_KINDS: readonly GitRefKind[] = ["local_branch", "remote_branch", "tag", "backup"];

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

function worktreeFrom(entry: unknown): GitWorktree[] {
  const item = (entry ?? {}) as Record<string, unknown>;
  const path = text(item.path);
  return path ? [{ path, head: text(item.head) ?? null, branch: text(item.branch) ?? null }] : [];
}

function strings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === "string") : [];
}

function rowFrom(entry: unknown): GitGraphRow[] {
  const item = (entry ?? {}) as Record<string, unknown>;
  const id = text(item.id);
  if (!id || !Number.isInteger(item.lane) || (item.lane as number) < 0) return [];
  return [
    {
      id,
      parents: strings(item.parents),
      authorName: text(item.authorName) ?? "",
      authorTime: typeof item.authorTime === "number" ? item.authorTime : 0,
      subject: text(item.subject) ?? "",
      backupOnly: item.backupOnly === true,
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
    refsDigest: text(page.refsDigest) ?? null,
    refs: page.refs.flatMap(refFrom),
    worktrees: Array.isArray(page.worktrees) ? page.worktrees.flatMap(worktreeFrom) : [],
    rows: page.rows.flatMap(rowFrom),
    next: page.next ?? null,
  };
}

/** 그래프의 한 쪽을 읽는다. `cursor` 가 `null` 이면 첫 쪽이다. `limit` 이 없으면 Gateway 의 기본 줄 수이다. */
export async function fetchGraphPage(request: GatewayRequest, cwd: string, cursor: unknown, limit?: number): Promise<GitGraphPage> {
  return graphPageFrom(await request(GIT_GRAPH_METHOD, { cwd, ...(cursor === null ? {} : { cursor }), ...(limit === undefined ? {} : { limit }) }));
}

/** 첫 쪽도 조회 중 바뀔 수 있다. 일시적인 stale은 제한해서 재시도한다. */
export async function fetchFirstGraphPage(request: GatewayRequest, cwd: string, limit?: number): Promise<GitGraphPage> {
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const page = await fetchGraphPage(request, cwd, null, limit);
    if (!page.stale) return page;
  }
  throw new Error("커밋 그래프를 읽는 사이에 저장소가 바뀌었습니다. 다시 읽어 주십시오.");
}

export function loadedFromFirstPage(cwd: string, page: GitGraphPage): LoadedGitGraph {
  if (page.stale) throw new Error("커밋 그래프를 읽는 사이에 저장소가 바뀌었습니다. 다시 읽어 주십시오.");
  return {
    phase: "loaded",
    cwd,
    repository: page.repository,
    head: page.head,
    refsDigest: page.refsDigest,
    refs: page.refs,
    worktrees: page.worktrees,
    rows: page.rows,
    next: page.next,
    loadingMore: false,
    moreError: null,
  };
}

/**
 * 첫 쪽부터 `atLeast` 줄이 될 때까지 읽는다. 저장소가 바뀐 뒤 화면을 다시 채울 때 쓴다: 읽던 만큼 다시 읽어야
 * 사용자가 보던 줄이 남는다. 읽는 사이에 ref 가 또 바뀌면 오류이다. 부른 쪽은 다음 기회에 다시 한다.
 */
export async function fetchGraphRows(request: GatewayRequest, cwd: string, atLeast: number): Promise<LoadedGitGraph> {
  const pageRows = Math.min(GIT_GRAPH_PAGE_LIMIT_MAX, atLeast);
  let loaded = loadedFromFirstPage(cwd, await fetchFirstGraphPage(request, cwd, pageRows > 0 ? pageRows : undefined));
  while (loaded.next !== null && loaded.rows.length < atLeast) {
    const page = await fetchGraphPage(request, cwd, loaded.next, Math.min(GIT_GRAPH_PAGE_LIMIT_MAX, atLeast - loaded.rows.length));
    if (page.stale) throw new Error("커밋 그래프를 읽는 사이에 저장소가 바뀌었습니다.");
    loaded = appendPage(loaded, page);
  }
  return loaded;
}

/** 작업 트리의 끝나지 않은 일이다. 상태 띠에 쓴다. */
export interface GitWorktreeProgress {
  /** `merge`, `rebase`, `cherry-pick`, `revert`, `bisect` 이다. */
  operation: string | null;
  conflictedFiles: number;
}

/** `akra/git/refs` 의 답이다. 저장소가 아니면 지문이 `null` 이다. 옛 Gateway 는 끝나지 않은 일을 주지 않는다. */
export async function fetchRefsState(request: GatewayRequest, cwd: string): Promise<{ refsDigest: string | null; progress: GitWorktreeProgress }> {
  const state = (await request(GIT_REFS_METHOD, { cwd })) as Record<string, unknown> | null;
  const conflicted = state?.conflictedFiles;
  return {
    refsDigest: text(state?.refsDigest) ?? null,
    progress: { operation: text(state?.operationInProgress) ?? null, conflictedFiles: Number.isInteger(conflicted) && (conflicted as number) > 0 ? (conflicted as number) : 0 },
  };
}

/**
 * 상태 띠의 글들이다: 끝나지 않은 작업, 충돌, 브랜치 없는 HEAD. 끝나지 않은 일이 없으면 빈 목록이다. 화면은 한 줄로
 * 잇는다.
 */
export function unfinishedWork(progress: GitWorktreeProgress | null, refs: readonly GitRef[], head: string | null): string[] {
  const parts: string[] = [];
  if (progress?.operation) parts.push(`${progress.operation} 진행 중`);
  if (progress && progress.conflictedFiles > 0) parts.push(`충돌 파일 ${progress.conflictedFiles}개`);
  if (head && !refs.some((ref) => ref.head)) parts.push(`HEAD 분리 (${shortSha(head)})`);
  return parts;
}

/** Gateway 의 커밋 상세를 읽는다. 그 커밋이 없으면 `null` 이다. */
export function commitDetailFrom(result: unknown): GitCommitDetail | null {
  const commit = (result as Record<string, unknown> | null)?.commit as Record<string, unknown> | null | undefined;
  if (commit === null) return null;
  const id = text(commit?.id);
  if (!commit || !id) throw new Error("커밋 상세의 형식을 알 수 없습니다.");
  const seconds = (value: unknown) => (typeof value === "number" ? value : 0);
  return {
    id,
    parents: strings(commit.parents),
    authorName: text(commit.authorName) ?? "",
    authorEmail: text(commit.authorEmail) ?? "",
    authorTime: seconds(commit.authorTime),
    committerName: text(commit.committerName) ?? "",
    committerTime: seconds(commit.committerTime),
    message: text(commit.message) ?? "",
    diff: text(commit.diff) ?? "",
    diffTooLarge: commit.diffTooLarge === true,
  };
}

export async function fetchCommitDetail(request: GatewayRequest, cwd: string, commit: string): Promise<GitCommitDetail | null> {
  return commitDetailFrom(await request(GIT_COMMIT_METHOD, { cwd, commit }));
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
 * 커밋마다 붙일 라벨이다. 순서는 HEAD 브랜치, 로컬 브랜치, 원격 브랜치, 태그, 백업이다. 원격 브랜치가 같은 이름의 로컬
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
  for (const kind of ["tag", "backup"] as const) {
    for (const ref of refs.filter((candidate) => candidate.kind === kind)) {
      labelsAt(ref.target).push({ name: ref.name, kind: ref.kind, head: false, syncedRemotes: [] });
    }
  }
  return byCommit;
}

/** 스레드 목록에서 배지에 쓰는 값이다. */
export interface BadgeFolder {
  cwd: string;
  threads: readonly { id: string; title: string; updatedAt: number }[];
}

/** 커밋 옆에 붙이는 스레드 배지이다. 작업 트리 하나에 하나이다. */
export interface GitThreadBadge {
  /** 그 작업 트리의 가장 최근 스레드이다. 배지를 누르면 이 스레드를 연다. */
  threadId: string;
  title: string;
  /** 그 작업 트리의 스레드 수이다. */
  threadCount: number;
  worktreePath: string;
}

/** 폴더 글을 견줄 수 있게 맞춘다. Windows 의 드라이브 경로는 대소문자와 `\` 를 가리지 않는다. */
function comparablePath(path: string): string {
  const slashed = path.replace(/\\/g, "/").replace(/\/+$/, "");
  return /^[a-zA-Z]:\//.test(slashed) || slashed.startsWith("//") ? slashed.toLowerCase() : slashed;
}

/** 두 글이 같은 폴더인가. 구분자와 끝의 구분자를 가리지 않는다. */
export function samePath(left: string, right: string): boolean {
  return comparablePath(left) === comparablePath(right);
}

/** `folder` 가 든 작업 트리이다. 작업 트리가 겹치면 가장 깊은 것이다. 어느 작업 트리에도 없으면 `null` 이다. */
export function worktreeOf(folder: string, worktrees: readonly GitWorktree[]): GitWorktree | null {
  const path = comparablePath(folder);
  const owners = worktrees.filter((worktree) => {
    const root = comparablePath(worktree.path);
    return path === root || path.startsWith(`${root}/`);
  });
  return owners.sort((left, right) => comparablePath(right.path).length - comparablePath(left.path).length)[0] ?? null;
}

/** `folder` 와 같은 작업 트리에 폴더가 있는 스레드들의 id 이다. */
export function threadIdsInWorktreeOf(folder: string, worktrees: readonly GitWorktree[], folders: readonly BadgeFolder[]): string[] {
  const owner = worktreeOf(folder, worktrees);
  if (!owner) return [];
  return folders.filter((candidate) => worktreeOf(candidate.cwd, worktrees) === owner).flatMap((candidate) => candidate.threads.map((thread) => thread.id));
}

/**
 * 작업 트리가 체크아웃한 커밋마다 붙일 스레드 배지이다. 스레드는 그 스레드의 폴더가 든 작업 트리에 속한다. 작업
 * 트리가 겹치면 (작업 트리 안의 작업 트리) 가장 깊은 것에 속한다. 스레드가 없는 작업 트리는 배지가 없다.
 */
export function threadBadges(worktrees: readonly GitWorktree[], folders: readonly BadgeFolder[]): Map<string, GitThreadBadge[]> {
  const threadsByWorktree = new Map<GitWorktree, BadgeFolder["threads"][number][]>();
  for (const folder of folders) {
    const owner = worktreeOf(folder.cwd, worktrees);
    if (owner) threadsByWorktree.set(owner, [...(threadsByWorktree.get(owner) ?? []), ...folder.threads]);
  }
  const byCommit = new Map<string, GitThreadBadge[]>();
  for (const worktree of worktrees) {
    const threads = threadsByWorktree.get(worktree) ?? [];
    const latest = [...threads].sort((left, right) => right.updatedAt - left.updatedAt)[0];
    if (!worktree.head || !latest) continue;
    byCommit.set(worktree.head, [...(byCommit.get(worktree.head) ?? []), { threadId: latest.id, title: latest.title, threadCount: threads.length, worktreePath: worktree.path }]);
  }
  return byCommit;
}

/**
 * 읽은 줄에서 `query` 가 있는 줄의 자리이다. 제목, 작성자, 그 커밋의 라벨 이름에서 대소문자를 가리지 않고 찾는다.
 * 16 진수 4 자 이상이면 그 글로 시작하는 sha 도 찾는다. 빈 글이면 없다.
 */
export function findRows(rows: readonly GitGraphRow[], labels: ReadonlyMap<string, readonly GitRefLabel[]>, query: string): number[] {
  const needle = query.trim().toLowerCase();
  if (needle.length === 0) return [];
  const shaPrefix = /^[0-9a-f]{4,64}$/.test(needle);
  return rows.flatMap((row, index) => {
    const found =
      row.subject.toLowerCase().includes(needle) ||
      row.authorName.toLowerCase().includes(needle) ||
      (shaPrefix && row.id.startsWith(needle)) ||
      (labels.get(row.id)?.some((label) => label.name.toLowerCase().includes(needle)) ?? false);
    return found ? [index] : [];
  });
}

/** `from` 줄 다음의 찾은 줄이다. 끝을 넘으면 처음으로 돌아간다. `step` 이 -1 이면 앞쪽으로 간다. 찾은 줄이 없으면 `null` 이다. */
export function nextMatch(matches: readonly number[], from: number, step: 1 | -1): number | null {
  if (matches.length === 0) return null;
  if (step === 1) return matches.find((index) => index > from) ?? matches[0]!;
  return matches.findLast((index) => index < from) ?? matches[matches.length - 1]!;
}

/**
 * 줄을 바꾼 뒤의 스크롤 위치이다. 화면 맨 위에 있던 커밋이 새 줄에서도 같은 자리에 보이게 한다. 그 커밋이 없어졌으면
 * 그 아래의 커밋 가운데 남은 첫 커밋을 쓴다. 남은 커밋이 없으면 위치를 그대로 둔다. 맨 위에 있었으면 맨 위에 둔다:
 * 새 커밋은 위에 온다. 맨 위를 보는 사용자는 새 커밋을 봐야 한다.
 */
export function scrollTopAfterRowsChange(previous: readonly GitGraphRow[], next: readonly GitGraphRow[], scrollTop: number): number {
  if (scrollTop <= 0) return 0;
  const top = Math.floor(scrollTop / ROW_HEIGHT);
  const offset = scrollTop - top * ROW_HEIGHT;
  const nextIndex = new Map(next.map((row, index) => [row.id, index]));
  for (let index = top; index < previous.length; index += 1) {
    const kept = nextIndex.get(previous[index]!.id);
    if (kept !== undefined) return (kept - (index - top)) * ROW_HEIGHT + offset;
  }
  return scrollTop;
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

/** 시각을 이 컴퓨터의 날짜와 시간으로 쓴다. */
export function fullTime(unixSeconds: number): string {
  return unixSeconds > 0 ? new Date(unixSeconds * 1000).toLocaleString() : "";
}

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
