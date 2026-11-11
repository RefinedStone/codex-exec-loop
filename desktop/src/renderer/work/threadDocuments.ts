// 스레드에 나온 외부 문서이다 (plan 22). Gateway 가 스레드 기록에서 모델 없이 찾아 장부에 적은 것을 읽는다.
// 종류(`kind`)는 Gateway 의 인식기가 정한다. 여기서는 이름표만 붙인다. 모르는 종류는 종류 이름 그대로 보인다.
// "바뀜"도 Gateway 가 정한다: 이 스레드가 읽은 버전과 지금 버전이 다른 문서이다.
import type { GatewayRequest } from "./notionHub";

export const CONTEXT_THREAD_METHOD = "akra/context/thread";
/** 목록을 읽고, 확인한 지 오래된 문서의 지금 버전을 외부 서비스에 묻는다. 목록 조회보다 느리다. */
export const CONTEXT_REFRESH_METHOD = "akra/context/refresh";
/** 사용자가 바뀐 문서를 열었다. 지금 버전이 이 스레드의 읽은 버전이 된다. */
export const CONTEXT_MARK_READ_METHOD = "akra/context/markRead";

export interface ThreadDocument {
  kind: string;
  key: string;
  url: string;
  title: string;
  /** 처음 나온 곳이다: user, agent, tool */
  source: string;
  firstSeenAt: string;
  /** 이 스레드가 읽은 뒤에 문서가 바뀌었다. */
  changed: boolean;
}

export type ThreadDocumentsState =
  | { phase: "idle" }
  | { phase: "loading"; threadId: string }
  | { phase: "loaded"; threadId: string; documents: ThreadDocument[] }
  | { phase: "failed"; threadId: string; message: string };

const KIND_LABELS: Record<string, string> = { notion: "Notion", jira: "Jira", github: "GitHub" };
const SOURCE_LABELS: Record<string, string> = { user: "사용자", agent: "AI", tool: "도구" };

export function kindLabel(kind: string): string {
  return KIND_LABELS[kind] ?? kind;
}

export function sourceLabel(source: string): string {
  return SOURCE_LABELS[source] ?? source;
}

function text(value: unknown): string | undefined {
  return typeof value === "string" && value ? value : undefined;
}

/** Gateway 응답을 읽는다. 주소가 https 가 아니거나 필드가 빠진 항목은 뺀다. */
export function documentsFrom(result: unknown): ThreadDocument[] {
  const resources = (result as { resources?: unknown } | null)?.resources;
  if (!Array.isArray(resources)) throw new Error("문서 목록의 형식을 알 수 없습니다.");
  return resources.flatMap((entry) => {
    const item = (entry ?? {}) as Record<string, unknown>;
    const kind = text(item.kind);
    const key = text(item.key);
    const url = text(item.url);
    if (!kind || !key || !url?.startsWith("https://")) return [];
    return [{ kind, key, url, title: text(item.title) ?? url, source: text(item.source) ?? "", firstSeenAt: text(item.firstSeenAt) ?? "", changed: item.changed === true }];
  });
}

export function changedCount(documents: ThreadDocument[]): number {
  return documents.filter((document) => document.changed).length;
}

export async function fetchThreadDocuments(request: GatewayRequest, threadId: string): Promise<ThreadDocument[]> {
  return documentsFrom(await request(CONTEXT_THREAD_METHOD, { threadId }));
}

export async function refreshThreadDocuments(request: GatewayRequest, threadId: string): Promise<ThreadDocument[]> {
  return documentsFrom(await request(CONTEXT_REFRESH_METHOD, { threadId }));
}

export async function markThreadDocumentRead(request: GatewayRequest, threadId: string, document: ThreadDocument): Promise<ThreadDocument[]> {
  return documentsFrom(await request(CONTEXT_MARK_READ_METHOD, { threadId, kind: document.kind, key: document.key }));
}

/** 종류별로 묶는다. 종류와 문서 모두 처음 나온 순서를 지킨다. */
export function groupByKind(documents: ThreadDocument[]): { kind: string; documents: ThreadDocument[] }[] {
  const groups: { kind: string; documents: ThreadDocument[] }[] = [];
  for (const document of documents) {
    const group = groups.find((candidate) => candidate.kind === document.kind);
    if (group) group.documents.push(document);
    else groups.push({ kind: document.kind, documents: [document] });
  }
  return groups;
}
