// 턴의 아이템을 화면 덩어리로 묶는다. 이어진 도구 호출(명령, 파일 변경, MCP 도구, 웹 검색)은 작업 묶음 하나가 된다. 순수 함수이다.
import type { ThreadItem } from "../../shared/protocol/generated/v2/ThreadItem";
import type { TurnStatus } from "../../shared/protocol/generated/v2/TurnStatus";

const STEP_TYPES = ["commandExecution", "fileChange", "mcpToolCall", "dynamicToolCall", "collabAgentToolCall", "webSearch"] as const;
export type StepItem = Extract<ThreadItem, { type: (typeof STEP_TYPES)[number] }>;
export type StepKind = "read" | "search" | "edit" | "command" | "tool" | "web";
/** `unknown` 은 끝을 받지 못한 실행이다. 완료로 보지 않는다 (#2378). */
export type StepStatus = "running" | "ok" | "failed" | "interrupted" | "unknown";

export type TurnBlock =
  | { type: "user"; id: string; text: string }
  | { type: "reasoning"; id: string; text: string }
  | { type: "activity"; id: string; steps: StepItem[] }
  | { type: "agent"; id: string; text: string }
  /** 따로 모양을 정하지 않은 아이템이다. `ItemView` 가 그린다. */
  | { type: "other"; id: string; item: ThreadItem };

function isStep(item: ThreadItem): item is StepItem {
  return (STEP_TYPES as readonly string[]).includes(item.type);
}

function userText(item: Extract<ThreadItem, { type: "userMessage" }>): string {
  return item.content.map((input) => (input.type === "text" ? input.text : `[${input.type}]`)).join("\n");
}

export function turnBlocks(items: readonly ThreadItem[]): TurnBlock[] {
  const blocks: TurnBlock[] = [];
  for (const item of items) {
    const last = blocks.at(-1);
    if (isStep(item)) {
      if (last?.type === "activity") {
        last.steps.push(item);
      } else {
        blocks.push({ type: "activity", id: item.id, steps: [item] });
      }
    } else if (item.type === "userMessage") {
      blocks.push({ type: "user", id: item.id, text: userText(item) });
    } else if (item.type === "agentMessage") {
      blocks.push({ type: "agent", id: item.id, text: item.text });
    } else if (item.type === "reasoning") {
      // 요약이 없는 생각 아이템은 보여 줄 것이 없다.
      if (item.summary.length > 0) blocks.push({ type: "reasoning", id: item.id, text: item.summary.join("\n") });
    } else {
      blocks.push({ type: "other", id: item.id, item });
    }
  }
  return blocks;
}

/** 명령이 파일 읽기나 검색 하나이면 그 종류로, 아니면 명령으로 본다. */
export function stepKind(item: StepItem): StepKind {
  if (item.type === "fileChange") return "edit";
  if (item.type === "webSearch") return "web";
  if (item.type !== "commandExecution") return "tool";
  const [action, ...rest] = item.commandActions ?? [];
  if (action && rest.length === 0 && (action.type === "read" || action.type === "search")) return action.type;
  return "command";
}

/** 검색어, 연 페이지의 주소, 페이지 안에서 찾은 글을 쓴다. 동작이 없으면 아이템의 검색어를 쓴다. */
function webSearchLabel(item: Extract<StepItem, { type: "webSearch" }>): string {
  const action = item.action;
  if (action?.type === "search") return action.queries?.join(", ") || action.query || item.query;
  if (action?.type === "openPage") return action.url || item.query;
  if (action?.type === "findInPage") return [action.pattern, action.url].filter(Boolean).join(" · ") || item.query;
  return item.query;
}

export function stepLabel(item: StepItem): string {
  if (item.type === "webSearch") return webSearchLabel(item);
  if (item.type === "fileChange") return item.changes.map((change) => change.path).join(", ");
  if (item.type === "mcpToolCall") return `${item.server}.${item.tool}`;
  if (item.type === "dynamicToolCall") return [item.namespace, item.tool].filter(Boolean).join(".");
  if (item.type === "collabAgentToolCall") return item.tool;
  const action = item.commandActions?.[0];
  const kind = stepKind(item);
  if (kind === "read" && action?.type === "read") return action.name;
  if (kind === "search" && action?.type === "search") return [action.query, action.path].filter(Boolean).join(" · ") || item.command;
  return item.command;
}

