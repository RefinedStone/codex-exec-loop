import { describe, expect, it } from "vitest";

import {
  appendPage,
  commitDetailFrom,
  fetchCommitDetail,
  fetchGraphPage,
  fetchGraphRows,
  fetchRefsDigest,
  findRows,
  GIT_COMMIT_METHOD,
  GIT_GRAPH_METHOD,
  GIT_REFS_METHOD,
  type GitRef,
  graphPageFrom,
  headName,
  loadedFromFirstPage,
  nextMatch,
  refLabels,
  relativeTime,
  scrollTopAfterRowsChange,
  threadBadges,
} from "../src/renderer/git/gitGraph";
import { ROW_HEIGHT } from "../src/renderer/git/graphGeometry";

const row = (overrides: Record<string, unknown>) => ({
  id: "a".repeat(40),
  parents: ["b".repeat(40)],
  authorName: "Author",
  authorTime: 1_700_000_000,
  subject: "subject",
  lane: 0,
  lanesIn: [],
  lanesOut: [0],
  lanesThrough: [],
  ...overrides,
});

const ref = (name: string, kind: GitRef["kind"], target: string, head = false): GitRef => ({ name, kind, target, head });

describe("graphPageFrom", () => {
  it("reads rows and refs and drops entries with missing fields", () => {
    const page = graphPageFrom({
      repository: true,
      stale: false,
      head: "a".repeat(40),
      refs: [ref("main", "local_branch", "a".repeat(40), true), { name: "x", kind: "stash", target: "c" }, { name: "", kind: "tag", target: "c" }, null],
      rows: [row({}), row({ id: "" }), row({ lane: -1 }), row({ id: "c", lanesThrough: [1, "2", -3, 4.5] }), null],
      next: { skip: 2 },
    });
    expect(page.rows.map((entry) => entry.id)).toEqual(["a".repeat(40), "c"]);
    expect(page.rows[1]!.lanesThrough).toEqual([1]);
    expect(page.refs).toEqual([ref("main", "local_branch", "a".repeat(40), true)]);
    expect(page.next).toEqual({ skip: 2 });
  });

  it("reads a page of a folder that is not a repository", () => {
    expect(graphPageFrom({ repository: false, stale: false, head: null, refs: [], rows: [], next: null })).toMatchObject({ repository: false, head: null, next: null });
  });

  it("refuses an answer that is not a page", () => {
    expect(() => graphPageFrom(null)).toThrow();
    expect(() => graphPageFrom({ repository: true, rows: [] })).toThrow();
  });
});

describe("fetchGraphPage", () => {
  it("sends the folder for the first page and returns the cursor unchanged for the next page", async () => {
    const calls: { method: string; params: unknown }[] = [];
    const request = async <T>(method: string, params: unknown): Promise<T> => {
      calls.push({ method, params });
      return { repository: true, stale: false, head: null, refs: [], rows: [], next: null } as T;
    };
    await fetchGraphPage(request, "/repo", null);
    await fetchGraphPage(request, "/repo", { skip: 300, lanes: ["a", null] });
    expect(calls).toEqual([
      { method: GIT_GRAPH_METHOD, params: { cwd: "/repo" } },
      { method: GIT_GRAPH_METHOD, params: { cwd: "/repo", cursor: { skip: 300, lanes: ["a", null] } } },
    ]);
  });
});

describe("graph pages", () => {
  it("appends the next page and keeps the refs of the first page", () => {
    const first = loadedFromFirstPage("/repo", graphPageFrom({ repository: true, head: "a", refs: [ref("main", "local_branch", "a", true)], rows: [row({ id: "a" })], next: { skip: 1 } }));
    const loaded = appendPage({ ...first, loadingMore: true }, graphPageFrom({ repository: true, head: "a", refs: [], rows: [row({ id: "b" })], next: null }));
    expect(loaded.rows.map((entry) => entry.id)).toEqual(["a", "b"]);
    expect(loaded.refs).toHaveLength(1);
    expect(loaded).toMatchObject({ next: null, loadingMore: false });
  });
});

