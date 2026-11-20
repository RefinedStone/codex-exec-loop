// GitHub Project 패널(R11)의 화면 밖 규칙이다: 저장소 필터, Status 묶음, 저장소에 맞는 작업 폴더 찾기.
import type { GitHubProjectItem } from "../../shared/bridge";
import type { GatewayRequest } from "./notionHub";

/** Gateway 가 이슈를 읽고 AI 전달 정책으로 가린 공유본을 준다 (#2428). 원격의 원시 값은 이 화면에 오지 않는다. */
export const ISSUE_HANDOFF_COPY_METHOD = "akra/issue/handoffCopy";
const MAX_PROMPT_CHARS = 20_000;

/** 저장소에 맞는 폴더를 찾을 때 원격을 읽는 최대 폴더 수이다. */
const MAX_FOLDER_LOOKUPS = 20;

function sameRepository(left: string | null, right: string): boolean {
  return left !== null && left.toLowerCase() === right.toLowerCase();
}

/** `repository` 가 null 이면 전체이다. 저장소로 거르면 초안(저장소 없음)은 빠진다. */
export function filterByRepository(items: GitHubProjectItem[], repository: string | null): GitHubProjectItem[] {
  return repository === null ? items : items.filter((item) => sameRepository(item.repository, repository));
}

/** 항목에 나온 저장소이다. 이름 순이다. */
export function repositoriesOf(items: GitHubProjectItem[]): string[] {
  const names = new Map<string, string>();
  for (const item of items) {
    if (item.repository && !names.has(item.repository.toLowerCase())) names.set(item.repository.toLowerCase(), item.repository);
  }
  return [...names.values()].sort((left, right) => left.localeCompare(right));
}

export interface StatusGroup {
  /** null 은 Status 가 없는 항목이다. */
  status: string | null;
  items: GitHubProjectItem[];
}

/** Status 별로 묶는다. Project 에 처음 나온 순서를 따르고, Status 가 없는 묶음은 마지막이다. */
export function groupByStatus(items: GitHubProjectItem[]): StatusGroup[] {
  const groups = new Map<string | null, GitHubProjectItem[]>();
  for (const item of items) {
    const group = groups.get(item.status);
    if (group) group.push(item);
    else groups.set(item.status, [item]);
  }
  const result = [...groups.entries()].map(([status, grouped]) => ({ status, items: grouped }));
  return [...result.filter((group) => group.status !== null), ...result.filter((group) => group.status === null)];
}

/**
 * 저장소의 이슈를 넘길 작업 폴더를 찾는다. 앞의 폴더(열린 스레드의 폴더)부터 `origin` 을 비교한다. 없으면 null 이고,
 * 그때는 보낼 때 폴더 선택 창이 열린다. 원격을 읽지 못한 폴더는 건너뛴다.
 */
export async function findFolderForRepository(
  repository: string,
  folders: string[],
  readRepository: (cwd: string) => Promise<string | null>,
): Promise<string | null> {
  const candidates = [...new Set(folders)].slice(0, MAX_FOLDER_LOOKUPS);
  for (const cwd of candidates) {
    const found = await readRepository(cwd).catch(() => null);
    if (sameRepository(found, repository)) return cwd;
  }
  return null;
}

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max)}…(생략)` : text;
}

/**
 * 이슈의 공유본을 Gateway 에서 읽어 새 스레드의 첫 메시지를 만든다. 입력은 가린 제목과 본문뿐이다. 댓글과 복원본은
 * 넣지 않는다. `cwd` 는 이슈의 저장소에 맞는 작업 폴더이고, 그 프로젝트의 이슈 정책을 고른다. 보내기는 사용자가 한다.
 */
export async function readIssueHandoffPrompt(request: GatewayRequest, url: string, repository: string, number: number, cwd: string | null): Promise<string> {
  const [owner, name] = repository.split("/");
  const { sharedCopy } = await request<{ sharedCopy: { title: string; body: string } }>(ISSUE_HANDOFF_COPY_METHOD, {
    owner,
    repository: name,
    number,
    ...(cwd ? { cwd } : {}),
  });
  const lines = [
    "다음 GitHub 이슈를 처리해 주세요. PR 을 만들면 본문에 `Closes " + `${repository}#${number}` + "` 를 적어 이슈와 연결해 주세요.",
    `이슈: ${url} (${sharedCopy.title})`,
    "",
    "## 본문",
    clip(sharedCopy.body.trim() || "(본문 없음)", MAX_PROMPT_CHARS),
  ];
  return clip(lines.join("\n").trim(), MAX_PROMPT_CHARS);
}
