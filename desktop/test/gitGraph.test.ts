import { describe, expect, it } from "vitest";

import { appendPage, fetchGraphPage, GIT_GRAPH_METHOD, type GitRef, graphPageFrom, headName, loadedFromFirstPage, refLabels, relativeTime } from "../src/renderer/git/gitGraph";

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