describe("refLabels", () => {
  it("merges a remote branch into the local branch of the same name on the same commit", () => {
    const labels = refLabels([ref("main", "local_branch", "a", true), ref("origin/main", "remote_branch", "a"), ref("fork/main", "remote_branch", "a"), ref("v1", "tag", "a")], "a");
    expect(labels.get("a")).toEqual([
      { name: "main", kind: "local_branch", head: true, syncedRemotes: ["origin", "fork"] },
      { name: "v1", kind: "tag", head: false, syncedRemotes: [] },
    ]);
  });

  it("keeps a remote branch that is on another commit or has no local branch", () => {
    const labels = refLabels([ref("main", "local_branch", "a", true), ref("origin/main", "remote_branch", "b"), ref("origin/topic", "remote_branch", "a")], "a");
    expect(labels.get("b")).toEqual([{ name: "origin/main", kind: "remote_branch", head: false, syncedRemotes: [] }]);
    expect(labels.get("a")!.map((label) => label.name)).toEqual(["main", "origin/topic"]);
  });

  it("puts the checked-out branch first and adds a HEAD label when no branch is checked out", () => {
    const refs = [ref("alpha", "local_branch", "a"), ref("zeta", "local_branch", "a", true)];
    expect(refLabels(refs, "a").get("a")!.map((label) => label.name)).toEqual(["zeta", "alpha"]);
    expect(headName(refs, "a")).toBe("zeta");

    const detached = [ref("alpha", "local_branch", "a")];
    expect(refLabels(detached, "abcdef0123").get("abcdef0123")).toEqual([{ name: "HEAD", kind: "head", head: true, syncedRemotes: [] }]);
    expect(headName(detached, "abcdef0123")).toBe("abcdef0");
    expect(headName([], null)).toBeNull();
  });
});

describe("G2 page fields", () => {
  it("reads the digest, the worktrees, the backup mark, and a backup ref", () => {
    const page = graphPageFrom({
      repository: true,
      head: "a",
      refsDigest: "d1",
      refs: [ref("main/op-1", "backup", "b")],
      worktrees: [{ path: "C:/repo", head: "a", branch: "main" }, { path: "C:/repo-x", head: "b" }, { head: "c" }],
      rows: [row({ id: "a" }), row({ id: "b", backupOnly: true })],
      next: null,
    });
    expect(page.refsDigest).toBe("d1");
    expect(page.worktrees).toEqual([
      { path: "C:/repo", head: "a", branch: "main" },
      { path: "C:/repo-x", head: "b", branch: null },
    ]);
    expect(page.rows.map((entry) => entry.backupOnly)).toEqual([false, true]);
    expect(refLabels(page.refs, "a").get("b")).toEqual([{ name: "main/op-1", kind: "backup", head: false, syncedRemotes: [] }]);
  });

  it("reads an answer of an older Gateway without the new fields", () => {
    const page = graphPageFrom({ repository: true, head: "a", refs: [], rows: [row({ id: "a" })], next: null });
    expect(page).toMatchObject({ refsDigest: null, worktrees: [] });
    expect(page.rows[0]!.backupOnly).toBe(false);
  });
});

describe("fetchGraphRows", () => {
  const pageOf = (ids: string[], next: unknown, stale = false) => ({ repository: true, stale, head: "c5", refsDigest: "d", refs: [], rows: ids.map((id) => row({ id })), next });

  it("reads pages until it has the rows that were on the screen", async () => {
    const calls: unknown[] = [];
    const answers = [pageOf(["c5", "c4"], { skip: 2 }), pageOf(["c3", "c2"], { skip: 4 }), pageOf(["c1"], null)];
    const request = async <T>(_method: string, params: unknown): Promise<T> => {
      calls.push(params);
      return answers[calls.length - 1] as T;
    };
    const loaded = await fetchGraphRows(request, "/repo", 3);
    expect(loaded.rows.map((entry) => entry.id)).toEqual(["c5", "c4", "c3", "c2"]);
    expect(loaded.next).toEqual({ skip: 4 });
    expect(calls).toEqual([
      { cwd: "/repo", limit: 3 },
      { cwd: "/repo", cursor: { skip: 2 }, limit: 1 },
    ]);
  });

  it("fails when the repository changes again between the pages", async () => {
    const answers = [pageOf(["c5"], { skip: 1 }), pageOf([], null, true)];
    let call = 0;
    const request = async <T>(): Promise<T> => answers[call++] as T;
    await expect(fetchGraphRows(request, "/repo", 5)).rejects.toThrow();
  });
});

