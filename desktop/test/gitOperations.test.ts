import { describe, expect, it } from "vitest";

import type { GitRef, GitWorktree } from "../src/renderer/git/gitGraph";
import {
  blockerText,
  canUndo,
  commitActions,
  executePlan,
  fetchOperations,
  fetchPlan,
  GIT_EXECUTE_METHOD,
  GIT_OPERATIONS_METHOD,
  GIT_PLAN_METHOD,
  GIT_UNDO_METHOD,
  type GitOperationRecord,
  noticeFrom,
  operationTitle,
  outcomeFrom,
  planFrom,
  planSummary,
  recordFrom,
  recordStatus,
  recordTitle,
  undoOperation,
} from "../src/renderer/git/gitOperations";

const sha = (letter: string) => letter.repeat(40);
const ref = (name: string, target: string, head = false, kind: GitRef["kind"] = "local_branch"): GitRef => ({ name, kind, target, head });
const worktree = (path: string, branch: string | null, head: string): GitWorktree => ({ path, branch, head });

const rawPlan = (overrides: Record<string, unknown> = {}) => ({
  operation: { kind: "reset", target: sha("a") },
  expectation: { head: { commit: sha("c"), branch: "main" }, branchCommit: null },
  branch: "main",
  before: { commit: sha("c"), branch: "main" },
  after: { commit: sha("a"), branch: "main" },
  impact: { dropped: 2, unreachable: 1, gained: 0 },
  blockers: [],
  ...overrides,
});

const rawRecord = (overrides: Record<string, unknown> = {}) => ({
  opId: "20261119T010203Z-0000002a",
  operation: { kind: "reset", target: sha("a") },
  worktree: "/repo",
  before: { commit: sha("c"), branch: "main" },
  after: { commit: sha("a"), branch: "main" },
  backupRef: "refs/akra/backup/main/20261119T010203Z-0000002a",
  status: "done",
  startedAt: 1_795_000_000,
  finishedAt: 1_795_000_001,
  error: null,
  undoOf: null,
  undoneBy: null,
  ...overrides,
});

describe("planFrom", () => {
  it("reads a plan and keeps the expectation as it came", () => {
    const plan = planFrom(rawPlan({ blockers: [{ code: "uncommitted_changes", message: "tracked files have changes" }, { message: "no code" }] }));
    expect(plan.operation).toEqual({ kind: "reset", target: sha("a") });
    expect(plan.expectation).toEqual({ head: { commit: sha("c"), branch: "main" }, branchCommit: null });
    expect(plan.impact).toEqual({ dropped: 2, unreachable: 1, gained: 0 });
    expect(plan.blockers).toEqual([{ code: "uncommitted_changes", message: "tracked files have changes" }]);
  });

  it("reads a plan that could not find its target", () => {
    const plan = planFrom(rawPlan({ branch: null, before: null, after: null, impact: null, blockers: [{ code: "not_repository", message: "" }] }));
    expect(plan).toMatchObject({ branch: null, before: null, after: null, impact: null });
  });

  it("refuses an answer without an operation or without the blocker list", () => {
    expect(() => planFrom(null)).toThrow();
    expect(() => planFrom(rawPlan({ operation: { kind: "push_force" } }))).toThrow();
    expect(() => planFrom(rawPlan({ blockers: undefined }))).toThrow();
  });
});

