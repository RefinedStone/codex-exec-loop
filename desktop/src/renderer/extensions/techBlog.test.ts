import { describe, expect, it } from "vitest";

import { stageMarks, stateLabel } from "./techBlogStages";

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