describe("refs digest and commit detail", () => {
  it("asks the Gateway for the digest of the folder", async () => {
    const calls: { method: string; params: unknown }[] = [];
    const request = async <T>(method: string, params: unknown): Promise<T> => {
      calls.push({ method, params });
      return (method === GIT_REFS_METHOD ? { repository: true, refsDigest: "d2" } : { commit: null }) as T;
    };
    expect(await fetchRefsDigest(request, "/repo")).toBe("d2");
    expect(await fetchCommitDetail(request, "/repo", "abc1234")).toBeNull();
    expect(calls).toEqual([
      { method: GIT_REFS_METHOD, params: { cwd: "/repo" } },
      { method: GIT_COMMIT_METHOD, params: { cwd: "/repo", commit: "abc1234" } },
    ]);
  });

  it("reads a commit detail and refuses an unknown shape", () => {
    const detail = commitDetailFrom({ commit: { id: "abc", parents: ["p1", 2], authorName: "A", authorEmail: "a@x", authorTime: 10, committerName: "C", committerTime: 11, message: "s\n\nb", diff: "diff --git a/f b/f", diffTooLarge: false } });
    expect(detail).toEqual({ id: "abc", parents: ["p1"], authorName: "A", authorEmail: "a@x", authorTime: 10, committerName: "C", committerTime: 11, message: "s\n\nb", diff: "diff --git a/f b/f", diffTooLarge: false });
    expect(commitDetailFrom({ commit: { id: "abc", diffTooLarge: true } })).toMatchObject({ diff: "", diffTooLarge: true, parents: [] });
    expect(() => commitDetailFrom({})).toThrow();
    expect(() => commitDetailFrom({ commit: { message: "no id" } })).toThrow();
  });
});

describe("threadBadges", () => {
  const worktrees = [
    { path: "C:/dev/repo", head: "a", branch: "main" },
    { path: "C:/dev/repo/nested", head: "b", branch: "nested" },
    { path: "C:/dev/repo-topic", head: "c", branch: "topic" },
    { path: "C:/dev/bare", head: null, branch: null },
  ];
  const thread = (id: string, updatedAt: number) => ({ id, title: `title ${id}`, updatedAt });

  it("puts the newest thread of each worktree on the commit that worktree checked out", () => {
    const badges = threadBadges(worktrees, [
      { cwd: "c:\\dev\\repo\\", threads: [thread("t1", 10), thread("t2", 30)] },
      { cwd: "C:\\dev\\repo\\src", threads: [thread("t3", 20)] },
      { cwd: "C:\\dev\\repo\\nested\\deep", threads: [thread("t4", 5)] },
      { cwd: "C:\\dev\\repo-topic-other", threads: [thread("t5", 99)] },
      { cwd: "C:\\dev\\bare", threads: [thread("t6", 1)] },
    ]);
    expect(badges.get("a")).toEqual([{ threadId: "t2", title: "title t2", threadCount: 3, worktreePath: "C:/dev/repo" }]);
    // 겹친 작업 트리는 가장 깊은 것이 스레드를 가진다.
    expect(badges.get("b")).toEqual([{ threadId: "t4", title: "title t4", threadCount: 1, worktreePath: "C:/dev/repo/nested" }]);
    // 이름이 비슷한 다른 폴더는 그 작업 트리가 아니다. 커밋이 없는 작업 트리는 배지가 없다.
    expect(badges.has("c")).toBe(false);
    expect(badges.size).toBe(2);
  });

  it("compares POSIX paths with their case", () => {
    const badges = threadBadges([{ path: "/home/u/Repo", head: "a", branch: null }], [{ cwd: "/home/u/repo", threads: [thread("t1", 1)] }, { cwd: "/home/u/Repo/sub", threads: [thread("t2", 1)] }]);
    expect(badges.get("a")!.map((badge) => badge.threadId)).toEqual(["t2"]);
  });
});

