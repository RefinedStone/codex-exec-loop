// push 와 PR 만들기이다 (R10). 실제 git 저장소(로컬 bare 원격)와 가짜 gh 로 시험한다. 네트워크를 쓰지 않는다.
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterAll, describe, expect, it } from "vitest";

import { readBranchSummary } from "../src/main/gitBranch";
import { executePullRequest, executePush, planPullRequest, planPush } from "../src/main/gitPublish";
import type { GhRunner } from "../src/main/githubService";
import { canCreatePullRequest, canPush, initialPullRequestDraft, type LoadedSummary } from "../src/renderer/conversation/branchPublish";

describe("push and create a PR", () => {
  const base = mkdtempSync(join(tmpdir(), "akra-publish-"));
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
  const draft = { title: "Add x", body: "- one\n- two", base: "main", draft: false };
  const login: GhRunner = async () => "sample-user\n";

  it("pushes the current branch to the branch of the same name and sets the upstream", async () => {
    gitIn(base, "init", "-q", "--bare", "-b", "main", "remote.git");
    gitIn(base, "clone", "-q", remote, "work");
    git("checkout", "-q", "-b", "main");
    commit("a.txt", "init");
    git("push", "-q", "-u", "origin", "main");
    // 이 저장소의 흐름이다: 새 브랜치의 upstream 은 기준 브랜치이다. push 는 같은 이름의 브랜치로 가야 한다.
    git("checkout", "-q", "-b", "feat/x", "--track", "origin/main");
    commit("b.txt", "feat: one");

    const planned = await planPush(root);
    expect(planned).toMatchObject({ ok: true, plan: { branch: "feat/x" } });
    if (!planned.ok) return;
    expect(planned.plan.remoteUrl).toContain("remote.git");
    expect(await executePush(planned.plan)).toEqual({ ok: true, url: null });

    expect(gitIn(remote, "rev-parse", "refs/heads/feat/x").trim()).toBe(git("rev-parse", "HEAD").trim());
    expect(gitIn(remote, "rev-parse", "refs/heads/main").trim()).toBe(git("rev-parse", "HEAD~1").trim());
    expect(git("rev-parse", "--abbrev-ref", "@{upstream}").trim()).toBe("origin/feat/x");
    const summary = await readBranchSummary(root, true, async () => "[]");
    expect(summary).toMatchObject({ ok: true, remoteBranch: "origin/feat/x" });
    if (summary.ok) expect(summary.commits.map((item) => item.pushed)).toEqual([true]);
  });

  it("does not publish annotated tags when push.followTags is enabled", async () => {
    git("tag", "-a", "local-only", "-m", "Local tag");
    git("config", "push.followTags", "true");
    try {
      const planned = await planPush(root);
      if (!planned.ok) throw new Error(planned.message);
      expect((await executePush(planned.plan)).ok).toBe(true);
      expect(gitIn(remote, "for-each-ref", "--format=%(refname)", "refs/tags")).toBe("");
    } finally {
      git("config", "--unset", "push.followTags");
    }
  });

  it("does not overwrite a remote branch that is ahead", async () => {
    const other = join(base, "other");
    gitIn(base, "clone", "-q", "-b", "feat/x", remote, "other");
    writeFileSync(join(other, "theirs.txt"), "theirs\n");
    gitIn(other, "add", ".");
    gitIn(other, "commit", "-q", "-m", "theirs");
    gitIn(other, "push", "-q");
    const theirs = gitIn(remote, "rev-parse", "refs/heads/feat/x").trim();
    commit("c.txt", "feat: two");

    const planned = await planPush(root);
    if (!planned.ok) throw new Error(planned.message);
    const result = await executePush(planned.plan);
    expect(result.ok).toBe(false);
    expect(gitIn(remote, "rev-parse", "refs/heads/feat/x").trim()).toBe(theirs);
  });

  it("shows the push URL rather than the fetch URL", async () => {
    git("config", "remote.origin.pushurl", "https://github.com/sample/publish.git");
    try {
      const planned = await planPush(root);
      expect(planned).toMatchObject({ ok: true, plan: { remoteUrl: "https://github.com/sample/publish.git" } });
      git("config", "--add", "remote.origin.pushurl", "https://github.com/sample/second.git");
      expect((await planPush(root)).ok).toBe(false);
    } finally {
      git("config", "--unset-all", "remote.origin.pushurl");
    }
  });

  it("refuses a push when its destination changes during confirmation", async () => {
    const unexpected = join(base, "unexpected.git");
    gitIn(base, "init", "-q", "--bare", unexpected);
    const planned = await planPush(root);
    if (!planned.ok) throw new Error(planned.message);
    git("config", "remote.origin.pushurl", unexpected);
    try {
      expect((await executePush(planned.plan)).ok).toBe(false);
      expect(gitIn(unexpected, "for-each-ref", "--format=%(refname)", "refs/heads")).toBe("");
    } finally {
      git("config", "--unset-all", "remote.origin.pushurl");
    }
  });

  it("refuses to push without a branch or without origin", async () => {
    git("checkout", "-q", "--detach");
    expect((await planPush(root)).ok).toBe(false);
    git("checkout", "-q", "feat/x");
    const lonely = mkdtempSync(join(tmpdir(), "akra-lonely-"));
    try {
      gitIn(lonely, "init", "-q", "-b", "main");
      expect(await planPush(lonely)).toEqual({ ok: false, message: "원격 저장소 origin 이 없습니다." });
    } finally {
      rmSync(lonely, { recursive: true, force: true });
    }
  });

  it("refuses a PR for a wrong draft, a repository that is not on GitHub, and commits that are not pushed", async () => {
    // 지금 원격은 로컬 폴더이다.
    expect(await planPullRequest(root, draft, login)).toEqual({ ok: false, message: "origin 이 GitHub 저장소가 아닙니다." });
    git("remote", "set-url", "origin", "https://github.com/sample/widgets.git");

    for (const wrong of [{ ...draft, title: " " }, { ...draft, title: "two\nlines" }, { ...draft, base: "--delete" }, { ...draft, base: "a b" }, { ...draft, body: 1 }, null]) {
      expect((await planPullRequest(root, wrong, login)).ok).toBe(false);
    }
    expect(await planPullRequest(root, { ...draft, base: "feat/x" }, login)).toEqual({ ok: false, message: "대상 브랜치가 현재 브랜치와 같습니다." });
    // "feat: two" 는 원격이 거절해서 아직 push 하지 않았다.
    expect(await planPullRequest(root, draft, login)).toEqual({ ok: false, message: "push 하지 않은 커밋이 1개 있습니다. 먼저 push 하십시오." });

    git("checkout", "-q", "-b", "never-pushed");
    expect(await planPullRequest(root, draft, login)).toEqual({ ok: false, message: "이 브랜치는 아직 push 하지 않았습니다. 먼저 push 하십시오." });
    git("checkout", "-q", "feat/x");
  });

  it("creates the PR with gh outside the repository and returns its address", async () => {
    // 원격 브랜치와 같게 만든다.
    git("update-ref", "refs/remotes/origin/feat/x", "HEAD");
    const failing: GhRunner = async () => {
      throw new Error("gh auth login 이 필요합니다");
    };
    expect(await planPullRequest(root, draft, failing)).toEqual({ ok: false, message: "gh auth login 이 필요합니다" });

    const planned = await planPullRequest(root, { ...draft, title: "  Add x  ", draft: true }, login);
    expect(planned).toEqual({ ok: true, plan: { repository: "sample/widgets", branch: "feat/x", base: "main", title: "Add x", body: "- one\n- two", draft: true, login: "sample-user" } });
    if (!planned.ok) return;

    const calls: string[][] = [];
    const run: GhRunner = async (args) => {
      calls.push(args);
      return "Creating pull request for feat/x into main in sample/widgets\n\nhttps://github.com/sample/widgets/pull/12\n";
    };
    expect(await executePullRequest(planned.plan, run)).toEqual({ ok: true, url: "https://github.com/sample/widgets/pull/12" });
    expect(calls).toEqual([["pr", "create", "--repo", "sample/widgets", "--head=feat/x", "--base=main", "--title=Add x", "--body=- one\n- two", "--draft"]]);

    const rejected: GhRunner = async () => {
      throw new Error("a pull request for branch feat/x already exists");
    };
    expect(await executePullRequest(planned.plan, rejected)).toEqual({ ok: false, message: "a pull request for branch feat/x already exists" });
  });
});

