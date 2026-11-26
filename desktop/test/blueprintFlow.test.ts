// 블루프린트의 사용자 흐름 계산(plan 33)이다: 전달 단계와 다음 행동, 주의 묶음과 이유, 다음 "내 차례".
import { describe, expect, it } from "vitest";

import { attentionOf, deliveryOf, nextInTurn } from "../src/renderer/blueprint/flow";
import type { MockThread } from "../src/renderer/blueprint/mockData";

const thread = (fields: Partial<MockThread> = {}): MockThread => ({
  id: "t",
  projectId: "p",
  title: "t",
  status: "idle",
  updated: "방금",
  branch: "b",
  worktree: true,
  added: 10,
  removed: 2,
  pullRequest: null,
  committed: false,
  pushed: false,
  criteria: null,
  pinned: false,
  done: false,
  snoozedUntil: null,
  ...fields,
});
const pullRequest = (checks: "pass" | "fail" | "pending", merged = false) => ({ number: 7, checks, merged });
const states = (item: MockThread, viewed = 0) => deliveryOf(item, viewed, 3)?.steps.map((step) => `${step.id}:${step.state}`);

describe("delivery steps", () => {
  it("has no steps for a thread without changes or a pull request", () => {
    expect(deliveryOf(thread({ added: 0, removed: 0 }), 0, 3)).toBeNull();
  });

  it("walks from review to merge, one next step at a time", () => {
    expect(states(thread())).toEqual(["review:active", "commit:todo", "push:todo", "pr:todo", "checks:todo", "merge:todo"]);
    expect(deliveryOf(thread(), 3, 3)?.next?.id).toBe("commit");
    expect(deliveryOf(thread({ committed: true }), 3, 3)?.action).toBe("push");
    expect(deliveryOf(thread({ committed: true, pushed: true }), 3, 3)?.action).toBe("PR 만들기");
    const waiting = deliveryOf(thread({ committed: true, pushed: true, pullRequest: pullRequest("pending") }), 3, 3);
    expect([waiting?.next?.label, waiting?.action]).toEqual(["CI 검사 중", null]);
    expect(deliveryOf(thread({ committed: true, pushed: true, pullRequest: pullRequest("pass") }), 3, 3)?.next?.id).toBe("merge");
    expect(deliveryOf(thread({ pullRequest: pullRequest("pass", true) }), 0, 3)?.next).toBeNull();
  });

  it("keeps a later step done when an earlier step is still open", () => {
    expect(states(thread({ committed: true, pushed: true }), 1)).toEqual(["review:active", "commit:done", "push:done", "pr:todo", "checks:todo", "merge:todo"]);
  });

  it("puts a failed check first, even when files are not viewed yet", () => {
    const failed = deliveryOf(thread({ committed: true, pushed: true, pullRequest: pullRequest("fail") }), 0, 3);
    expect([failed?.next?.id, failed?.next?.state, failed?.action]).toEqual(["checks", "blocked", "실패 기록 넘기기"]);
    expect(failed?.steps[0]).toMatchObject({ id: "review", state: "todo" });
  });

  it("shows the verify step only for a thread with criteria", () => {
    const criteria = { passed: 2, total: 3, missing: "화면 시험" };
    const withCriteria = deliveryOf(thread({ criteria }), 3, 3);
    expect([withCriteria?.next?.label, withCriteria?.hint]).toEqual(["검증 2/3", "증거가 없는 기준: 화면 시험"]);
    expect(states(thread())?.some((state) => state.startsWith("verify"))).toBe(false);
  });
});

describe("attention", () => {
  it("puts the cause before the next delivery step", () => {
    const delivery = deliveryOf(thread(), 3, 3);
    const approval = { command: "git push", cwd: "/w", reason: "" };
    expect(attentionOf(thread({ status: "approval" }), approval, delivery)).toEqual({ group: "mine", reason: "승인 필요: git push" });
    expect(attentionOf(thread({ status: "failed", pullRequest: pullRequest("fail") }), undefined, delivery)).toEqual({ group: "mine", reason: "PR #7 의 CI 실패" });
    expect(attentionOf(thread({ status: "unread" }), undefined, delivery)).toEqual({ group: "read", reason: "끝남 · 다음: 커밋" });
    expect(attentionOf(thread({ status: "running" }), undefined, delivery)).toEqual({ group: "working", reason: "" });
    expect(attentionOf(thread(), undefined, null)).toEqual({ group: "rest", reason: "" });
  });

  it("cycles through the threads that wait for the user and skips set-aside ones", () => {
    const threads = [
      thread({ id: "a", status: "approval" }),
      thread({ id: "b", status: "running" }),
      thread({ id: "c", status: "failed" }),
      thread({ id: "d", status: "failed", done: true }),
    ];
    expect([nextInTurn(threads, null), nextInTurn(threads, "a"), nextInTurn(threads, "c"), nextInTurn(threads, "b")]).toEqual(["a", "c", "a", "a"]);
    expect(nextInTurn([thread({ status: "running" })], null)).toBeNull();
  });
});

describe("unresolved PR attention", () => {
  it.each(["idle", "unread", "running"] as const)("keeps failed CI actionable while the turn is %s", (status) => {
    const item = thread({ status, pullRequest: pullRequest("fail") });
    expect(attentionOf(item, undefined, deliveryOf(item, 0, 3))).toEqual({ group: "mine", reason: "PR #7 의 CI 실패" });
    expect(nextInTurn([item], null)).toBe("t");
  });

  it("keeps approval first and does not revive a merged PR failure", () => {
    const item = thread({ status: "approval", pullRequest: pullRequest("fail") });
    expect(attentionOf(item, { command: "git push", cwd: "/w", reason: "" }, null).reason).toBe("승인 필요: git push");
    const merged = thread({ pullRequest: pullRequest("fail", true) });
    expect(attentionOf(merged, undefined, deliveryOf(merged, 0, 3)).group).toBe("rest");
    expect(nextInTurn([merged], null)).toBeNull();
  });
});
