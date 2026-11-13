// GitHub Project 패널(R11)의 화면 밖 규칙이다: 저장소 필터, Status 묶음, 저장소에 맞는 작업 폴더 찾기.
import type { GitHubProjectItem } from "../../shared/bridge";

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