describe("when the panel offers push and a new PR", () => {
  const commit = (subject: string, pushed: boolean | null) => ({ sha: subject, shortSha: subject, subject, author: "a", committedAt: 0, pushed });
  const summary = (fields: Partial<LoadedSummary>): LoadedSummary => ({
    ok: true,
    branch: "feat/x",
    remoteBranch: "origin/feat/x",
    repository: "sample/widgets",
    base: "origin/main",
    commits: [commit("second", true), commit("first", true)],
    truncated: false,
    pullRequest: null,
    pullRequestError: null,
    ...fields,
  });
  const pull = (state: "OPEN" | "MERGED" | "CLOSED") => ({ number: 1, title: "t", url: "u", state, isDraft: false, baseBranch: "main", checks: null });

  it("offers push only for commits that are known to be unpushed", () => {
    expect(canPush(summary({}))).toBe(false);
    expect(canPush(summary({ commits: [commit("new", false), commit("first", true)] }))).toBe(true);
    expect(canPush(summary({ commits: [commit("unknown", null)] }))).toBe(false);
    expect(canPush(summary({ branch: null, commits: [commit("new", false)] }))).toBe(false);
  });

  it("offers a new PR after everything is pushed and no PR is open", () => {
    expect(canCreatePullRequest(summary({}))).toBe(true);
    expect(canCreatePullRequest(summary({ pullRequest: pull("OPEN") }))).toBe(false);
    expect(canCreatePullRequest(summary({ pullRequest: pull("MERGED") }))).toBe(true);
    expect(canCreatePullRequest(summary({ pullRequest: pull("CLOSED") }))).toBe(true);
    expect(canCreatePullRequest(summary({ commits: [commit("new", false)] }))).toBe(false);
    expect(canCreatePullRequest(summary({ commits: [] }))).toBe(false);
    expect(canCreatePullRequest(summary({ remoteBranch: null }))).toBe(false);
    expect(canCreatePullRequest(summary({ repository: null }))).toBe(false);
    expect(canCreatePullRequest(summary({ base: null }))).toBe(false);
  });

  it("fills the form from the commits, oldest first", () => {
    expect(initialPullRequestDraft(summary({}))).toEqual({ title: "first", body: "- first\n- second", base: "main" });
    expect(initialPullRequestDraft(summary({ commits: [commit("only", true)], base: "origin/release/1.x" }))).toEqual({ title: "only", body: "", base: "release/1.x" });
  });
});
