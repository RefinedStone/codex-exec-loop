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
/** `-z` 와 함께 쓴다. 필드와 커밋이 모두 NUL 로 끝난다. 제목과 이름에는 NUL 이 들어갈 수 없다. */
const LOG_FIELDS = 5;
const LOG_FORMAT = "--format=%H%x00%h%x00%ct%x00%an%x00%s";
/** 같은 브랜치의 PR 을 다시 묻지 않는 시간이다. 파일을 스테이지할 때마다 gh 를 부르지 않게 한다. */
const PULL_REQUEST_CACHE_MS = 60_000;
const PULL_REQUEST_FIELDS = "number,title,url,state,isDraft,baseRefName,statusCheckRollup,isCrossRepository";

/** `origin` 주소에서 GitHub 의 `owner/name` 을 읽는다. GitHub 가 아니면 null 이다. */
export function parseGitHubRemote(url: string): string | null {
  const match = /^(?:https:\/\/(?:[^@/\s]+@)?github\.com\/|git@github\.com:|ssh:\/\/git@github\.com\/)([\w.-]+)\/([\w.-]+?)(?:\.git)?\/?$/.exec(url.trim());
  return match ? `${match[1]}/${match[2]}` : null;
}

/** 폴더의 `origin` 이 가리키는 GitHub `owner/name` 이다 (R11). git 저장소가 아니거나 GitHub 가 아니면 null 이다. */
export async function readGitHubRepository(cwdInput: unknown): Promise<string | null> {
  const output = await runGit(await requireDirectory(cwdInput), ["remote", "get-url", "origin"], 4096);
  return output.code === 0 ? parseGitHubRemote(output.stdout) : null;
}