describe("findRows", () => {
  const rows = graphPageFrom({
    repository: true,
    head: "abcd1111",
    refs: [],
    rows: [row({ id: "abcd1111", subject: "Fix the Parser", authorName: "Kim" }), row({ id: "ffff2222", subject: "Add docs", authorName: "Lee" }), row({ id: "0123beef", subject: "parser tests", authorName: "Park" })],
    next: null,
  }).rows;
  const labels = refLabels([ref("feature/parser-v2", "local_branch", "ffff2222")], null);

  it("finds in the subject, the author, the ref labels, and the sha prefix", () => {
    expect(findRows(rows, labels, " PARSER ")).toEqual([0, 1, 2]);
    expect(findRows(rows, labels, "lee")).toEqual([1]);
    expect(findRows(rows, labels, "0123")).toEqual([2]);
    // 16 진수가 아니거나 4 자보다 짧으면 sha 로 보지 않는다.
    expect(findRows(rows, labels, "abc")).toEqual([]);
    expect(findRows(rows, labels, "")).toEqual([]);
  });

  it("steps through the matches and wraps at both ends", () => {
    expect(nextMatch([2, 5, 9], -1, 1)).toBe(2);
    expect(nextMatch([2, 5, 9], 5, 1)).toBe(9);
    expect(nextMatch([2, 5, 9], 9, 1)).toBe(2);
    expect(nextMatch([2, 5, 9], 5, -1)).toBe(2);
    expect(nextMatch([2, 5, 9], 2, -1)).toBe(9);
    expect(nextMatch([], 0, 1)).toBeNull();
  });
});

describe("scrollTopAfterRowsChange", () => {
  const rowsOf = (...ids: string[]) => ids.map((id) => ({ ...graphPageFrom({ repository: true, refs: [], rows: [row({ id })], next: null }).rows[0]! }));

  it("keeps the top commit at the same place when new commits arrive above it", () => {
    const previous = rowsOf("c3", "c2", "c1");
    const next = rowsOf("c5", "c4", "c3", "c2", "c1");
    expect(scrollTopAfterRowsChange(previous, next, ROW_HEIGHT + 10)).toBe(3 * ROW_HEIGHT + 10);
    // 뒤에 줄을 이은 때는 그대로이다.
    expect(scrollTopAfterRowsChange(previous, [...previous, ...rowsOf("c0")], ROW_HEIGHT + 10)).toBe(ROW_HEIGHT + 10);
  });

  it("uses the next commit that is still there, and keeps the position when none is", () => {
    const previous = rowsOf("gone", "c2", "c1");
    expect(scrollTopAfterRowsChange(previous, rowsOf("c9", "c2", "c1"), 4)).toBe(4);
    expect(scrollTopAfterRowsChange(previous, rowsOf("c9", "c8", "c2"), 4)).toBe(ROW_HEIGHT + 4);
    expect(scrollTopAfterRowsChange(previous, rowsOf("x", "y"), 40)).toBe(40);
  });
});

describe("relativeTime", () => {
  const now = 1_700_000_000_000;
  it("uses short units and falls back to the date after thirty days", () => {
    expect(relativeTime(1_700_000_000 - 5, now)).toBe("방금");
    expect(relativeTime(1_700_000_000 - 5 * 60, now)).toBe("5분 전");
    expect(relativeTime(1_700_000_000 - 3 * 3600, now)).toBe("3시간 전");
    expect(relativeTime(1_700_000_000 - 2 * 86400, now)).toBe("2일 전");
    expect(relativeTime(1_700_000_000 - 40 * 86400, now)).toBe("2023-10-05");
    expect(relativeTime(0, now)).toBe("");
    // 시계가 어긋나 미래의 시각이 와도 음수를 쓰지 않는다.
    expect(relativeTime(1_700_000_000 + 60, now)).toBe("방금");
  });
});
