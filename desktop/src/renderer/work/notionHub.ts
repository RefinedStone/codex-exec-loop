// Notion 패널이 Gateway 의 MCP hub 를 쓰는 길이다 (plan 21 3단계). 로그인과 토큰은 hub 가 가진다. 패널은
// `akra/mcp/statuses` 로 연결 상태와 도구 목록을 읽고, `akra/mcp/call` 로 Notion MCP 도구를 부른다.
// Notion MCP 결과에는 출력 스키마가 없다. 결과 JSON 을 방어적으로 읽고, 모르는 모양이면 오류로 알린다.

export const NOTION_SERVER = "notion";
export const MCP_METHOD = { statuses: "akra/mcp/statuses", call: "akra/mcp/call" } as const;

/** 새 스레드 첫 메시지의 최대 길이이다. Jira 패널과 같다. */
const MAX_PROMPT_CHARS = 20_000;
const MAX_PAGE_TEXT_CHARS = 16_000;
const RECENT_LIMIT = 30;

/** 서버마다 이름이 다르다. 공식 Notion MCP 는 `notion-` 을 붙이고, ChatGPT 커넥터는 붙이지 않는다. */
const TOOL_CANDIDATES = {
  search: ["notion-search", "search"],
  fetch: ["notion-fetch", "fetch"],
  recent: ["notion-list-recent-pages"],
} as const;

export type GatewayRequest = <T>(method: string, params: unknown) => Promise<T>;

export interface NotionTools {
  search: string;
  fetch: string;
  recent?: string;
}

export type NotionHubState =
  | { kind: "unregistered" }
  | { kind: "needsLogin"; detail?: string }
  | { kind: "failed"; detail: string }
  | { kind: "ready"; tools: NotionTools };

export interface NotionPage {
  title: string;
  url: string;
  type?: string;
}

export interface NotionFetchedPage {
  title: string;
  url: string;
  text: string;
  path?: string;
  lastEditedAt?: string;
}

interface WireStatus {
  name?: string;
  auth?: string;
  connection?: string;
  tools?: { name?: string }[];
  error?: string | null;
}

interface ToolResult {
  content?: { type?: string; text?: string }[];
  structuredContent?: unknown;
  isError?: boolean;
}

function record(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : undefined;
}