describe("records and outcomes", () => {
  it("reads a record and drops one with a missing field", () => {
    expect(recordFrom(rawRecord())).toMatchObject({ opId: "20261119T010203Z-0000002a", status: "done", backupRef: "refs/akra/backup/main/20261119T010203Z-0000002a", undoOf: null });
    expect(recordFrom(rawRecord({ status: "queued" }))).toBeNull();
    expect(recordFrom(rawRecord({ after: null }))).toBeNull();
    expect(recordFrom(null)).toBeNull();
  });

  it("reads each outcome and refuses an unknown one", () => {
    expect(outcomeFrom({ status: "done", record: rawRecord() }).status).toBe("done");
    expect(outcomeFrom({ status: "failed", record: rawRecord({ status: "failed", error: "an untracked file is in the way" }) })).toMatchObject({ status: "failed", record: { error: "an untracked file is in the way" } });
    expect(outcomeFrom({ status: "stale", plan: rawPlan() })).toMatchObject({ status: "stale", plan: { branch: "main" } });
    expect(outcomeFrom({ status: "refused", message: "changed" })).toEqual({ status: "refused", message: "changed" });
    expect(() => outcomeFrom({ status: "done" })).toThrow();
    expect(() => outcomeFrom({ status: "queued", record: rawRecord() })).toThrow();
  });

  it("offers undo only for a finished operation that is not undone and is not an undo", () => {
    const record = (overrides: Record<string, unknown>) => recordFrom(rawRecord(overrides)) as GitOperationRecord;
    expect(canUndo(record({}))).toBe(true);
    expect(canUndo(record({ undoneBy: "x" }))).toBe(false);
    expect(canUndo(record({ undoOf: "x" }))).toBe(false);
    expect(canUndo(record({ status: "failed" }))).toBe(false);
    expect(canUndo(record({ status: "started" }))).toBe(false);
    expect(recordStatus(record({ undoneBy: "x" }))).toBe("되돌림");
    expect(recordStatus(record({ status: "started" }))).toBe("끝을 적지 못함");
    expect(recordTitle(record({}))).toBe("reset: main → aaaaaaa");
    expect(recordTitle(record({ undoOf: "x" }))).toBe("되돌리기 (reset: main → aaaaaaa)");
  });
});

describe("requests", () => {
  it("sends the plan's operation and expectation back for the execution", async () => {
    const calls: { method: string; params: unknown }[] = [];
    const request = async <T>(method: string, params: unknown): Promise<T> => {
      calls.push({ method, params });
      if (method === GIT_PLAN_METHOD) return rawPlan() as T;
      if (method === GIT_OPERATIONS_METHOD) return { operations: [rawRecord(), { opId: "broken" }] } as T;
      return { status: "done", record: rawRecord() } as T;
    };
    const operation = { kind: "reset", target: sha("a") } as const;
    const plan = await fetchPlan(request, "/repo", operation);
    await executePlan(request, "/repo", plan);
    expect((await fetchOperations(request, "/repo")).map((record) => record.opId)).toEqual(["20261119T010203Z-0000002a"]);
    await undoOperation(request, "/repo", "20261119T010203Z-0000002a");
    expect(calls).toEqual([
      { method: GIT_PLAN_METHOD, params: { cwd: "/repo", operation } },
      { method: GIT_EXECUTE_METHOD, params: { cwd: "/repo", operation, expectation: { head: { commit: sha("c"), branch: "main" }, branchCommit: null } } },
      { method: GIT_OPERATIONS_METHOD, params: { cwd: "/repo" } },
      { method: GIT_UNDO_METHOD, params: { cwd: "/repo", opId: "20261119T010203Z-0000002a" } },
    ]);
  });
});

describe("commitActions", () => {
  const refs = [ref("main", sha("c"), true), ref("topic", sha("b")), ref("held", sha("b")), ref("here", sha("a")), ref("akra-agent/slot-1/task", sha("b")), ref("origin/topic", sha("a"), false, "remote_branch")];
  const worktrees = [worktree("/repo", "main", sha("c")), worktree("/repo-held", "held", sha("b"))];

  it("offers a reset, the branches of this commit, a detached checkout, and the branches that can move here", () => {
    const actions = commitActions(sha("a"), refs, sha("c"), worktrees);
    expect(actions.reset).toEqual({ operation: { kind: "reset", target: sha("a") }, branch: "main" });
    expect(actions.checkoutBranches.map((entry) => entry.branch)).toEqual(["here"]);
    expect(actions.checkoutCommit).toEqual({ kind: "checkout_commit", target: sha("a") });
    // `main` 은 체크아웃했고, `held` 는 다른 작업 트리가 체크아웃했고, `here` 는 이미 이 커밋이고, 병렬 슬롯의 브랜치는 뺀다.
    expect(actions.movableBranches).toEqual(["topic"]);
  });

  it("offers no reset at the checked-out commit and no branch that a worktree holds", () => {
    const atHead = commitActions(sha("c"), refs, sha("c"), worktrees);
    expect(atHead.reset).toBeNull();
    expect(atHead.checkoutBranches).toEqual([]);
    // 브랜치 위의 커밋은 브랜치 없이 체크아웃할 수 있다.
    expect(atHead.checkoutCommit).toEqual({ kind: "checkout_commit", target: sha("c") });

    const atHeld = commitActions(sha("b"), refs, sha("c"), worktrees);
    expect(atHeld.checkoutBranches.map((entry) => entry.branch)).toEqual(["topic"]);
  });

  it("offers no reset without a current branch and nothing that writes in a parallel slot worktree", () => {
    const detachedRefs = [ref("main", sha("c")), ref("topic", sha("b"))];
    const detached = commitActions(sha("c"), detachedRefs, sha("c"), [worktree("/repo", null, sha("c"))]);
    expect(detached.reset).toBeNull();
    expect(detached.checkoutCommit).toBeNull();
    expect(detached.checkoutBranches.map((entry) => entry.branch)).toEqual(["main"]);

    const slotRefs = [ref("akra-agent/slot-1/task", sha("c"), true), ref("topic", sha("b"))];
    const slot = commitActions(sha("b"), slotRefs, sha("c"), [worktree("/slot", "akra-agent/slot-1/task", sha("c"))]);
    expect(slot).toMatchObject({ reset: null, checkoutBranches: [], checkoutCommit: null });
  });
});

