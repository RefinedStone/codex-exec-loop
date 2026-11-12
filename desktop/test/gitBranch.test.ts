// 브랜치의 커밋과 PR 읽기이다 (R10). 실제 git 저장소와 가짜 gh 로 시험한다. 네트워크를 쓰지 않는다.
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterAll, describe, expect, it } from "vitest";

import { checksRollup, parseCommitLog, parseGitHubRemote, readBranchSummary, readCommitDiff } from "../src/main/gitBranch";
import type { GhRunner } from "../src/main/githubService";

describe("branch summary parsing", () => {
  it("reads owner/name from GitHub remote addresses only", () => {
    expect(parseGitHubRemote("https://github.com/sample/widgets.git")).toBe("sample/widgets");
    expect(parseGitHubRemote("https://github.com/sample/widgets")).toBe("sample/widgets");
    expect(parseGitHubRemote("git@github.com:sample/wid.gets.git\n")).toBe("sample/wid.gets");
    expect(parseGitHubRemote("ssh://git@github.com/sample/widgets.git")).toBe("sample/widgets");
    expect(parseGitHubRemote("https://gitlab.com/sample/widgets.git")).toBeNull();
    expect(parseGitHubRemote("https://github.com.evil.example/sample/widgets")).toBeNull();
    expect(parseGitHubRemote("")).toBeNull();
  });

  it("reads git log records", () => {
    const output = "aaa\x1faa\x1f100\x1fKim\x1ffix: one\x1e\nbbb\x1fbb\x1f90\x1fLee\x1ffeat: two | with \t tab\x1e\n";
    expect(parseCommitLog(output)).toEqual([
      { sha: "aaa", shortSha: "aa", committedAt: 100, author: "Kim", subject: "fix: one" },
      { sha: "bbb", shortSha: "bb", committedAt: 90, author: "Lee", subject: "feat: two | with \t tab" },
    ]);
    expect(parseCommitLog("")).toEqual([]);
  });

  it("reduces the checks to one state", () => {
    expect(checksRollup([])).toBeNull();
    expect(checksRollup(null)).toBeNull();
    expect(checksRollup([{ status: "COMPLETED", conclusion: "SUCCESS" }, { status: "COMPLETED", conclusion: "SKIPPED" }])).toBe("SUCCESS");
    expect(checksRollup([{ status: "COMPLETED", conclusion: "SUCCESS" }, { status: "IN_PROGRESS", conclusion: "" }])).toBe("PENDING");
    expect(checksRollup([{ status: "IN_PROGRESS", conclusion: "" }, { status: "COMPLETED", conclusion: "FAILURE" }])).toBe("FAILURE");
    expect(checksRollup([{ state: "PENDING" }])).toBe("PENDING");
    expect(checksRollup([{ state: "SUCCESS" }])).toBe("SUCCESS");
    expect(checksRollup([{ state: "ERROR" }])).toBe("FAILURE");
  });
});

