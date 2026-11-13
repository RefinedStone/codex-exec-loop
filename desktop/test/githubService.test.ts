import { describe, expect, it } from "vitest";

import {
  issuePrompt,
  parseIssueUrl,
  parseProjectPage,
  parseProjectRef,
  parsePullRequestUrl,
  parseSummary,
  readGitHubSummary,
  readIssuePrompt,
  readProject,
  readReviewPrompt,
  reviewPrompt,
} from "../src/main/githubService";

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

  it("says when a thread or the PR has more than one page", () => {
    const json = JSON.stringify({
      data: {
        repository: {
          pullRequest: {
            url: "https://github.com/acme/app/pull/7",
            title: "t",
            reviewThreads: {
              totalCount: 130,
              nodes: [{ isResolved: false, path: "a.ts", line: 1, comments: { totalCount: 25, nodes: [{ author: { login: "k" }, body: "first" }] } }],
            },
          },
        },
      },
    });
    const prompt = reviewPrompt(json);
    expect(prompt).toContain("코멘트 25개 중 앞의 1개만");
    expect(prompt).toContain("리뷰 스레드 130개 중 앞의 1개만");
    expect(reviewPrompt(json.replace('"totalCount":25', '"totalCount":1').replace('"totalCount":130', '"totalCount":1'))).not.toContain("만 넣었습니다");
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

describe("GitHub Project", () => {
  const issue = (number: number, repository: string, state = "OPEN", status: string | null = "Todo") => ({
    fieldValueByName: status ? { name: status } : null,
    content: { __typename: "Issue", number, title: `Issue ${number}`, url: `https://github.com/${repository}/issues/${number}`, state, repository: { nameWithOwner: repository } },
  });
  const page = (nodes: unknown[], endCursor: string | null = null) =>
    JSON.stringify({
      data: {
        repositoryOwner: {
          projectV2: { title: "Akra", url: "https://github.com/users/sample/projects/1", items: { pageInfo: { hasNextPage: endCursor !== null, endCursor }, nodes } },
        },
      },
    });

  it("accepts owner/number and project addresses only", () => {
    expect(parseProjectRef("sample/1")).toEqual({ owner: "sample", number: 1 });
    expect(parseProjectRef(" https://github.com/users/sample/projects/12/views/3 ")).toEqual({ owner: "sample", number: 12 });
    expect(parseProjectRef("https://github.com/orgs/acme-co/projects/4")).toEqual({ owner: "acme-co", number: 4 });
    expect(parseProjectRef("sample/0")).toBeNull();
    expect(parseProjectRef("sample/widgets")).toBeNull();
    expect(parseProjectRef("https://evil.example/users/sample/projects/1")).toBeNull();
    expect(parseProjectRef("https://github.com/sample/widgets/projects/1")).toBeNull();
    expect(parseProjectRef(1)).toBeNull();
  });

  it("keeps open issues, pull requests and drafts, and counts closed items", () => {
    const read = parseProjectPage(
      page([
        issue(1, "sample/widgets"),
        issue(2, "sample/widgets", "CLOSED"),
        { fieldValueByName: { name: "Done" }, content: { __typename: "PullRequest", number: 3, title: "PR", url: "https://github.com/sample/lib/pull/3", state: "MERGED", repository: { nameWithOwner: "sample/lib" } } },
        { fieldValueByName: { name: "In progress" }, content: { __typename: "PullRequest", number: 4, title: "PR", url: "https://github.com/sample/lib/pull/4", state: "OPEN", repository: { nameWithOwner: "sample/lib" } } },
        { fieldValueByName: null, content: { __typename: "DraftIssue", title: "Idea" } },
        { fieldValueByName: null, content: null },
        { content: { __typename: "Issue", number: 5, title: "x", url: "https://evil.example/issues/5", state: "OPEN" } },
      ]),
    );
    expect(read.title).toBe("Akra");
    expect(read.closedCount).toBe(2);
    expect(read.nextCursor).toBeNull();
    expect(read.items).toEqual([
      { kind: "issue", title: "Issue 1", url: "https://github.com/sample/widgets/issues/1", repository: "sample/widgets", number: 1, status: "Todo" },
      { kind: "pullRequest", title: "PR", url: "https://github.com/sample/lib/pull/4", repository: "sample/lib", number: 4, status: "In progress" },
      { kind: "draft", title: "Idea", url: null, repository: null, number: null, status: null },
    ]);
  });

  it("reports a missing project", () => {
    expect(() => parseProjectPage(JSON.stringify({ data: { repositoryOwner: { projectV2: null } } }))).toThrow("Project 를 찾을 수 없습니다");
    expect(() => parseProjectPage(JSON.stringify({ data: { repositoryOwner: null } }))).toThrow("Project 를 찾을 수 없습니다");
  });

  it("follows the cursor and stops at the page limit", async () => {
    const calls: string[][] = [];
    const two = await readProject("sample/1", async (args) => {
      calls.push(args);
      return calls.length === 1 ? page([issue(1, "sample/a")], "c1") : page([issue(2, "sample/b")]);
    });
    expect(two.items.map((item) => item.number)).toEqual([1, 2]);
    expect(two.truncated).toBe(false);
    expect(calls[0]).toEqual(expect.arrayContaining(["owner=sample", "number=1"]));
    expect(calls[0]!.some((arg) => arg.startsWith("cursor="))).toBe(false);
    expect(calls[1]).toEqual(expect.arrayContaining(["cursor=c1"]));

    let count = 0;
    const endless = await readProject("sample/1", async () => page([issue((count += 1), "sample/a")], `c${count}`));
    expect(endless.items).toHaveLength(5);
    expect(endless.truncated).toBe(true);
  });

  it("refuses a bad project before calling gh", async () => {
    let called = false;
    await expect(
      readProject("not a project", async () => {
        called = true;
        return "";
      }),
    ).rejects.toThrow("owner/번호");
    expect(called).toBe(false);
  });
});

describe("issue hand-off prompt", () => {
  const issueJson = (comments: unknown[], totalCount = comments.length, body: string | null = "Steps to reproduce") =>
    JSON.stringify({
      data: {
        repository: {
          nameWithOwner: "sample/widgets",
          issue: { number: 42, url: "https://github.com/sample/widgets/issues/42", title: "Crash on start", body, comments: { totalCount, nodes: comments } },
        },
      },
    });

  it("accepts github.com issue addresses only", () => {
    expect(parseIssueUrl("https://github.com/sample/widgets/issues/42")).toEqual({ owner: "sample", name: "widgets", number: 42 });
    expect(parseIssueUrl("https://github.com/sample/widgets/pull/42")).toBeNull();
    expect(parseIssueUrl("https://evil.example/sample/widgets/issues/42")).toBeNull();
  });

  it("puts the issue, its body, recent comments and the closing keyword in the message", () => {
    const prompt = issuePrompt(issueJson([{ author: { login: "kim" }, body: "Also on Windows." }], 12));
    expect(prompt).toContain("Closes sample/widgets#42");
    expect(prompt).toContain("https://github.com/sample/widgets/issues/42 (Crash on start)");
    expect(prompt).toContain("Steps to reproduce");
    expect(prompt).toContain("kim: Also on Windows.");
    expect(prompt).toContain("댓글 12개 중 최근 1개");
  });

  it("marks an empty body and clips long text", () => {
    expect(issuePrompt(issueJson([], 0, null))).toContain("(본문 없음)");
    expect(issuePrompt(issueJson([], 0, "x".repeat(30_000))).length).toBeLessThanOrEqual(20_000 + 10);
  });

  it("asks gh for the issue and refuses other addresses", async () => {
    const calls: string[][] = [];
    await readIssuePrompt("https://github.com/sample/widgets/issues/42", async (args) => (calls.push(args), issueJson([])));
    expect(calls[0]).toEqual(expect.arrayContaining(["owner=sample", "name=widgets", "number=42"]));
    await expect(readIssuePrompt("https://github.com/sample/widgets/pull/42", async () => "")).rejects.toThrow("이슈 주소");
    expect(() => issuePrompt(JSON.stringify({ data: { repository: { issue: null } } }))).toThrow("이슈를 찾을 수 없습니다");
  });
});
