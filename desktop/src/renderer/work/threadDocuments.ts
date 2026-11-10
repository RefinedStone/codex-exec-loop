// 스레드에 나온 외부 문서이다 (plan 22). Gateway 가 스레드 기록에서 모델 없이 찾아 장부에 적은 것을 읽는다.
// 종류(`kind`)는 Gateway 의 인식기가 정한다. 여기서는 이름표만 붙인다. 모르는 종류는 종류 이름 그대로 보인다.
import type { GatewayRequest } from "./notionHub";

export const CONTEXT_THREAD_METHOD = "akra/context/thread";

export interface ThreadDocument {
  kind: string;
  key: string;
  url: string;
  title: string;
  /** 처음 나온 곳이다: user, agent, tool */
  source: string;
  firstSeenAt: string;
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
    return [{ kind, key, url, title: text(item.title) ?? url, source: text(item.source) ?? "", firstSeenAt: text(item.firstSeenAt) ?? "" }];
  });
}

export async function fetchThreadDocuments(request: GatewayRequest, threadId: string): Promise<ThreadDocument[]> {
  return documentsFrom(await request(CONTEXT_THREAD_METHOD, { threadId }));
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
