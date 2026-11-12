// 브랜치를 push 하고 PR 을 만든다 (R10). 둘 다 저장소 밖에 쓰는 동작이다. 여기서는 확인(`plan…`)과 실행(`execute…`)을
// 나눈다. 부르는 쪽(main 의 IPC 처리기)이 그 사이에 사용자에게 확인한다. 강제 push 는 하지 않는다.
import type { GitPublishResult, PullRequestDraft } from "../shared/bridge";
import { clearPullRequestCache, parseGitHubRemote } from "./gitBranch";
import { failureMessage, requireDirectory, runGit } from "./gitStatus";
import { type GhRunner, runGh } from "./githubService";

const PUSH_TIMEOUT_MS = 120_000;
/** git 이 암호를 물으려고 터미널이나 창을 띄우지 않게 한다. 자격 증명이 없으면 바로 실패한다. */
const NO_PROMPT_ENV = { GIT_TERMINAL_PROMPT: "0", GCM_INTERACTIVE: "never" };
const TITLE_LIMIT = 256;
const BODY_LIMIT = 60_000;
/** git 브랜치 이름으로 쓸 수 있는 글자만 받는다. `-` 로 시작하면 옵션으로 읽힐 수 있다. */
const BRANCH_NAME = /^(?!-)[\w./-]+$/;

export interface PushPlan {
  cwd: string;
  branch: string;
  /** `origin` 의 push 주소이다. 확인 창에 보여 준다. */
  remoteUrl: string;
  commit: string;
}

export interface PullRequestPlan {
  repository: string;
  branch: string;
  base: string;
  title: string;
  body: string;
  draft: boolean;
  /** PR 을 만들 gh 계정이다. 확인 창에 보여 준다. */
  login: string;
}

type Planned<T> = { ok: true; plan: T } | { ok: false; message: string };

function refuse(message: string): { ok: false; message: string } {
  return { ok: false, message };
}

async function gitLine(cwd: string, ...args: string[]): Promise<string | null> {
  const output = await runGit(cwd, args, 4096);
  return output.code === 0 ? output.stdout.trim() : null;
}

export async function planPush(cwdInput: unknown): Promise<Planned<PushPlan>> {
  const cwd = await requireDirectory(cwdInput);
  const branch = await gitLine(cwd, "symbolic-ref", "-q", "--short", "HEAD");
  if (!branch) return refuse("브랜치가 없습니다. 브랜치를 만든 뒤에 push 하십시오.");
  const remoteUrl = await gitLine(cwd, "remote", "get-url", "--push", "--all", "origin");
  if (!remoteUrl) return refuse("원격 저장소 origin 이 없습니다.");
  if (remoteUrl.includes("\n")) return refuse("push 주소가 여러 개입니다. 터미널에서 대상 주소를 확인하십시오.");
  const commit = await gitLine(cwd, "rev-parse", "--verify", "HEAD");
  if (!commit) return refuse("push 할 커밋이 없습니다.");
  return { ok: true, plan: { cwd, branch, remoteUrl, commit } };
}

/** 현재 브랜치를 `origin` 의 같은 이름 브랜치로 push 한다. 원격이 앞서 있으면 git 이 거절한다. 강제로 덮어쓰지 않는다. */
export async function executePush(plan: PushPlan): Promise<GitPublishResult> {
  const current = await planPush(plan.cwd);
  if (!current.ok) return current;
  if (current.plan.branch !== plan.branch || current.plan.remoteUrl !== plan.remoteUrl || current.plan.commit !== plan.commit) {
    return refuse("확인하는 동안 브랜치나 원격이 바뀌었습니다. 다시 확인하십시오.");
  }
  const ref = `refs/heads/${plan.branch}`;
  const output = await runGit(plan.cwd, ["push", "--no-follow-tags", "--set-upstream", "origin", `${ref}:${ref}`], 1 << 16, { timeoutMs: PUSH_TIMEOUT_MS, env: NO_PROMPT_ENV });
  if (output.code !== 0) {
    return refuse(failureMessage(output, "git push 가 실패했습니다."));
  }
  return { ok: true, url: null };
}

function validDraft(input: unknown): PullRequestDraft | null {
  const fields = (input ?? {}) as Partial<PullRequestDraft>;
  const title = typeof fields.title === "string" ? fields.title.trim() : "";
  if (title.length === 0 || title.length > TITLE_LIMIT || /[\r\n]/.test(title)) return null;
  if (typeof fields.body !== "string" || fields.body.length > BODY_LIMIT) return null;
  if (typeof fields.base !== "string" || !BRANCH_NAME.test(fields.base)) return null;
  return { title, body: fields.body, base: fields.base, draft: fields.draft === true };
}

/** PR 을 만들 수 있는지 확인한다. 브랜치가 `origin` 에 있고 push 하지 않은 커밋이 없어야 한다. */
export async function planPullRequest(cwdInput: unknown, input: unknown, run: GhRunner = runGh): Promise<Planned<PullRequestPlan>> {
  const cwd = await requireDirectory(cwdInput);
  const draft = validDraft(input);
  if (!draft) return refuse("제목(한 줄, 256자까지)과 대상 브랜치를 확인하십시오.");
  const branch = await gitLine(cwd, "symbolic-ref", "-q", "--short", "HEAD");
  if (!branch) return refuse("브랜치가 없습니다.");
  if (branch === draft.base) return refuse("대상 브랜치가 현재 브랜치와 같습니다.");
  const repository = parseGitHubRemote((await gitLine(cwd, "remote", "get-url", "origin")) ?? "");
  if (!repository) return refuse("origin 이 GitHub 저장소가 아닙니다.");
  const remoteBranch = `refs/remotes/origin/${branch}`;
  const unpushed = await gitLine(cwd, "rev-list", "--count", "--end-of-options", `${remoteBranch}..HEAD`);
  if (unpushed === null) return refuse("이 브랜치는 아직 push 하지 않았습니다. 먼저 push 하십시오.");
  if (unpushed !== "0") return refuse(`push 하지 않은 커밋이 ${unpushed}개 있습니다. 먼저 push 하십시오.`);
  let login: string;
  try {
    login = (await run(["api", "user", "--jq", ".login"])).trim();
  } catch (error) {
    return refuse(error instanceof Error ? error.message : String(error));
  }
  if (!login) return refuse("gh 계정을 읽지 못했습니다. `gh auth login` 을 확인하십시오.");
  return { ok: true, plan: { repository, branch, ...draft, login } };
}

/** gh 로 PR 을 만든다. gh 는 저장소 밖에서 실행한다. 그래서 저장소와 브랜치를 인자로 준다. */
export async function executePullRequest(plan: PullRequestPlan, run: GhRunner = runGh): Promise<GitPublishResult> {
  // 본문은 표준 입력으로 준다. Windows 는 명령행 전체가 32,767자를 넘으면 프로세스를 만들지 못한다. 본문은 60,000자까지 받는다.
  const args = ["pr", "create", "--repo", plan.repository, `--head=${plan.branch}`, `--base=${plan.base}`, `--title=${plan.title}`, "--body-file", "-"];
  try {
    const output = await run(plan.draft ? [...args, "--draft"] : args, plan.body);
    // 방금 만든 PR 이 바로 보이게 한다.
    clearPullRequestCache();
    const url = output.split(/\s+/).find((word) => word.startsWith("https://github.com/")) ?? null;
    return { ok: true, url };
  } catch (error) {
    return refuse(error instanceof Error ? error.message : String(error));
  }
}
