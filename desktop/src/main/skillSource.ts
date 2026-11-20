// 스킬의 원본을 공개 GitHub 에서 읽는다 (#2383). 인증 없이 읽기만 한다. 받은 내용은 실행하지 않는다.
// ref 는 항상 커밋으로 해소하고, 파일은 그 커밋의 트리에 있는 것만 받는다.
import { createHash } from "node:crypto";

import type { SkillCheckFailureKind, SkillSource, SkillSourceInput, SkillTrackingPolicy } from "../shared/skills";
import { assertBundle, relativePathProblem, SKILL_LIMITS, SkillBundleError, type SkillFile } from "./skillManifest";

export interface SkillFetchResponse {
  status: number;
  headers: { get(name: string): string | null };
  arrayBuffer(): Promise<ArrayBuffer>;
}
export type SkillFetch = (url: string, init: { headers: Record<string, string>; signal: AbortSignal; redirect: "manual" }) => Promise<SkillFetchResponse>;

/** 확인이 실패한 이유이다. 장부는 이 종류를 적고 마지막 성공 결과를 남긴다. */
export class SkillSourceError extends Error {
  constructor(
    readonly kind: SkillCheckFailureKind,
    message: string,
  ) {
    super(message);
  }
}

const API = "https://api.github.com";
const RAW = "https://raw.githubusercontent.com";
const TIMEOUT_MS = 20_000;
const REPOSITORY = /^[a-z0-9](?:[a-z0-9-]{0,38})\/(?!\.{1,2}$)[a-z0-9._-]{1,100}$/;
const COMMIT = /^[0-9a-f]{40}$/;

function invalid(message: string): never {
  throw new SkillSourceError("invalid", message);
}