function text(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function pick(available: Set<string>, candidates: readonly string[]): string | undefined {
  return candidates.find((name) => available.has(name));
}

/** `akra/mcp/statuses` 결과에서 Notion 서버의 상태를 읽는다. */
export function hubState(result: unknown): NotionHubState {
  const statuses = (record(result)?.statuses ?? []) as WireStatus[];
  const status = Array.isArray(statuses) ? statuses.find((entry) => entry?.name === NOTION_SERVER) : undefined;
  if (!status) return { kind: "unregistered" };
  const available = new Set((status.tools ?? []).map((tool) => tool?.name).filter((name): name is string => !!name));
  const detail = status.error ?? undefined;
  if (status.auth === "notLoggedIn" || status.connection === "authenticationRequired") {
    return detail ? { kind: "needsLogin", detail } : { kind: "needsLogin" };
  }
  const search = pick(available, TOOL_CANDIDATES.search);
  const fetch = pick(available, TOOL_CANDIDATES.fetch);
  if (!search || !fetch) {
    return { kind: "failed", detail: detail ?? "Notion 서버에서 검색 도구를 찾지 못했습니다." };
  }
  const recent = pick(available, TOOL_CANDIDATES.recent);
  return { kind: "ready", tools: recent ? { search, fetch, recent } : { search, fetch } };
}

/** 도구 결과의 JSON 본문이다. 구조화 결과가 있으면 그것을, 없으면 첫 글 내용을 JSON 으로 읽는다. */
export function toolPayload(result: unknown): unknown {
  const tool = (record(result) ?? {}) as ToolResult;
  const firstText = tool.content?.find((item) => item?.type === "text")?.text;
  if (tool.isError) {
    throw new Error(firstText?.trim() || "Notion 도구가 오류를 돌려주었습니다.");
  }
  if (tool.structuredContent !== undefined && tool.structuredContent !== null) {
    return tool.structuredContent;
  }
  if (firstText === undefined) throw new Error("Notion 도구가 내용을 돌려주지 않았습니다.");
  try {
    return JSON.parse(firstText) as unknown;
  } catch {
    throw new Error("Notion 도구의 결과 형식을 알 수 없습니다.");
  }
}

/** 검색이나 최근 목록 결과의 페이지들이다. 제목이나 주소가 없는 항목은 뺀다. */
export function pagesFrom(result: unknown): NotionPage[] {
  const results = record(toolPayload(result))?.results;
  if (!Array.isArray(results)) throw new Error("Notion 결과에 목록이 없습니다.");
  return results.flatMap((entry) => {
    const item = record(entry);
    const url = text(item?.url);
    if (!url) return [];
    const type = text(item?.type);
    return [{ title: text(item?.title) ?? "(제목 없음)", url, ...(type ? { type } : {}) }];
  });
}

export function fetchedPageFrom(result: unknown): NotionFetchedPage {
  const page = record(toolPayload(result));
  const url = text(page?.url);
  if (!page || !url) throw new Error("Notion 페이지를 읽지 못했습니다.");
  const path = text(page.path);
  const lastEditedAt = text(page.page_last_edited_at);
  return {
    title: text(page.title) ?? "(제목 없음)",
    url,
    text: typeof page.text === "string" ? page.text : "",
    ...(path ? { path } : {}),
    ...(lastEditedAt ? { lastEditedAt } : {}),
  };
}

function clip(value: string, max: number): string {
  return value.length > max ? `${value.slice(0, max)}…(생략)` : value;
}

/** 페이지 내용으로 새 스레드의 첫 메시지를 만든다. 보내기는 사용자가 한다. */
export function pagePrompt(page: NotionFetchedPage): string {
  const lines = ["다음 Notion 페이지를 확인하고 작업해 주세요.", `페이지: ${page.title} (${page.url})`];
  const facts = [page.path && `위치: ${page.path}`, page.lastEditedAt && `마지막 수정: ${page.lastEditedAt}`].filter(Boolean);
  if (facts.length > 0) lines.push(facts.join(" · "));
  lines.push("", "내용:", page.text.trim() ? clip(page.text.trim(), MAX_PAGE_TEXT_CHARS) : "(내용이 없습니다.)");
  // 에이전트가 이 글을 낡은 복사본으로 다루고, 고칠 때는 hub 로 다시 읽게 한다.
  lines.push("", `페이지를 다시 읽거나 고칠 때는 \`akra mcp\` 의 \`${NOTION_SERVER}\` 서버 도구를 쓰세요.`);
  return clip(lines.join("\n").trim(), MAX_PROMPT_CHARS);
}

export class NotionHub {
  constructor(private readonly request: GatewayRequest) {}

  async state(): Promise<NotionHubState> {
    return hubState(await this.request(MCP_METHOD.statuses, { server: NOTION_SERVER }));
  }

  /** 최근 본 페이지이다. 서버에 그 도구가 없으면 빈 목록이다. */
  async recent(tools: NotionTools): Promise<NotionPage[]> {
    if (!tools.recent) return [];
    return pagesFrom(await this.call(tools.recent, { limit: RECENT_LIMIT }));
  }

  async search(tools: NotionTools, query: string): Promise<NotionPage[]> {
    // 서버마다 다른 선택 인자는 보내지 않는다. 모르는 인자를 거절하는 서버가 있다.
    return pagesFrom(await this.call(tools.search, { query }));
  }

  async pagePrompt(tools: NotionTools, url: string): Promise<string> {
    return pagePrompt(fetchedPageFrom(await this.call(tools.fetch, { id: url })));
  }

  private call(tool: string, args: Record<string, unknown>): Promise<unknown> {
    return this.request(MCP_METHOD.call, { server: NOTION_SERVER, tool, arguments: args });
  }
}
