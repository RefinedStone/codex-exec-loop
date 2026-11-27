import { describe, expect, it } from "vitest";

import { activityTitle, blogActivity, finishedArticles, jobArticle, stageMarks, stateLabel, taskMarks, taskStages } from "./techBlogStages";

describe("stageMarks", () => {
  it("marks earlier stages done and waits on the optional rewrite", () => {
    expect(stageMarks("write", 0, false)).toEqual(["done", "active", "wait", "wait"]);
  });
  it("shows the second review after one rewrite", () => {
    expect(stageMarks("review", 1, false)).toEqual(["done", "done", "active", "done"]);
  });
  it("marks the stage where the job stopped", () => {
    expect(stageMarks("revise", 0, true)).toEqual(["done", "done", "done", "failed"]);
  });
  it("leaves the rewrite waiting when the article passed the first review", () => {
    expect(stageMarks(null, 0, false)).toEqual(["done", "done", "done", "wait"]);
  });
});

describe("stateLabel", () => {
  it("counts findings", () => expect(stateLabel("findings", 2)).toBe("지적 2건"));
});

describe("taskMarks", () => {
  it("shows only the rewrite and its review for a manual rewrite", () => {
    expect(taskStages("revise").map(s => s.id)).toEqual(["revise", "review"]);
    expect(taskMarks("revise", "review", 1, false)).toEqual(["done", "active"]);
    expect(taskMarks("revise", "revise", 1, true)).toEqual(["failed", "wait"]);
  });
  it("shows one step for a manual review", () => {
    expect(taskMarks("review", "review", 1, false)).toEqual(["active"]);
  });
});

describe("blog activity", () => {
  const row = (id: string, state: "running" | "reviewed" | "stopped", unread = false) =>
    ({ id, title: id, state, stage: null, task: "generate" as const, updated: 1, unread });
  it("counts running and unread articles for the top bar", () => {
    const activity = blogActivity([row("a", "running"), row("b", "reviewed", true), row("c", "stopped", true)]);
    expect(activity).toEqual({ running: 1, unread: 2, stopped: 1, next: "b" });
    expect(blogActivity([{ ...row("d", "reviewed", true), failed: true }]).stopped).toBe(1);
    expect(activityTitle(activity)).toBe("기술 블로그 · 글을 만드는 중");
    expect(activityTitle({ running: 0, unread: 1, stopped: 0 })).toBe("기술 블로그 · 새 글 1개");
  });
  it("reports only articles that finished since the last list and are unread", () => {
    const before = [row("a", "running"), row("b", "running"), row("c", "reviewed", true)];
    const after = [row("a", "reviewed", true), row("b", "stopped"), row("c", "reviewed", true)];
    expect(finishedArticles(before, after).map(a => a.id)).toEqual(["a"]);
  });
});

describe("jobArticle", () => {
  const instant = () => Promise.resolve();
  it("returns the article that the job reports in progress", async () => {
    const states = [{ id: "j", state: "running", progress: null }, { id: "j", state: "running", progress: { article_id: "a1" } }];
    const call = (async () => states.shift()) as never;
    await expect(jobArticle(call, { id: "j", state: "queued" }, instant)).resolves.toBe("a1");
  });
  it("raises the worker rejection of a failed job", async () => {
    const call = (async () => ({ id: "j", state: "failed", error: "다른 창에서 글이 바뀌었습니다." })) as never;
    await expect(jobArticle(call, { id: "j", state: "queued" }, instant)).rejects.toThrow("다른 창에서");
  });
  it("stops waiting when the job never starts", async () => {
    const call = (async () => ({ id: "j", state: "queued" })) as never;
    await expect(jobArticle(call, { id: "j", state: "queued" }, instant)).rejects.toThrow("아직 시작되지");
  });
});
