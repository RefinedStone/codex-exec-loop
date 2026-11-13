// GitHub 업무 패널의 데이터이다 (R4). 공식 CLI `gh` 로 GraphQL 을 호출한다. 자격 증명은 gh 가 OS 키체인에 둔다.
// 앱은 토큰을 읽거나 저장하지 않는다 (D-14). 모두 읽기 요청이다.
import { spawn } from "node:child_process";
import { homedir } from "node:os";

import type { GitHubProject, GitHubProjectItem, GitHubPullRequest, GitHubSummary } from "../shared/bridge";

const GH_TIMEOUT_MS = 20_000;
const OUTPUT_LIMIT = 8 << 20;
const MAX_COMMENT_CHARS = 2_000;
const MAX_PROMPT_CHARS = 20_000;
/** Project 항목을 한 번에 읽는 수이다 (GraphQL 의 상한). */
const PROJECT_PAGE_SIZE = 100;
/** Project 항목을 읽는 최대 페이지 수이다. 넘으면 뒤의 항목은 읽지 않는다. */
const MAX_PROJECT_PAGES = 5;
/** 이슈 넘기기에 넣는 최근 댓글 수이다. */
const ISSUE_COMMENT_COUNT = 10;

/** `stdin` 을 주면 gh 의 표준 입력에 쓴다 (`--body-file -` 처럼 긴 값을 인자 대신 넘길 때). */
export type GhRunner = (args: string[], stdin?: string) => Promise<string>;

/** gh 를 셸 없이 실행한다. 저장소 폴더 밖(홈)에서 실행해서 git 을 부르지 않게 하고, 대화형 질문을 끈다. */
export const runGh: GhRunner = (args, stdin) =>
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
    // gh 가 입력을 다 읽기 전에 끝날 수 있다. 실패는 종료 코드가 알린다.
    child.stdin.on("error", () => undefined);
    child.stdin.end(stdin);
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
        totalCount
        nodes { isResolved path line comments(first: 20) { totalCount nodes { author { login } body } } }
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
    data?: {
      repository?: {
        pullRequest?: {
          url?: string;
          title?: string;
          reviewThreads?: {
            totalCount?: number;
            nodes?: Array<{
              isResolved?: boolean;
              path?: string;
              line?: number | null;
              comments?: { totalCount?: number; nodes?: Array<{ author?: { login?: string } | null; body?: string }> };
            }>;
          };
        };
      };
    };
  }).data?.repository?.pullRequest;
  if (!pull?.url) throw new Error("PR 을 찾을 수 없습니다.");
  const threads = pull.reviewThreads?.nodes ?? [];
  const open = threads.filter((thread) => thread.isResolved === false);
  const lines = [`다음 PR 의 해결되지 않은 리뷰 코멘트를 확인하고 반영해 주세요.`, `PR: ${pull.url} (${pull.title ?? ""})`, ""];
  if (open.length === 0) lines.push("(해결되지 않은 리뷰 코멘트가 없습니다.)");
  open.forEach((thread, index) => {
    lines.push(`${index + 1}. ${thread.path ?? "(파일 없음)"}${thread.line ? `:${thread.line}` : ""}`);
    const comments = thread.comments?.nodes ?? [];
    for (const comment of comments) {
      lines.push(`${comment.author?.login ?? "?"}: ${clip((comment.body ?? "").trim(), MAX_COMMENT_CHARS)}`);
    }
    // 한 번에 읽는 개수보다 많으면 빠진 것이 있다고 적는다. 에이전트가 이것이 전부라고 여기지 않게 한다.
    const total = thread.comments?.totalCount ?? comments.length;
    if (total > comments.length) {
      lines.push(`(이 스레드의 코멘트 ${total}개 중 앞의 ${comments.length}개만 넣었습니다. 나머지는 PR 에서 확인하세요.)`);
    }
    lines.push("");
  });
  const totalThreads = pull.reviewThreads?.totalCount ?? threads.length;
  if (totalThreads > threads.length) {
    lines.push(`(리뷰 스레드 ${totalThreads}개 중 앞의 ${threads.length}개만 읽었습니다. 나머지는 PR 에서 확인하세요.)`);
  }
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

// ---- GitHub Project (R11) ----