const SUMMARY_LIMIT = 120;
// shortcut: 흔한 모양(`KEY=값`, `--token 값`, `Bearer 값`)만 가린다. 다른 모양의 비밀이 보이면 규칙을 더한다.
const SECRET_VALUE = /((?:token|secret|passw(?:or)?d|api[-_]?key|authorization|credential)[\w-]*(?:\s*[=:]\s*|\s+)|bearer\s+)("[^"]*"|'[^']*'|(?:bearer\s+)?\S+)/gi;

/** 접힌 카드에 쓰는 대상 요약이다 (#2378). 첫 줄만 쓰고, 비밀 값은 가리고, 길면 자른다. 전체 글은 펼친 카드에 있다. */
export function stepSummary(item: StepItem): string {
  const line = (stepLabel(item).trim().split("\n")[0] ?? "").replace(SECRET_VALUE, "$1***");
  return line.length > SUMMARY_LIMIT ? `${line.slice(0, SUMMARY_LIMIT)}…` : line;
}

/** 아이템의 상태와 턴의 상태로 정한다. 턴이 끝났는데 아직 실행 중이라는 아이템은 끝을 확인할 수 없다. */
export function stepStatus(item: StepItem, turnStatus: TurnStatus): StepStatus {
  // 웹 검색 아이템에는 상태가 없다.
  if (item.type === "webSearch") return "ok";
  if (item.status === "completed") return "ok";
  if (item.status === "interrupted") return "interrupted";
  if (item.status !== "inProgress") return "failed";
  if (turnStatus === "inProgress") return "running";
  return turnStatus === "interrupted" ? "interrupted" : "unknown";
}

function textParts(parts: readonly unknown[]): string | null {
  const text = parts.flatMap((part) => (typeof (part as { text?: unknown } | null)?.text === "string" ? [(part as { text: string }).text] : []));
  return text.length > 0 ? text.join("\n") : null;
}

/** 펼친 카드에 쓰는 보존된 출력과 오류이다 (#2378). 없으면 null 이다. */
export function stepDetail(item: StepItem): { output: string | null; error: string | null } {
  if (item.type === "commandExecution") {
    const output = [`$ ${item.command}`, item.aggregatedOutput].filter(Boolean).join("\n");
    return { output, error: item.exitCode ? `종료 코드 ${item.exitCode}` : null };
  }
  if (item.type === "mcpToolCall") return { output: textParts(item.result?.content ?? []), error: item.error?.message ?? null };
  if (item.type === "dynamicToolCall") return { output: textParts(item.contentItems ?? []), error: null };
  return { output: null, error: null };
}

export function activitySummary(steps: readonly StepItem[]): string {
  const count = (kind: StepKind) => steps.filter((step) => stepKind(step) === kind).length;
  const parts = [
    count("read") > 0 ? `파일 ${count("read")}개 읽음` : null,
    count("search") > 0 ? `검색 ${count("search")}회` : null,
    count("edit") > 0 ? `파일 변경 ${count("edit")}회` : null,
    count("command") > 0 ? `명령 ${count("command")}개 실행` : null,
    count("tool") > 0 ? `도구 ${count("tool")}회` : null,
    count("web") > 0 ? `웹 검색 ${count("web")}회` : null,
  ];
  return parts.filter(Boolean).join(" · ");
}

/** 걸린 시간을 짧게 쓴다: `0.4초`, `12초`, `2분 14초`. */
export function formatDuration(ms: number): string {
  if (ms < 10_000) return `${(ms / 1000).toFixed(1)}초`;
  const seconds = Math.round(ms / 1000);
  return seconds >= 60 ? `${Math.floor(seconds / 60)}분 ${seconds % 60}초` : `${seconds}초`;
}
