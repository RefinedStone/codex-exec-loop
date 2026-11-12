// 스레드 cwd 의 브랜치가 기준 브랜치 뒤에 더한 커밋과 그 브랜치의 PR 을 읽는다 (R10). 모두 읽기 명령이다.
// 에이전트가 커밋까지 하면 작업 트리 목록은 비어 보인다. 그때 무엇을 커밋했고 push 와 PR 이 어디까지 됐는지 보여 준다.
import type { GitBranchSummary, GitChecksState, GitCommit, GitCommitDiff, GitPullRequest } from "../shared/bridge";
import { failureMessage, requireDirectory, runGit } from "./gitStatus";
import { type GhRunner, runGh } from "./githubService";

/** 한 번에 보여 주는 커밋 수이다. 더 많으면 최근 것만 보여 준다. */
const COMMIT_LIMIT = 100;
const LOG_OUTPUT_LIMIT = 1 << 20;
/** 커밋 하나의 diff 한도이다. 넘으면 앞부분만 보여 준다. */
const COMMIT_DIFF_LIMIT = 2 << 20;
const FIELD = "\x1f";
const RECORD = "\x1e";
const LOG_FORMAT = `--format=%H${FIELD}%h${FIELD}%ct${FIELD}%an${FIELD}%s${RECORD}`;

/** `origin` 주소에서 GitHub 의 `owner/name` 을 읽는다. GitHub 가 아니면 null 이다. */
export function parseGitHubRemote(url: string): string | null {
  const match = /^(?:https:\/\/(?:[^@/\s]+@)?github\.com\/|git@github\.com:|ssh:\/\/git@github\.com\/)([\w.-]+)\/([\w.-]+?)(?:\.git)?\/?$/.exec(url.trim());
  return match ? `${match[1]}/${match[2]}` : null;
}

/** `git log` 출력(위 형식)을 커밋 목록으로 바꾼다. push 여부는 부르는 쪽이 채운다. */
export function parseCommitLog(output: string): Omit<GitCommit, "pushed">[] {
  return output
    .split(RECORD)
    .map((record) => record.replace(/^\n/, "").split(FIELD))
    .filter((fields) => fields.length === 5 && fields[0] !== "")
    .map(([sha, shortSha, time, author, subject]) => ({ sha: sha!, shortSha: shortSha!, committedAt: Number(time), author: author!, subject: subject! }));
}

const FAILED_CHECKS = new Set(["FAILURE", "ERROR", "TIMED_OUT", "CANCELLED", "ACTION_REQUIRED", "STARTUP_FAILURE"]);

/** `gh` 의 `statusCheckRollup` 목록을 상태 하나로 줄인다. 실패가 하나라도 있으면 실패, 끝나지 않은 것이 있으면 진행 중이다. */
export function checksRollup(items: unknown): GitChecksState | null {
  if (!Array.isArray(items) || items.length === 0) return null;
  let pending = false;
  for (const item of items as Array<{ status?: unknown; conclusion?: unknown; state?: unknown }>) {
    // 검사 실행은 status 와 conclusion 을, 커밋 상태는 state 를 준다.
    const result = String(item.conclusion || item.state || "").toUpperCase();
    if (FAILED_CHECKS.has(result)) return "FAILURE";
    const finished = item.state !== undefined ? result !== "PENDING" && result !== "EXPECTED" : String(item.status).toUpperCase() === "COMPLETED";
    if (!finished) pending = true;
  }
  return pending ? "PENDING" : "SUCCESS";
}

function parsePullRequest(output: string): GitPullRequest | null {
  const raw = (JSON.parse(output) as Array<Record<string, unknown>>)[0];
  if (!raw || typeof raw.number !== "number" || typeof raw.url !== "string") return null;
  const state = raw.state === "MERGED" || raw.state === "CLOSED" ? raw.state : "OPEN";
  return {
    number: raw.number,
    title: String(raw.title ?? ""),
    url: raw.url,
    state,
    isDraft: raw.isDraft === true,
    baseBranch: String(raw.baseRefName ?? ""),
    checks: checksRollup(raw.statusCheckRollup),
  };
}