function policyOf(raw: unknown): SkillTrackingPolicy {
  const policy = (raw ?? {}) as { kind?: unknown; branch?: unknown; ref?: unknown };
  const refOf = (value: unknown, label: string): string => {
    const ref = typeof value === "string" ? value.trim() : "";
    // git 이 ref 이름에 받지 않는 글자와 주소를 바꿀 수 있는 글자를 거절한다.
    if (!ref || ref.length > 200 || /[\s~^:?*[\\\0-\x1f]|\.\.|^\/|\/$|\/\/|@\{/.test(ref)) invalid(`${label} 이름이 올바르지 않습니다.`);
    return ref;
  };
  if (policy.kind === "release") return { kind: "release" };
  if (policy.kind === "branch") return { kind: "branch", branch: refOf(policy.branch, "브랜치") };
  if (policy.kind === "pinned") return { kind: "pinned", ref: refOf(policy.ref, "고정 ref") };
  return invalid("추적 정책을 고르세요.");
}

/** 사용자가 쓴 출처를 정규화한다. `owner/name` 또는 github.com 주소를 받는다. */
export function normalizeSource(raw: unknown): SkillSource {
  const input = (raw ?? {}) as Partial<SkillSourceInput>;
  const text = typeof input.repository === "string" ? input.repository.trim() : "";
  const repository = text
    .replace(/^https:\/\/github\.com\//i, "")
    .replace(/\.git$/i, "")
    .replace(/\/+$/, "")
    .toLowerCase();
  if (!REPOSITORY.test(repository)) invalid("저장소는 공개 GitHub 의 owner/name 또는 https://github.com/owner/name 이어야 합니다.");
  const subPath = (typeof input.subPath === "string" ? input.subPath.trim() : "").replace(/^\/+|\/+$/g, "");
  if (subPath !== "") {
    const problem = relativePathProblem(subPath);
    if (problem) invalid(`스킬 하위 경로가 올바르지 않습니다 (${problem}).`);
  }
  return { repository, subPath, policy: policyOf(input.policy) };
}

function encodePath(path: string): string {
  return path.split("/").map(encodeURIComponent).join("/");
}

async function request(fetcher: SkillFetch, url: string, accept: string): Promise<SkillFetchResponse> {
  let response: SkillFetchResponse;
  try {
    response = await fetcher(url, {
      headers: { Accept: accept, "User-Agent": "akra-desktop", "X-GitHub-Api-Version": "2022-11-28" },
      signal: AbortSignal.timeout(TIMEOUT_MS),
      redirect: "manual",
    });
  } catch (error) {
    throw new SkillSourceError("offline", `GitHub 에 닿지 못했습니다: ${error instanceof Error ? error.message : String(error)}`);
  }
  if (response.status === 200) return response;
  // 이름을 바꾸거나 옮긴 저장소는 다른 주소로 보낸다. 따라가지 않는다: 사용자가 새 출처를 직접 연결한다.
  if (response.status === 0 || (response.status >= 300 && response.status < 400)) throw new SkillSourceError("moved", "원본 저장소가 다른 주소로 옮겨졌습니다.");
  if (response.status === 403 || response.status === 429) throw new SkillSourceError("rateLimited", "GitHub 요청 한도를 넘었습니다. 나중에 다시 확인하세요.");
  if (response.status === 404 || response.status === 409 || response.status === 422) throw new SkillSourceError("missing", "원본에서 찾지 못했습니다 (저장소, ref 또는 경로가 없습니다).");
  throw new SkillSourceError("invalid", `GitHub 가 HTTP ${response.status} 로 답했습니다.`);
}

async function requestJson(fetcher: SkillFetch, url: string): Promise<unknown> {
  const response = await request(fetcher, url, "application/vnd.github+json");
  try {
    return JSON.parse(Buffer.from(await response.arrayBuffer()).toString("utf8"));
  } catch {
    throw new SkillSourceError("invalid", "GitHub 의 답을 읽지 못했습니다.");
  }
}

/** 최신 안정 릴리스의 태그이다. GitHub 의 `releases/latest` 는 prerelease 와 draft 를 뺀다. */
async function latestReleaseTag(fetcher: SkillFetch, repository: string): Promise<string> {
  try {
    const release = (await requestJson(fetcher, `${API}/repos/${repository}/releases/latest`)) as { tag_name?: unknown };
    if (typeof release.tag_name !== "string" || release.tag_name === "") throw new SkillSourceError("invalid", "릴리스에 태그가 없습니다.");
    return release.tag_name;
  } catch (error) {
    if (!(error instanceof SkillSourceError) || error.kind !== "missing") throw error;
    // 404 는 저장소가 없을 때도 온다. 저장소가 있으면 안정 릴리스가 없는 것이다. 브랜치로 바꾸지 않는다.
    await requestJson(fetcher, `${API}/repos/${repository}`);
    throw new SkillSourceError("noRelease", "안정 릴리스가 없습니다. 브랜치를 추적하려면 추적 정책을 직접 바꾸세요.");
  }
}

/** 정책이 가리키는 ref 를 커밋으로 해소한다. */
export async function resolveCommit(fetcher: SkillFetch, source: SkillSource): Promise<{ ref: string; commit: string }> {
  const policy = source.policy;
  const [ref, lookup] =
    policy.kind === "release"
      ? await latestReleaseTag(fetcher, source.repository).then((tag) => [tag, `tags/${tag}`] as const)
      : policy.kind === "branch"
        ? ([policy.branch, `heads/${policy.branch}`] as const)
        : ([policy.ref, policy.ref] as const);
  const response = await request(fetcher, `${API}/repos/${source.repository}/commits/${encodePath(lookup)}`, "application/vnd.github.sha");
  const commit = Buffer.from(await response.arrayBuffer()).toString("utf8").trim();
  if (!COMMIT.test(commit)) throw new SkillSourceError("invalid", "GitHub 가 커밋을 돌려주지 않았습니다.");
  return { ref, commit };
}

interface TreeEntry {
  path?: unknown;
  mode?: unknown;
  type?: unknown;
  sha?: unknown;
  size?: unknown;
}

function gitBlobSha(bytes: Buffer): string {
  return createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
}

/**
 * 커밋의 스킬 파일 묶음을 받는다. `subPath` 아래의 파일만 받는다: 저장소의 다른 스킬이나 루트 문서는 넣지 않는다.
 * 링크(120000)와 서브모듈은 묶음 전체를 거절한다. 받은 파일은 트리의 blob 해시와 같아야 한다.
 */
export async function fetchBundle(fetcher: SkillFetch, repository: string, subPath: string, commit: string): Promise<SkillFile[]> {
  if (!COMMIT.test(commit)) throw new SkillSourceError("invalid", "커밋이 올바르지 않습니다.");
  const tree = (await requestJson(fetcher, `${API}/repos/${repository}/git/trees/${commit}?recursive=1`)) as { tree?: unknown; truncated?: unknown };
  // shortcut: 잘린 트리는 읽지 않는다. 파일이 아주 많은 저장소가 필요해지면 하위 트리를 따로 읽는다.
  if (tree.truncated === true || !Array.isArray(tree.tree)) throw new SkillSourceError("invalid", "저장소가 너무 커서 파일 목록을 읽지 못했습니다.");
  const prefix = subPath === "" ? "" : `${subPath}/`;
  const wanted: { path: string; sha: string; size: number; executable: boolean }[] = [];
  for (const entry of tree.tree as TreeEntry[]) {
    if (typeof entry.path !== "string" || !entry.path.startsWith(prefix) || entry.type === "tree") continue;
    const path = entry.path.slice(prefix.length);
    if (path === ".DS_Store" || path.endsWith("/.DS_Store")) continue;
    if (entry.type !== "blob" || (entry.mode !== "100644" && entry.mode !== "100755")) {
      throw new SkillSourceError("invalid", `원본에 링크나 서브모듈이 있어 받지 않습니다: ${path}`);
    }
    if (typeof entry.sha !== "string" || typeof entry.size !== "number") throw new SkillSourceError("invalid", "GitHub 의 파일 목록을 읽지 못했습니다.");
    const problem = relativePathProblem(path);
    if (problem) throw new SkillSourceError("invalid", `원본에 받을 수 없는 경로가 있습니다 (${problem}): ${JSON.stringify(path)}`);
    if (entry.size > SKILL_LIMITS.fileBytes) throw new SkillSourceError("invalid", `원본 파일이 너무 큽니다: ${path}`);
    wanted.push({ path, sha: entry.sha, size: entry.size, executable: entry.mode === "100755" });
  }
  if (!wanted.some((file) => file.path === "SKILL.md")) throw new SkillSourceError("missing", "원본의 그 경로에 SKILL.md 가 없습니다 (스킬이 옮겨졌거나 지워졌습니다).");
  if (wanted.length > SKILL_LIMITS.files || wanted.reduce((sum, file) => sum + file.size, 0) > SKILL_LIMITS.totalBytes) {
    throw new SkillSourceError("invalid", "원본 파일 묶음이 한도보다 큽니다.");
  }
  const files: SkillFile[] = [];
  for (const file of wanted) {
    const response = await request(fetcher, `${RAW}/${repository}/${commit}/${encodePath(prefix + file.path)}`, "*/*");
    const bytes = Buffer.from(await response.arrayBuffer());
    if (bytes.length !== file.size || gitBlobSha(bytes) !== file.sha) throw new SkillSourceError("invalid", `받은 파일이 커밋의 내용과 다릅니다: ${file.path}`);
    files.push({ path: file.path, bytes, executable: file.executable });
  }
  try {
    assertBundle(files);
  } catch (error) {
    throw new SkillSourceError("invalid", error instanceof SkillBundleError ? error.message : String(error));
  }
  return files;
}