/** `git log -z` 출력(위 형식)을 커밋 목록으로 바꾼다. push 여부는 부르는 쪽이 채운다. */
export function parseCommitLog(output: string): Omit<GitCommit, "pushed">[] {
  const fields = output.split("\0");
  const commits: Omit<GitCommit, "pushed">[] = [];
  for (let index = 0; index + LOG_FIELDS <= fields.length; index += LOG_FIELDS) {
    const [sha, shortSha, time, author, subject] = fields.slice(index, index + LOG_FIELDS) as [string, string, string, string, string];
    commits.push({ sha, shortSha, committedAt: Number(time), author, subject });
  }
  return commits;
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

/** gh 의 PR 목록에서 이 저장소의 브랜치로 올린 가장 최근 PR 을 고른다. fork 에서 올린 같은 이름의 브랜치는 뺀다. */
export function parsePullRequest(output: string): GitPullRequest | null {
  const raw = (JSON.parse(output) as Array<Record<string, unknown>>).find((item) => item.isCrossRepository !== true);
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

const pullRequestCache = new Map<string, { at: number; pullRequest: GitPullRequest | null }>();

/** 저장한 PR 을 지운다. PR 을 만들거나 바꾼 뒤에 부른다. */
export function clearPullRequestCache(): void {
  pullRequestCache.clear();
}

/**
 * 브랜치의 PR 을 gh 로 읽는다. gh 는 저장소 밖에서 실행한다 (`runGh`). 그래서 저장소를 `--repo` 로 준다.
 * 성공한 결과만 잠깐 저장한다. `fresh` 이면 저장한 값을 쓰지 않는다.
 */
async function readPullRequest(repository: string, branch: string, fresh: boolean, run: GhRunner): Promise<GitPullRequest | null> {
  const key = `${repository}\n${branch}`;
  const cached = pullRequestCache.get(key);
  if (!fresh && cached && Date.now() - cached.at < PULL_REQUEST_CACHE_MS) {
    return cached.pullRequest;
  }
  const pullRequest = parsePullRequest(await run(["pr", "list", "--repo", repository, `--head=${branch}`, "--state", "all", "--limit", "5", "--json", PULL_REQUEST_FIELDS]));
  pullRequestCache.set(key, { at: Date.now(), pullRequest });
  return pullRequest;
}

/**
 * 브랜치의 커밋과 PR 을 읽는다. 기준 브랜치는 처음 찾은 것이다:
 * PR 의 대상 브랜치, 이 브랜치를 만든 원격 브랜치(upstream 이 다른 브랜치일 때), `origin/HEAD`, `origin/main`, `origin/master`.
 * shortcut: push 는 했고 PR 은 없는 브랜치는 `origin/HEAD` 와 비교한다. 다른 브랜치에서 갈라졌으면 커밋이 많이 나온다.
 * 화면이 기준을 보여 준다. 필요하면 폴더별 기준 브랜치 설정을 더한다.
 */
export async function readBranchSummary(cwdInput: unknown, fresh: unknown = false, run: GhRunner = runGh): Promise<GitBranchSummary> {
  const cwd = await requireDirectory(cwdInput);
  /** 성공하면 출력을, 실패하면 null 을 준다. */
  const git = async (...args: string[]): Promise<string | null> => {
    const output = await runGit(cwd, args, 4096);
    return output.code === 0 ? output.stdout.trim() : null;
  };
  /** 커밋을 가리키는 ref 이면 그 이름을, 아니면 null 을 준다. */
  const existing = async (ref: string | null): Promise<string | null> =>
    ref && (await git("rev-parse", "--verify", "-q", "--end-of-options", `${ref}^{commit}`)) !== null ? ref : null;

  if ((await git("rev-parse", "--show-toplevel")) === null) {
    return { ok: false, message: "git 저장소가 아닙니다." };
  }
  const branch = await git("symbolic-ref", "-q", "--short", "HEAD");
  // 이 브랜치를 push 한 원격 브랜치이다. upstream 은 쓰지 않는다. `git worktree add -b x <path> origin/main` 으로 만든
  // 브랜치의 upstream 은 `origin/main` 이다.
  const remoteBranch = branch ? await existing(`refs/remotes/origin/${branch}`) : null;
  const upstream = await git("rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}");
  const repository = parseGitHubRemote((await git("remote", "get-url", "origin")) ?? "");

  // gh 는 느릴 수 있다. PR 과 관계없는 git 읽기와 함께 실행한다.
  const pullRequestRead =
    repository && branch
      ? readPullRequest(repository, branch, fresh === true, run).then(
          (pullRequest) => ({ pullRequest, error: null }),
          (error: unknown) => ({ pullRequest: null, error: error instanceof Error ? error.message : String(error) }),
        )
      : Promise.resolve({ pullRequest: null, error: null });
  const createdFrom = upstream?.startsWith("origin/") && upstream !== `origin/${branch}` ? upstream : null;
  const fallbacks = [await git("symbolic-ref", "-q", "--short", "refs/remotes/origin/HEAD"), "origin/main", "origin/master"];
  const { pullRequest, error: pullRequestError } = await pullRequestRead;

  let base: string | null = null;
  for (const candidate of [pullRequest?.baseBranch ? `origin/${pullRequest.baseBranch}` : null, createdFrom, ...fallbacks]) {
    base = await existing(candidate);
    if (base) break;
  }
  const shared = { ok: true as const, branch, remoteBranch: remoteBranch ? `origin/${branch}` : null, repository, pullRequest, pullRequestError };
  if (!base) {
    return { ...shared, base: null, commits: [], truncated: false };
  }

  const log = await runGit(cwd, ["log", "-z", LOG_FORMAT, "-n", String(COMMIT_LIMIT + 1), "--end-of-options", `${base}..HEAD`], LOG_OUTPUT_LIMIT);
  if (log.code !== 0) {
    return { ok: false, message: failureMessage(log, "git log 가 실패했습니다.") };
  }
  const parsed = parseCommitLog(log.stdout);
  let unpushed: Set<string> | null = null;
  if (remoteBranch) {
    const output = await runGit(cwd, ["rev-list", "-n", String(COMMIT_LIMIT + 1), "--end-of-options", `${remoteBranch}..HEAD`], LOG_OUTPUT_LIMIT);
    unpushed = output.code === 0 ? new Set(output.stdout.split("\n").filter(Boolean)) : null;
  }
  // 원격 브랜치가 없으면 push 전이다. 다만 머지한 뒤 지운 브랜치와 브랜치 없이 보는 커밋은 알 수 없다.
  const unknown = !remoteBranch && (branch === null || pullRequest?.state === "MERGED");
  const commits = parsed.slice(0, COMMIT_LIMIT).map((commit) => ({ ...commit, pushed: unknown ? null : unpushed ? !unpushed.has(commit.sha) : false }));
  return { ...shared, base, commits, truncated: parsed.length > COMMIT_LIMIT || log.truncated };
}

/** 커밋 하나의 변경이다 (git diff 형식). 병합 커밋은 첫 부모와 비교한다. */
export async function readCommitDiff(cwdInput: unknown, sha: unknown): Promise<GitCommitDiff> {
  const cwd = await requireDirectory(cwdInput);
  if (typeof sha !== "string" || !/^[0-9a-f]{7,40}$/.test(sha)) {
    throw new Error("커밋 id 가 올바르지 않습니다.");
  }
  const output = await runGit(cwd, ["show", "--format=", "--first-parent", "--no-color", "--no-ext-diff", "--no-textconv", "--end-of-options", `${sha}^{commit}`, "--"], COMMIT_DIFF_LIMIT);
  if (output.code !== 0) {
    throw new Error(failureMessage(output, "git show 가 실패했습니다."));
  }
  return { text: output.stdout, truncated: output.truncated };
}