describe("texts", () => {
  it("names each operation", () => {
    expect(operationTitle({ kind: "reset", target: sha("a") }, null)).toBe("reset: 현재 브랜치 → aaaaaaa");
    expect(operationTitle({ kind: "move_branch", branch: "topic", target: sha("a") }, null)).toBe("브랜치 이동: topic → aaaaaaa");
    expect(operationTitle({ kind: "checkout_branch", branch: "topic" }, null)).toBe("체크아웃: topic");
    expect(operationTitle({ kind: "checkout_commit", target: sha("a") }, null)).toBe("체크아웃: aaaaaaa (브랜치 없이)");
  });

  it("writes the impact of a move and says what only the backup keeps", () => {
    const lines = planSummary(planFrom(rawPlan()));
    expect(lines[0]).toBe("main: ccccccc → aaaaaaa");
    expect(lines).toContain("브랜치에서 커밋 2개가 빠집니다.");
    expect(lines).toContain("그중 1개는 다른 브랜치와 태그가 닿지 못합니다. 백업 ref 가 지킵니다.");

    const reachable = planSummary(planFrom(rawPlan({ impact: { dropped: 1, unreachable: 0, gained: 3 } })));
    expect(reachable).toContain("빠지는 커밋은 모두 다른 브랜치나 태그에서 닿습니다.");
    expect(reachable).toContain("브랜치에 커밋 3개가 들어옵니다.");

    const forward = planSummary(planFrom(rawPlan({ operation: { kind: "move_branch", branch: "topic", target: sha("a") }, branch: "topic", impact: { dropped: 0, unreachable: 0, gained: 2 } })));
    expect(forward.some((line) => line.includes("빠집니다"))).toBe(false);
    expect(forward).toContain("작업 트리의 파일은 바뀌지 않습니다.");
  });

  it("writes a checkout with the branch names of both places", () => {
    const checkout = planFrom(rawPlan({ operation: { kind: "checkout_commit", target: sha("a") }, branch: null, after: { commit: sha("a"), branch: null }, impact: null }));
    expect(planSummary(checkout)[0]).toBe("HEAD: main (ccccccc) → aaaaaaa");
  });

  it("writes a known blocker in Korean and an unknown one with the Gateway text", () => {
    expect(blockerText({ code: "uncommitted_changes", message: "tracked files have changes" })).toContain("커밋하지 않은 변경");
    expect(blockerText({ code: "future_reason", message: "a new reason" })).toBe("a new reason");
    expect(blockerText({ code: "future_reason", message: "" })).toBe("future_reason");
  });

  it("writes a notice for each outcome and offers undo only after a finished operation", () => {
    const done = noticeFrom(outcomeFrom({ status: "done", record: rawRecord() }), false);
    expect(done).toMatchObject({ failed: false, text: "reset: main → aaaaaaa · 완료" });
    expect(done.record).not.toBeNull();
    const failed = noticeFrom(outcomeFrom({ status: "failed", record: rawRecord({ status: "failed", error: "file in the way" }) }), false);
    expect(failed).toMatchObject({ failed: true, record: null });
    expect(failed.text).toContain("file in the way");
    const stale = noticeFrom(outcomeFrom({ status: "stale", plan: rawPlan({ blockers: [{ code: "uncommitted_changes", message: "" }] }) }), true);
    expect(stale.text).toContain("되돌리지 않았습니다.");
    expect(stale.text).toContain("커밋하지 않은 변경");
    expect(noticeFrom({ status: "refused", message: "changed" }, true).failed).toBe(true);
  });
});
