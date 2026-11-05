// GitHub 업무 패널의 데이터이다 (R4). 공식 CLI `gh` 로 GraphQL 을 호출한다. 자격 증명은 gh 가 OS 키체인에 둔다.
// 앱은 토큰을 읽거나 저장하지 않는다 (D-14). 모두 읽기 요청이다.
import { spawn } from "node:child_process";
import { homedir } from "node:os";

import type { GitHubPullRequest, GitHubSummary } from "../shared/bridge";

const GH_TIMEOUT_MS = 20_000;
const OUTPUT_LIMIT = 8 << 20;
const MAX_COMMENT_CHARS = 2_000;
const MAX_PROMPT_CHARS = 20_000;

export type GhRunner = (args: string[]) => Promise<string>;

/** gh 를 셸 없이 실행한다. 저장소 폴더 밖(홈)에서 실행해서 git 을 부르지 않게 하고, 대화형 질문을 끈다. */
export const runGh: GhRunner = (args) =>
  new Promise((resolve, reject) => {
    const child = spawn("gh", args, {
      cwd: homedir(),
      windowsHide: true,
      env: { ...process.env, GH_PROMPT_DISABLED: "1", GH_NO_UPDATE_NOTIFIER: "1", NO_COLOR: "1" },
    });
    const chunks: Buffer[] = [];
    let size = 0;
    let stderr = "";
    const timer = setTimeout(() => child.kill(), GH_TIMEOUT_MS);
    child.stdout.on("data", (chunk: Buffer) => {
      size += chunk.length;
      if (size <= OUTPUT_LIMIT) chunks.push(chunk);
    });
    child.stderr.on("data", (chunk: Buffer) => {
      stderr = (stderr + chunk.toString("utf8")).slice(-2000);
    });
    child.on("error", (error) => {
      clearTimeout(timer);
      reject(new Error(`gh 를 실행할 수 없습니다 (설치 확인): ${error.message}`));
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      if (code === 0) resolve(Buffer.concat(chunks).toString("utf8"));
      else reject(new Error(stderr.trim() || `gh 가 실패했습니다 (종료 코드 ${code}). \`gh auth login\` 을 확인하세요.`));
    });
  });

const SUMMARY_QUERY = `query {
  viewer {
    login
    pullRequests(first: 30, states: OPEN, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes {
        number title url isDraft updatedAt reviewDecision
        repository { nameWithOwner }
        commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
        reviewThreads(first: 100) { nodes { isResolved } }
      }
    }
  }
  reviewRequests: search(query: "is:open is:pr review-requested:@me archived:false", type: ISSUE, first: 30) {
    nodes {
      ... on PullRequest { number title url isDraft updatedAt repository { nameWithOwner } author { login } }
    }
  }
}`;

const THREADS_QUERY = `query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      url title
      reviewThreads(first: 100) {
        nodes { isResolved path line comments(first: 20) { nodes { author { login } body } } }
      }
    }
  }
}`;

interface RawPullRequest {
  number?: number;
  title?: string;
  url?: string;
  isDraft?: boolean;
  updatedAt?: string;
  reviewDecision?: string | null;
  repository?: { nameWithOwner?: string };
  author?: { login?: string } | null;
  commits?: { nodes?: Array<{ commit?: { statusCheckRollup?: { state?: string } | null } }> };
  reviewThreads?: { nodes?: Array<{ isResolved?: boolean }> };
}

function pullRequest(raw: RawPullRequest): GitHubPullRequest | null {
  if (typeof raw?.number !== "number" || typeof raw.url !== "string" || !raw.url.startsWith("https://github.com/")) return null;
  const threads = raw.reviewThreads?.nodes;
  return {
    number: raw.number,
    title: raw.title ?? "",
    url: raw.url,
    repository: raw.repository?.nameWithOwner ?? "",
    isDraft: raw.isDraft === true,
    updatedAt: raw.updatedAt ?? "",
    author: raw.author?.login ?? null,
    checks: raw.commits?.nodes?.[0]?.commit?.statusCheckRollup?.state ?? null,
    reviewDecision: raw.reviewDecision ?? null,
    unresolvedThreads: threads ? threads.filter((thread) => thread.isResolved === false).length : null,
  };
}

