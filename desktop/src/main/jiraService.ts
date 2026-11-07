// Jira 업무 패널의 데이터이다 (R4). Orca 와 같은 방식으로 main 이 Jira REST 를 직접 부른다.
// Cloud 는 이메일 + API 토큰(Basic, REST v3), Server/Data Center 는 PAT(Bearer, REST v2)이다.
// 토큰은 OS 암호화(safeStorage)로 userData 에만 저장하고 renderer 에 돌려주지 않는다 (D-14). 읽기 요청만 한다.
import { readFile, rename, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";

import type { JiraConnectRequest, JiraDeployment, JiraIssue, JiraStatus, JiraSummary } from "../shared/bridge";

const REQUEST_TIMEOUT_MS = 20_000;
const RESPONSE_LIMIT = 8 << 20;
const MAX_ISSUES = 30;
const MAX_COMMENTS = 10;
const MAX_DESCRIPTION_CHARS = 8_000;
const MAX_COMMENT_CHARS = 2_000;
const MAX_PROMPT_CHARS = 20_000;
const MY_OPEN_ISSUES_JQL = "assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC";
const ISSUE_KEY = /^[A-Z][A-Z0-9_]{0,49}-\d{1,9}$/;

export interface SecretBox {
  isEncryptionAvailable(): boolean;
  encryptString(text: string): Buffer;
  decryptString(data: Buffer): string;
}

export type JiraFetch = (url: string, init: { headers: Record<string, string>; signal: AbortSignal }) => Promise<{ status: number; text(): Promise<string> }>;

interface StoredConnection {
  siteUrl: string;
  deployment: JiraDeployment;
  email: string | null;
}

/** https 주소만 받는다. Server 는 경로(예: /jira)가 붙을 수 있어서 경로를 남기고, 끝의 / 와 질의·조각은 뺀다. */
export function normalizeSiteUrl(input: unknown): string | null {
  if (typeof input !== "string") return null;
  let url: URL;
  try {
    url = new URL(input.trim());
  } catch {
    return null;
  }
  // shortcut: 사내 http 전용 Server 는 받지 않는다. 그런 사이트가 실제로 있으면 그때 허용 범위를 정한다.
  if (url.protocol !== "https:" || url.username || url.password) return null;
  return `${url.origin}${url.pathname.replace(/\/+$/, "")}`;
}

function apiBase(connection: StoredConnection): string {
  return `${connection.siteUrl}/rest/api/${connection.deployment === "cloud" ? "3" : "2"}`;
}

/** Jira 문서 형식(ADF, Cloud v3)을 글로 바꾼다. Server v2 의 설명은 이미 글이다. */
export function adfToText(node: unknown): string {
  if (typeof node === "string") return node;
  if (!node || typeof node !== "object") return "";
  const item = node as { type?: string; text?: string; attrs?: { text?: string; url?: string }; content?: unknown[] };
  if (item.type === "text") return item.text ?? "";
  if (item.type === "hardBreak") return "\n";
  if (item.type === "mention" || item.type === "emoji") return item.attrs?.text ?? "";
  if (item.type === "inlineCard" || item.type === "blockCard") return item.attrs?.url ?? "";
  const inner = (item.content ?? []).map(adfToText).join("");
  // 목록 항목, 표, 인용은 안의 paragraph 가 줄을 바꾼다.
  if (item.type === "listItem") return `- ${inner}`;
  return item.type === "paragraph" || item.type === "heading" || item.type === "codeBlock" || item.type === "rule" ? `${inner}\n` : inner;
}

interface RawIssue {
  key?: string;
  fields?: {
    summary?: string;
    updated?: string;
    status?: { name?: string; statusCategory?: { key?: string } } | null;
    issuetype?: { name?: string } | null;
    priority?: { name?: string } | null;
    description?: unknown;
    comment?: { total?: number; comments?: Array<{ author?: { displayName?: string } | null; body?: unknown }> } | null;
  };
}

function issue(raw: RawIssue, siteUrl: string): JiraIssue | null {
  if (typeof raw?.key !== "string" || !ISSUE_KEY.test(raw.key)) return null;
  const fields = raw.fields ?? {};
  return {
    key: raw.key,
    summary: fields.summary ?? "",
    url: `${siteUrl}/browse/${raw.key}`,
    status: fields.status?.name ?? null,
    statusCategory: fields.status?.statusCategory?.key ?? null,
    issueType: fields.issuetype?.name ?? null,
    priority: fields.priority?.name ?? null,
    updatedAt: fields.updated ?? "",
  };
}

/** 검색 응답을 패널용 값으로 바꾼다. 키가 올바르지 않은 항목은 버린다. */
export function parseIssues(json: string, siteUrl: string): JiraIssue[] {
  const issues = (JSON.parse(json) as { issues?: RawIssue[] }).issues ?? [];
  return issues.map((raw) => issue(raw, siteUrl)).filter((item): item is JiraIssue => item !== null);
}

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max)}…(생략)` : text;
}

/** 티켓 내용으로 새 스레드의 첫 메시지를 만든다. 보내기는 사용자가 한다. */
export function issuePrompt(json: string, siteUrl: string): string {
  const raw = JSON.parse(json) as RawIssue;
  const parsed = issue(raw, siteUrl);
  if (!parsed) throw new Error("Jira 티켓을 찾을 수 없습니다.");
  const fields = raw.fields ?? {};
  const facts = [parsed.issueType && `유형: ${parsed.issueType}`, parsed.status && `상태: ${parsed.status}`, parsed.priority && `우선순위: ${parsed.priority}`];
  const lines = [`다음 Jira 티켓을 확인하고 작업해 주세요.`, `티켓: ${parsed.key} ${parsed.summary} (${parsed.url})`];
  const factLine = facts.filter(Boolean).join(" · ");
  if (factLine) lines.push(factLine);
  const description = adfToText(fields.description).trim();
  lines.push("", "설명:", description ? clip(description, MAX_DESCRIPTION_CHARS) : "(설명이 없습니다.)");
  const comments = fields.comment?.comments ?? [];
  const recent = comments.slice(-MAX_COMMENTS);
  if (recent.length > 0) {
    lines.push("", "최근 댓글:");
    for (const comment of recent) {
      lines.push(`${comment.author?.displayName ?? "?"}: ${clip(adfToText(comment.body).trim(), MAX_COMMENT_CHARS)}`);
    }
  }
  // 빠진 댓글이 있다고 적는다. 에이전트가 이것이 전부라고 여기지 않게 한다.
  const total = fields.comment?.total ?? comments.length;
  if (total > recent.length) {
    lines.push(`(댓글 ${total}개 중 최근 ${recent.length}개만 넣었습니다. 나머지는 티켓에서 확인하세요.)`);
  }
  return clip(lines.join("\n").trim(), MAX_PROMPT_CHARS);
}

function authorization(connection: StoredConnection, token: string): string {
  return connection.email ? `Basic ${Buffer.from(`${connection.email}:${token}`).toString("base64")}` : `Bearer ${token}`;
}

/** 연결 요청을 검증한다. Cloud 는 이메일이 필요하고, Server 는 PAT 만 쓴다. */
export function validateConnectRequest(input: unknown): { connection: StoredConnection; token: string } {
  const raw = (input ?? {}) as Partial<Record<keyof JiraConnectRequest, unknown>>;
  const siteUrl = normalizeSiteUrl(raw.siteUrl);
  if (!siteUrl) throw new Error("Jira 주소는 https:// 로 시작해야 합니다.");
  const deployment = raw.deployment === "server" ? "server" : raw.deployment === "cloud" ? "cloud" : null;
  if (!deployment) throw new Error("Jira 종류(Cloud 또는 Server)를 고르세요.");
  const token = typeof raw.token === "string" ? raw.token.trim() : "";
  if (!token) throw new Error("토큰을 입력하세요.");
  const email = typeof raw.email === "string" && raw.email.trim() ? raw.email.trim() : null;
  if (deployment === "cloud" && !email) throw new Error("Jira Cloud 는 Atlassian 계정 이메일이 필요합니다.");
  return { connection: { siteUrl, deployment, email: deployment === "cloud" ? email : null }, token };
}

const defaultFetch: JiraFetch = async (url, init) => {
  // Chromium 네트워크(net.fetch)를 쓴다. 회사 프록시와 인증서 설정을 따른다.
  const { net } = await import("electron");
  return net.fetch(url, { headers: init.headers, signal: init.signal, credentials: "omit" });
};

export class JiraService {
  constructor(
    private readonly directory: string,
    private readonly secrets: SecretBox,
    private readonly fetcher: JiraFetch = defaultFetch,
  ) {}

  private get connectionPath() {
    return join(this.directory, "jira-connection.json");
  }

  private get tokenPath() {
    return join(this.directory, "jira-token.bin");
  }

  private async load(): Promise<{ connection: StoredConnection; token: string } | null> {
    try {
      const stored = JSON.parse(await readFile(this.connectionPath, "utf8")) as Partial<StoredConnection>;
      const siteUrl = normalizeSiteUrl(stored.siteUrl);
      const deployment = stored.deployment === "cloud" || stored.deployment === "server" ? stored.deployment : null;
      if (!siteUrl || !deployment) return null;
      const token = this.secrets.decryptString(await readFile(this.tokenPath));
      return { connection: { siteUrl, deployment, email: typeof stored.email === "string" ? stored.email : null }, token };
    } catch {
      return null;
    }
  }

  private async get(connection: StoredConnection, token: string, path: string): Promise<string> {
    let response: { status: number; text(): Promise<string> };
    try {
      response = await this.fetcher(`${apiBase(connection)}${path}`, {
        headers: { Accept: "application/json", Authorization: authorization(connection, token) },
        signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
      });
    } catch (error) {
      throw new Error(`Jira 에 연결할 수 없습니다: ${error instanceof Error ? error.message : String(error)}`);
    }
    if (response.status === 401 || response.status === 403) {
      throw new Error("Jira 인증에 실패했습니다. 토큰을 확인하고 다시 연결하세요.");
    }
    if (response.status === 404) throw new Error("Jira 에서 찾을 수 없습니다. 주소와 종류(Cloud/Server)를 확인하세요.");
    if (response.status < 200 || response.status >= 300) throw new Error(`Jira 요청이 실패했습니다 (HTTP ${response.status}).`);
    const body = await response.text();
    if (body.length > RESPONSE_LIMIT) throw new Error("Jira 응답이 너무 큽니다.");
    return body;
  }

  private async require(): Promise<{ connection: StoredConnection; token: string }> {
    const stored = await this.load();
    if (!stored) throw new Error("Jira 가 연결되어 있지 않습니다.");
    return stored;
  }

  async status(): Promise<JiraStatus> {
    const stored = await this.load();
    return stored ? { connected: true, ...stored.connection } : { connected: false };
  }

  /** 토큰으로 `myself` 를 읽어서 확인한 뒤에만 저장한다. */
  async connect(input: unknown): Promise<JiraStatus> {
    const { connection, token } = validateConnectRequest(input);
    if (!this.secrets.isEncryptionAvailable()) {
      throw new Error("이 시스템에서 OS 암호화를 쓸 수 없어서 토큰을 저장하지 않습니다.");
    }
    await this.get(connection, token, "/myself");
    await writeAtomically(this.tokenPath, this.secrets.encryptString(token));
    await writeAtomically(this.connectionPath, `${JSON.stringify(connection, null, 2)}\n`);
    return { connected: true, ...connection };
  }

  async disconnect(): Promise<JiraStatus> {
    await rm(this.tokenPath, { force: true });
    await rm(this.connectionPath, { force: true });
    return { connected: false };
  }

  async summary(): Promise<JiraSummary> {
    const { connection, token } = await this.require();
    const fields = "summary,status,issuetype,priority,updated";
    const query = `jql=${encodeURIComponent(MY_OPEN_ISSUES_JQL)}&maxResults=${MAX_ISSUES}&fields=${fields}`;
    // Cloud 의 예전 /search 는 없어졌다. Cloud 는 /search/jql, Server 는 /search 를 쓴다.
    const path = connection.deployment === "cloud" ? `/search/jql?${query}` : `/search?${query}`;
    return { siteUrl: connection.siteUrl, issues: parseIssues(await this.get(connection, token, path), connection.siteUrl) };
  }

  async issuePrompt(key: unknown): Promise<string> {
    if (typeof key !== "string" || !ISSUE_KEY.test(key)) throw new Error("Jira 티켓 키가 올바르지 않습니다.");
    const { connection, token } = await this.require();
    const fields = "summary,status,issuetype,priority,updated,description,comment";
    return issuePrompt(await this.get(connection, token, `/issue/${key}?fields=${fields}`), connection.siteUrl);
  }

  /** 연결된 사이트의 티켓 주소인지 확인한다. 기본 브라우저로 열어도 되는 주소이다. */
  async isSiteUrl(url: unknown): Promise<boolean> {
    const stored = await this.load();
    if (!stored || typeof url !== "string") return false;
    return url.startsWith(`${stored.connection.siteUrl}/browse/`) && ISSUE_KEY.test(url.slice(`${stored.connection.siteUrl}/browse/`.length));
  }
}

/** 임시 파일에 쓰고 바꿔 넣는다. 쓰는 도중에 앱이 끝나도 이전 파일이 남는다. 다른 사용자가 읽지 못하게 한다. */
async function writeAtomically(path: string, data: string | Buffer): Promise<void> {
  const temporary = `${path}.tmp`;
  await writeFile(temporary, data, { mode: 0o600 });
  await rename(temporary, path);
}