describe("branch summary in a real repository", () => {
  const base = mkdtempSync(join(tmpdir(), "akra-branch-"));
  afterAll(() => rmSync(base, { recursive: true, force: true }));
  const remote = join(base, "remote.git");
  const root = join(base, "work");
  const gitIn = (cwd: string, ...args: string[]) =>
    execFileSync("git", ["-c", "user.name=Tester", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" });
  const git = (...args: string[]) => gitIn(root, ...args);
  const commit = (path: string, message: string) => {
    writeFileSync(join(root, path), `${message}\n`);
    git("add", ".");
    git("commit", "-q", "-m", message);
  };
  const noPullRequest: GhRunner = async () => "[]";

  it("lists the commits after the default branch and marks the pushed ones", async () => {
    gitIn(base, "init", "-q", "--bare", "-b", "main", "remote.git");
    gitIn(base, "clone", "-q", remote, "work");
    git("checkout", "-q", "-b", "main");
    commit("a.txt", "init");
    git("push", "-q", "-u", "origin", "main");
    git("remote", "set-head", "origin", "main");
    git("checkout", "-q", "-b", "feat/x");
    commit("b.txt", "feat: pushed");
    git("push", "-q", "-u", "origin", "feat/x");
    commit("c.txt", "feat: local only");

    const summary = await readBranchSummary(root, noPullRequest);
    expect(summary).toMatchObject({ ok: true, branch: "feat/x", upstream: "origin/feat/x", base: "origin/main", repository: null, pullRequest: null, truncated: false });
    if (!summary.ok) return;
    expect(summary.commits.map((item) => [item.subject, item.pushed, item.author])).toEqual([
      ["feat: local only", false, "Tester"],
      ["feat: pushed", true, "Tester"],
    ]);

    const diff = await readCommitDiff(root, summary.commits[0]!.sha);
    expect(diff.text).toContain("+++ b/c.txt");
    expect(diff.text).not.toContain("b.txt");
    await expect(readCommitDiff(root, "HEAD")).rejects.toThrow();
    await expect(readCommitDiff(root, "--output=x")).rejects.toThrow();
  });

  it("asks gh for the PR of the remote branch and compares against the PR base", async () => {
    git("remote", "set-url", "origin", "https://github.com/sample/widgets.git");
    git("update-ref", "refs/remotes/origin/release", "HEAD~1");
    const calls: string[][] = [];
    const run: GhRunner = async (args) => {
      calls.push(args);
      return JSON.stringify([
        { number: 7, title: "Add x", url: "https://github.com/sample/widgets/pull/7", state: "OPEN", isDraft: true, baseRefName: "release", statusCheckRollup: [{ status: "COMPLETED", conclusion: "FAILURE" }] },
      ]);
    };

    const summary = await readBranchSummary(root, run);
    expect(calls).toEqual([["pr", "list", "--repo", "sample/widgets", "--head=feat/x", "--state", "all", "--limit", "1", "--json", "number,title,url,state,isDraft,baseRefName,statusCheckRollup"]]);
    expect(summary).toMatchObject({
      ok: true,
      repository: "sample/widgets",
      base: "origin/release",
      pullRequest: { number: 7, title: "Add x", state: "OPEN", isDraft: true, baseBranch: "release", checks: "FAILURE" },
      pullRequestError: null,
    });
    if (!summary.ok) return;
    // 기준이 PR 의 대상 브랜치이므로 그 뒤의 커밋 하나만 나온다.
    expect(summary.commits.map((item) => item.subject)).toEqual(["feat: local only"]);
  });

  it("keeps the commits when gh fails and falls back to the default branch", async () => {
    const failing: GhRunner = async () => {
      throw new Error("gh 를 실행할 수 없습니다");
    };
    const summary = await readBranchSummary(root, failing);
    expect(summary).toMatchObject({ ok: true, base: "origin/main", pullRequest: null, pullRequestError: "gh 를 실행할 수 없습니다" });
    if (!summary.ok) return;
    expect(summary.commits).toHaveLength(2);
  });

  it("marks every commit unpushed on a branch without a remote branch and skips gh without a branch", async () => {
    git("checkout", "-q", "-b", "local-only");
    const summary = await readBranchSummary(root, noPullRequest);
    expect(summary).toMatchObject({ ok: true, branch: "local-only", upstream: null });
    if (!summary.ok) return;
    expect(summary.commits.map((item) => item.pushed)).toEqual([false, false]);

    git("checkout", "-q", "--detach");
    let asked = false;
    const detached = await readBranchSummary(root, async () => {
      asked = true;
      return "[]";
    });
    expect(detached).toMatchObject({ ok: true, branch: null, pullRequest: null });
    expect(asked).toBe(false);
  });

  it("reports a folder that is not a git repository", async () => {
    const plain = mkdtempSync(join(tmpdir(), "akra-plain-"));
    try {
      expect(await readBranchSummary(plain, noPullRequest)).toEqual({ ok: false, message: "git 저장소가 아닙니다." });
    } finally {
      rmSync(plain, { recursive: true, force: true });
    }
  });
});