/** `owner/번호` 또는 `https://github.com/(users|orgs)/<owner>/projects/<번호>` 를 받는다. 주소 뒤의 보기 경로는 무시한다. */
export function parseProjectRef(input: unknown): { owner: string; number: number } | null {
  if (typeof input !== "string") return null;
  const text = input.trim();
  const match =
    /^([A-Za-z0-9-]+)\/(\d+)$/.exec(text) ?? /^https:\/\/github\.com\/(?:users|orgs)\/([A-Za-z0-9-]+)\/projects\/(\d+)(?:[/?#].*)?$/.exec(text);
  const number = match ? Number(match[2]) : 0;
  return match && number > 0 ? { owner: match[1]!, number } : null;
}

/** User 와 Organization 이 모두 `ProjectV2Owner` 이다. 그래서 소유자 종류를 몰라도 한 쿼리로 읽는다. */
const PROJECT_QUERY = `query($owner: String!, $number: Int!, $first: Int!, $cursor: String) {
  repositoryOwner(login: $owner) {
    ... on ProjectV2Owner {
      projectV2(number: $number) {
        title url
        items(first: $first, after: $cursor) {
          pageInfo { hasNextPage endCursor }
          nodes {
            fieldValueByName(name: "Status") { ... on ProjectV2ItemFieldSingleSelectValue { name } }
            content {
              __typename
              ... on Issue { number title url state repository { nameWithOwner } }
              ... on PullRequest { number title url state repository { nameWithOwner } }
              ... on DraftIssue { title }
            }
          }
        }
      }
    }
  }
}`;

interface RawProjectItem {
  fieldValueByName?: { name?: string } | null;
  content?: {
    __typename?: string;
    number?: number;
    title?: string;
    url?: string;
    state?: string;
    repository?: { nameWithOwner?: string };
  } | null;
}

interface ProjectPage {
  title: string;
  url: string;
  items: GitHubProjectItem[];
  closedCount: number;
  nextCursor: string | null;
}

/** 항목 하나를 패널용 값으로 바꾼다. 닫힌 항목은 "closed", 모르는 항목(권한 없는 항목 등)은 null 이다. */
function projectItem(raw: RawProjectItem): GitHubProjectItem | "closed" | null {
  const content = raw?.content;
  const status = raw?.fieldValueByName?.name ?? null;
  if (content?.__typename === "DraftIssue") {
    return { kind: "draft", title: content.title ?? "", url: null, repository: null, number: null, status };
  }
  if (content?.__typename !== "Issue" && content?.__typename !== "PullRequest") return null;
  if (typeof content.number !== "number" || typeof content.url !== "string" || !content.url.startsWith("https://github.com/")) return null;
  if (content.state !== "OPEN") return "closed";
  return {
    kind: content.__typename === "Issue" ? "issue" : "pullRequest",
    title: content.title ?? "",
    url: content.url,
    repository: content.repository?.nameWithOwner ?? null,
    number: content.number,
    status,
  };
}

/** Project 쿼리의 한 페이지를 읽는다. Project 가 없으면 오류이다. */
export function parseProjectPage(json: string): ProjectPage {
  const project = (JSON.parse(json) as {
    data?: {
      repositoryOwner?: {
        projectV2?: {
          title?: string;
          url?: string;
          items?: { pageInfo?: { hasNextPage?: boolean; endCursor?: string | null }; nodes?: RawProjectItem[] };
        } | null;
      } | null;
    };
  }).data?.repositoryOwner?.projectV2;
  if (!project) throw new Error("Project 를 찾을 수 없습니다. 주소와 `read:project` 권한을 확인하세요.");
  const items: GitHubProjectItem[] = [];
  let closedCount = 0;
  for (const raw of project.items?.nodes ?? []) {
    const item = projectItem(raw);
    if (item === "closed") closedCount += 1;
    else if (item) items.push(item);
  }
  const pageInfo = project.items?.pageInfo;
  return {
    title: project.title ?? "",
    url: project.url ?? "",
    items,
    closedCount,
    nextCursor: pageInfo?.hasNextPage && pageInfo.endCursor ? pageInfo.endCursor : null,
  };
}

/** Project 의 항목을 페이지를 넘기며 읽는다. 한도를 넘으면 `truncated` 이다. */
export async function readProject(input: unknown, run: GhRunner = runGh): Promise<GitHubProject> {
  const target = parseProjectRef(input);
  if (!target) throw new Error("Project 는 `owner/번호` 또는 github.com 의 Project 주소로 적으세요.");
  const project: GitHubProject = { title: "", url: "", items: [], closedCount: 0, truncated: false };
  let cursor: string | null = null;
  for (let page = 0; page < MAX_PROJECT_PAGES; page += 1) {
    const args = [
      "api", "graphql",
      "-f", `query=${PROJECT_QUERY}`,
      "-f", `owner=${target.owner}`,
      "-F", `number=${target.number}`,
      "-F", `first=${PROJECT_PAGE_SIZE}`,
    ];
    if (cursor) args.push("-f", `cursor=${cursor}`);
    const read = parseProjectPage(await run(args));
    project.title = read.title;
    project.url = read.url;
    project.items.push(...read.items);
    project.closedCount += read.closedCount;
    cursor = read.nextCursor;
    if (!cursor) return project;
  }
  project.truncated = true;
  return project;
}

/** `https://github.com/<owner>/<name>/issues/<number>` 만 받는다. */
export function parseIssueUrl(url: unknown): { owner: string; name: string; number: number } | null {
  const match = typeof url === "string" ? /^https:\/\/github\.com\/([A-Za-z0-9-]+)\/([A-Za-z0-9._-]+)\/issues\/(\d+)$/.exec(url) : null;
  return match ? { owner: match[1]!, name: match[2]!, number: Number(match[3]) } : null;
}

const ISSUE_QUERY = `query($owner: String!, $name: String!, $number: Int!, $comments: Int!) {
  repository(owner: $owner, name: $name) {
    nameWithOwner
    issue(number: $number) {
      number url title body
      comments(last: $comments) { totalCount nodes { author { login } body } }
    }
  }
}`;

/** 이슈의 제목, 본문, 최근 댓글로 새 스레드의 첫 메시지를 만든다. 보내기는 사용자가 한다. */
export function issuePrompt(json: string): string {
  const repository = (JSON.parse(json) as {
    data?: {
      repository?: {
        nameWithOwner?: string;
        issue?: {
          number?: number;
          url?: string;
          title?: string;
          body?: string | null;
          comments?: { totalCount?: number; nodes?: Array<{ author?: { login?: string } | null; body?: string }> };
        } | null;
      } | null;
    };
  }).data?.repository;
  const issue = repository?.issue;
  if (!issue?.url || typeof issue.number !== "number") throw new Error("이슈를 찾을 수 없습니다.");
  const reference = `${repository?.nameWithOwner ?? ""}#${issue.number}`;
  const lines = [
    "다음 GitHub 이슈를 처리해 주세요. PR 을 만들면 본문에 `Closes " + reference + "` 를 적어 이슈와 연결해 주세요.",
    `이슈: ${issue.url} (${issue.title ?? ""})`,
    "",
    "## 본문",
    clip((issue.body ?? "").trim() || "(본문 없음)", MAX_PROMPT_CHARS),
  ];
  const comments = issue.comments?.nodes ?? [];
  if (comments.length > 0) {
    lines.push("", "## 최근 댓글");
    for (const comment of comments) {
      lines.push(`${comment.author?.login ?? "?"}: ${clip((comment.body ?? "").trim(), MAX_COMMENT_CHARS)}`);
    }
    const total = issue.comments?.totalCount ?? comments.length;
    if (total > comments.length) {
      lines.push(`(댓글 ${total}개 중 최근 ${comments.length}개만 넣었습니다. 나머지는 이슈에서 확인하세요.)`);
    }
  }
  return clip(lines.join("\n").trim(), MAX_PROMPT_CHARS);
}

export async function readIssuePrompt(url: unknown, run: GhRunner = runGh): Promise<string> {
  const target = parseIssueUrl(url);
  if (!target) throw new Error("GitHub 이슈 주소가 아닙니다.");
  const json = await run([
    "api", "graphql",
    "-f", `query=${ISSUE_QUERY}`,
    "-f", `owner=${target.owner}`,
    "-f", `name=${target.name}`,
    "-F", `number=${target.number}`,
    "-F", `comments=${ISSUE_COMMENT_COUNT}`,
  ]);
  return issuePrompt(json);
}
