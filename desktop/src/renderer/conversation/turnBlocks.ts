// 턴의 아이템을 화면 덩어리로 묶는다. 이어진 도구 호출(명령, 파일 변경, MCP 도구)은 작업 묶음 하나가 된다. 순수 함수이다.
import type { ThreadItem } from "../../shared/protocol/generated/v2/ThreadItem";

export type StepItem = Extract<ThreadItem, { type: "commandExecution" | "fileChange" | "mcpToolCall" }>;
export type StepKind = "read" | "search" | "edit" | "command" | "tool";
export type StepStatus = "running" | "ok" | "failed";

export type TurnBlock =
  | { type: "user"; id: string; text: string }
  | { type: "reasoning"; id: string; text: string }
  | { type: "activity"; id: string; steps: StepItem[] }
  | { type: "agent"; id: string; text: string }
  /** 따로 모양을 정하지 않은 아이템이다. `ItemView` 가 그린다. */
  | { type: "other"; id: string; item: ThreadItem };

function isStep(item: ThreadItem): item is StepItem {
  return item.type === "commandExecution" || item.type === "fileChange" || item.type === "mcpToolCall";
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
  if (item.type === "mcpToolCall") return "tool";
  const [action, ...rest] = item.commandActions ?? [];
  if (action && rest.length === 0 && (action.type === "read" || action.type === "search")) return action.type;
  return "command";
}

export function stepLabel(item: StepItem): string {
  if (item.type === "fileChange") return item.changes.map((change) => change.path).join(", ");
  if (item.type === "mcpToolCall") return `${item.server}.${item.tool}`;
  const action = item.commandActions?.[0];
  const kind = stepKind(item);
  if (kind === "read" && action?.type === "read") return action.name;
  if (kind === "search" && action?.type === "search") return [action.query, action.path].filter(Boolean).join(" · ") || item.command;
  return item.command;
}

export function stepStatus(item: StepItem): StepStatus {
  if (item.status === "inProgress") return "running";
  return item.status === "completed" ? "ok" : "failed";
}

export function activitySummary(steps: readonly StepItem[]): string {
  const count = (kind: StepKind) => steps.filter((step) => stepKind(step) === kind).length;
  const parts = [
    count("read") > 0 ? `파일 ${count("read")}개 읽음` : null,
    count("search") > 0 ? `검색 ${count("search")}회` : null,
    count("edit") > 0 ? `파일 변경 ${count("edit")}회` : null,
    count("command") > 0 ? `명령 ${count("command")}개 실행` : null,
    count("tool") > 0 ? `도구 ${count("tool")}회` : null,
  ];
  return parts.filter(Boolean).join(" · ");
}

/** 걸린 시간을 짧게 쓴다: `0.4초`, `12초`, `2분 14초`. */
export function formatDuration(ms: number): string {
  if (ms < 10_000) return `${(ms / 1000).toFixed(1)}초`;
  const seconds = Math.round(ms / 1000);
  return seconds >= 60 ? `${Math.floor(seconds / 60)}분 ${seconds % 60}초` : `${seconds}초`;
}