/**
 * 브랜치의 커밋과 PR 을 읽는다. 기준 브랜치는 PR 의 대상 브랜치이다. PR 이 없으면 `origin/HEAD` 가 가리키는 브랜치이다.
 * gh 는 저장소 밖에서 실행한다 (`runGh`). 그래서 저장소는 `origin` 주소에서 읽어 `--repo` 로 준다.
 */
export async function readBranchSummary(cwdInput: unknown, run: GhRunner = runGh): Promise<GitBranchSummary> {
  const cwd = await requireDirectory(cwdInput);
  /** 성공하면 출력의 첫 줄을, 실패하면 null 을 준다. */
  const git = async (...args: string[]): Promise<string | null> => {
    const output = await runGit(cwd, args, 4096);
    return output.code === 0 ? output.stdout.trim() : null;
  };
  if ((await git("rev-parse", "--show-toplevel")) === null) {
    return { ok: false, message: "git 저장소가 아닙니다." };
  }
  const branch = await git("symbolic-ref", "-q", "--short", "HEAD");
  const upstream = await git("rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}");
  const repository = parseGitHubRemote((await git("remote", "get-url", "origin")) ?? "");

  let pullRequest: GitPullRequest | null = null;
  let pullRequestError: string | null = null;
  if (repository && branch) {
    // 원격 브랜치 이름이 로컬과 다를 수 있다. upstream 이 origin 에 있으면 그 이름으로 찾는다.
    const head = upstream?.startsWith("origin/") ? upstream.slice("origin/".length) : branch;
    const fields = "number,title,url,state,isDraft,baseRefName,statusCheckRollup";
    try {
      pullRequest = parsePullRequest(await run(["pr", "list", "--repo", repository, `--head=${head}`, "--state", "all", "--limit", "1", "--json", fields]));
    } catch (error) {
      pullRequestError = error instanceof Error ? error.message : String(error);
    }
  }

  const candidate = pullRequest?.baseBranch ? `origin/${pullRequest.baseBranch}` : await git("symbolic-ref", "-q", "--short", "refs/remotes/origin/HEAD");
  const base = candidate && (await git("rev-parse", "--verify", "-q", `${candidate}^{commit}`)) !== null ? candidate : null;
  if (!base) {
    return { ok: true, branch, upstream, repository, base: null, commits: [], truncated: false, pullRequest, pullRequestError };
  }

  const log = await runGit(cwd, ["log", LOG_FORMAT, "-n", String(COMMIT_LIMIT + 1), `${base}..HEAD`], LOG_OUTPUT_LIMIT);
  if (log.code !== 0) {
    return { ok: false, message: failureMessage(log, "git log 가 실패했습니다.") };
  }
  const parsed = parseCommitLog(log.stdout);
  // upstream 이 없으면 한 번도 push 하지 않은 브랜치이다. 모든 커밋이 push 전이다.
  const unpushed = upstream ? await runGit(cwd, ["rev-list", "-n", String(COMMIT_LIMIT + 1), `${upstream}..HEAD`], LOG_OUTPUT_LIMIT) : null;
  const unpushedShas = unpushed && unpushed.code === 0 ? new Set(unpushed.stdout.split("\n").filter(Boolean)) : null;
  const commits = parsed.slice(0, COMMIT_LIMIT).map((commit) => ({ ...commit, pushed: unpushedShas ? !unpushedShas.has(commit.sha) : false }));
  return { ok: true, branch, upstream, repository, base, commits, truncated: parsed.length > COMMIT_LIMIT || log.truncated, pullRequest, pullRequestError };
}

/** 커밋 하나의 변경이다 (git diff 형식). 병합 커밋은 첫 부모와 비교한다. */
export async function readCommitDiff(cwdInput: unknown, sha: unknown): Promise<GitCommitDiff> {
  const cwd = await requireDirectory(cwdInput);
  if (typeof sha !== "string" || !/^[0-9a-f]{7,40}$/.test(sha)) {
    throw new Error("커밋 id 가 올바르지 않습니다.");
  }
  const output = await runGit(cwd, ["show", "--format=", "--first-parent", "--no-color", "--no-ext-diff", "--no-textconv", sha, "--"], COMMIT_DIFF_LIMIT);
  if (output.code !== 0) {
    throw new Error(failureMessage(output, "git show 가 실패했습니다."));
  }
  return { text: output.stdout, truncated: output.truncated };
}
