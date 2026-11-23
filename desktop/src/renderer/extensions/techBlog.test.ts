import { describe, expect, it } from "vitest";

import { stageMarks, stateLabel, taskMarks, taskStages } from "./techBlogStages";

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