/** GraphQL 응답을 패널용 값으로 바꾼다. 모르는 항목(다른 종류의 검색 결과 등)은 버린다. */
export function parseSummary(json: string): GitHubSummary {
  const data = (JSON.parse(json) as { data?: { viewer?: { login?: string; pullRequests?: { nodes?: RawPullRequest[] } }; reviewRequests?: { nodes?: RawPullRequest[] } } }).data;
  const list = (nodes: RawPullRequest[] | undefined) => (nodes ?? []).map(pullRequest).filter((item): item is GitHubPullRequest => item !== null);
  return {
    login: data?.viewer?.login ?? "",
    myPullRequests: list(data?.viewer?.pullRequests?.nodes),
    reviewRequests: list(data?.reviewRequests?.nodes),
  };
}

export async function readGitHubSummary(run: GhRunner = runGh): Promise<GitHubSummary> {
  return parseSummary(await run(["api", "graphql", "-f", `query=${SUMMARY_QUERY}`]));
}

/** `https://github.com/<owner>/<name>/pull/<number>` 만 받는다. */
export function parsePullRequestUrl(url: unknown): { owner: string; name: string; number: number } | null {
  const match = typeof url === "string" ? /^https:\/\/github\.com\/([A-Za-z0-9-]+)\/([A-Za-z0-9._-]+)\/pull\/(\d+)$/.exec(url) : null;
  return match ? { owner: match[1]!, name: match[2]!, number: Number(match[3]) } : null;
}

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max)}…(생략)` : text;
}

/** 해결되지 않은 리뷰 코멘트로 새 스레드의 첫 메시지를 만든다. 보내기는 사용자가 한다. */
export function reviewPrompt(json: string): string {
  const pull = (JSON.parse(json) as {
    data?: { repository?: { pullRequest?: { url?: string; title?: string; reviewThreads?: { nodes?: Array<{ isResolved?: boolean; path?: string; line?: number | null; comments?: { nodes?: Array<{ author?: { login?: string } | null; body?: string }> } }> } } } };
  }).data?.repository?.pullRequest;
  if (!pull?.url) throw new Error("PR 을 찾을 수 없습니다.");
  const open = (pull.reviewThreads?.nodes ?? []).filter((thread) => thread.isResolved === false);
  const lines = [`다음 PR 의 해결되지 않은 리뷰 코멘트를 확인하고 반영해 주세요.`, `PR: ${pull.url} (${pull.title ?? ""})`, ""];
  if (open.length === 0) lines.push("(해결되지 않은 리뷰 코멘트가 없습니다.)");
  open.forEach((thread, index) => {
    lines.push(`${index + 1}. ${thread.path ?? "(파일 없음)"}${thread.line ? `:${thread.line}` : ""}`);
    for (const comment of thread.comments?.nodes ?? []) {
      lines.push(`${comment.author?.login ?? "?"}: ${clip((comment.body ?? "").trim(), MAX_COMMENT_CHARS)}`);
    }
    lines.push("");
  });
  return clip(lines.join("\n").trim(), MAX_PROMPT_CHARS);
}

export async function readReviewPrompt(url: unknown, run: GhRunner = runGh): Promise<string> {
  const target = parsePullRequestUrl(url);
  if (!target) throw new Error("GitHub PR 주소가 아닙니다.");
  const json = await run([
    "api", "graphql",
    "-f", `query=${THREADS_QUERY}`,
    "-f", `owner=${target.owner}`,
    "-f", `name=${target.name}`,
    "-F", `number=${target.number}`,
  ]);
  return reviewPrompt(json);
}
