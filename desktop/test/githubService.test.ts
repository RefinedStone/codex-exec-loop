import { describe, expect, it } from "vitest";

import { parsePullRequestUrl, parseSummary, readGitHubSummary, readReviewPrompt, reviewPrompt } from "../src/main/githubService";

const summaryJson = JSON.stringify({
  data: {
    viewer: {
      login: "octo",
      pullRequests: {
        nodes: [
          {
            number: 7,
            title: "Fix login",
            url: "https://github.com/acme/app/pull/7",
            isDraft: false,
            updatedAt: "2026-11-05T10:00:00Z",
            reviewDecision: "CHANGES_REQUESTED",
            repository: { nameWithOwner: "acme/app" },
            commits: { nodes: [{ commit: { statusCheckRollup: { state: "FAILURE" } } }] },
            reviewThreads: { nodes: [{ isResolved: false }, { isResolved: true }, { isResolved: false }] },
          },
          { number: 8, title: "No checks", url: "https://github.com/acme/app/pull/8", repository: { nameWithOwner: "acme/app" }, commits: { nodes: [{ commit: { statusCheckRollup: null } }] } },
          { number: 9, title: "Not GitHub", url: "https://evil.example/pull/9" },
        ],
      },
    },
    reviewRequests: { nodes: [{ number: 3, title: "Add API", url: "https://github.com/acme/api/pull/3", isDraft: true, repository: { nameWithOwner: "acme/api" }, author: { login: "kim" } }, {}] },
  },
});

describe("GitHub summary", () => {
  it("reads my pull requests with checks and unresolved review threads", () => {
    const summary = parseSummary(summaryJson);
    expect(summary.login).toBe("octo");
    expect(summary.myPullRequests.map((pull) => [pull.number, pull.checks, pull.unresolvedThreads])).toEqual([
      [7, "FAILURE", 2],
      [8, null, null],
    ]);
    expect(summary.reviewRequests).toEqual([
      expect.objectContaining({ number: 3, repository: "acme/api", author: "kim", isDraft: true, unresolvedThreads: null }),
    ]);
  });

  it("asks gh for one GraphQL query", async () => {
    const calls: string[][] = [];
    await readGitHubSummary(async (args) => (calls.push(args), summaryJson));
    expect(calls).toHaveLength(1);
    expect(calls[0]!.slice(0, 3)).toEqual(["api", "graphql", "-f"]);
  });
});

describe("hand-off prompt", () => {
  const threadsJson = (threads: unknown[]) =>
    JSON.stringify({ data: { repository: { pullRequest: { url: "https://github.com/acme/app/pull/7", title: "Fix login", reviewThreads: { nodes: threads } } } } });

  it("lists only unresolved threads with file, line and comments", () => {
    const prompt = reviewPrompt(
      threadsJson([
        { isResolved: false, path: "src/a.ts", line: 12, comments: { nodes: [{ author: { login: "kim" }, body: "Handle null here." }, { author: { login: "octo" }, body: "ok" }] } },
        { isResolved: true, path: "src/b.ts", line: 3, comments: { nodes: [{ author: { login: "kim" }, body: "resolved" }] } },
      ]),
    );
    expect(prompt).toContain("PR: https://github.com/acme/app/pull/7 (Fix login)");
    expect(prompt).toContain("1. src/a.ts:12\nkim: Handle null here.\nocto: ok");
    expect(prompt).not.toContain("resolved");
  });

  it("clips very long comments", () => {
    const prompt = reviewPrompt(threadsJson([{ isResolved: false, path: "a", line: 1, comments: { nodes: [{ author: { login: "k" }, body: "x".repeat(5000) }] } }]));
    expect(prompt.length).toBeLessThan(2_300);
    expect(prompt).toContain("…(생략)");
  });

  it("accepts only github.com pull request URLs and passes them as variables", async () => {
    expect(parsePullRequestUrl("https://github.com/acme/app/pull/7")).toEqual({ owner: "acme", name: "app", number: 7 });
    for (const bad of ["https://github.com/acme/app/issues/7", "https://evil.example/acme/app/pull/7", "https://github.com/a/b/pull/7?x=1", 7]) {
      expect(parsePullRequestUrl(bad)).toBeNull();
    }
    await expect(readReviewPrompt("https://evil.example/x", async () => "")).rejects.toThrow();
    const calls: string[][] = [];
    await readReviewPrompt("https://github.com/acme/app/pull/7", async (args) => (calls.push(args), threadsJson([])));
    expect(calls[0]).toEqual(expect.arrayContaining(["owner=acme", "name=app", "number=7"]));
  });
});
