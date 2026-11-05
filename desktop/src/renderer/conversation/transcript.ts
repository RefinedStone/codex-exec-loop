// 열린 스레드의 화면용 사본이다. 원본은 app-server 에 있다 (D-05).
// `thread/resume` 결과로 시작하고, 그 스레드의 `turn/*`, `item/*` 알림을 순서대로 반영한다. 모두 순수 함수이다.
import type { ThreadItem } from "../../shared/protocol/generated/v2/ThreadItem";
import type { ThreadResumeResponse } from "../../shared/protocol/generated/v2/ThreadResumeResponse";
import type { ThreadStatus } from "../../shared/protocol/generated/v2/ThreadStatus";
import type { Turn } from "../../shared/protocol/generated/v2/Turn";

export interface TurnView {
  id: string;
  status: Turn["status"];
  errorMessage: string | null;
  items: ThreadItem[];
}

export interface Transcript {
  threadId: string;
  title: string;
  cwd: string;
  status: ThreadStatus | null;
  turns: TurnView[];
}

interface Notification {
  method: string;
  params: unknown;
}

const DELTA_METHODS = new Set([
  "item/agentMessage/delta",
  "item/commandExecution/outputDelta",
  "item/plan/delta",
  "item/reasoning/summaryTextDelta",
]);

/** 이 스레드에 대한 알림인지 확인한다. threadId 가 없는 알림은 대화 상태와 관계없다. */
export function notificationThreadId(params: unknown): string | null {
  const threadId = (params as { threadId?: unknown } | null)?.threadId;
  return typeof threadId === "string" ? threadId : null;
}

export function transcriptFromResume(response: ThreadResumeResponse): Transcript {
  const { thread } = response;
  return {
    threadId: thread.id,
    title: thread.name ?? (thread.preview.trim() || "(제목 없음)"),
    cwd: thread.cwd,
    status: thread.status,
    turns: thread.turns.map(turnView),
  };
}

function turnView(turn: Turn): TurnView {
  return { id: turn.id, status: turn.status, errorMessage: turn.error?.message ?? null, items: [...turn.items] };
}

export function isTurnInProgress(transcript: Transcript | null): boolean {
  return transcript?.turns.at(-1)?.status === "inProgress";
}

/** 알림 여러 개를 순서대로 반영한다. 화면 프레임마다 한 번 부른다. */
export function applyNotifications(transcript: Transcript, notifications: readonly Notification[]): Transcript {
  return notifications.reduce(applyNotification, transcript);
}

export function applyNotification(transcript: Transcript, notification: Notification): Transcript {
  const params = notification.params as Record<string, unknown>;
  if (notificationThreadId(params) !== transcript.threadId) {
    return transcript;
  }
  switch (notification.method) {
    case "thread/status/changed":
      return { ...transcript, status: params.status as ThreadStatus };
    case "turn/started":
    case "turn/completed":
      return upsertTurn(transcript, params.turn as Turn);
    case "item/started":
    case "item/completed":
      return upsertItem(transcript, params.turnId as string, params.item as ThreadItem);
    default:
      if (DELTA_METHODS.has(notification.method)) {
        return appendDelta(transcript, notification.method, params);
      }
      return transcript;
  }
}

function upsertTurn(transcript: Transcript, turn: Turn): Transcript {
  const index = transcript.turns.findIndex((existing) => existing.id === turn.id);
  if (index < 0) {
    return { ...transcript, turns: [...transcript.turns, turnView(turn)] };
  }
  const existing = transcript.turns[index]!;
  // turn/completed 의 items 는 비어 있을 수 있다. 그때는 알림으로 모은 아이템을 유지한다.
  const items = turn.items.length > 0 ? mergeItems(existing.items, turn.items) : existing.items;
  const updated: TurnView = { ...existing, status: turn.status, errorMessage: turn.error?.message ?? null, items };
  return { ...transcript, turns: replaceAt(transcript.turns, index, updated) };
}

function mergeItems(current: ThreadItem[], incoming: ThreadItem[]): ThreadItem[] {
  const merged = [...current];
  for (const item of incoming) {
    const index = merged.findIndex((existing) => existing.id === item.id);
    if (index < 0) {
      merged.push(item);
    } else {
      merged[index] = item;
    }
  }
  return merged;
}

function upsertItem(transcript: Transcript, turnId: string, item: ThreadItem): Transcript {
  return updateTurn(transcript, turnId, (turn) => ({ ...turn, items: mergeItems(turn.items, [item]) }));
}

function appendDelta(transcript: Transcript, method: string, params: Record<string, unknown>): Transcript {
  const turnId = params.turnId as string;
  const itemId = params.itemId as string;
  const delta = String(params.delta ?? "");
  return updateTurn(transcript, turnId, (turn) => {
    const index = turn.items.findIndex((item) => item.id === itemId);
    if (index < 0) {
      return turn;
    }
    const item = turn.items[index]!;
    return { ...turn, items: replaceAt(turn.items, index, withDelta(item, method, delta, params)) };
  });
}

function withDelta(item: ThreadItem, method: string, delta: string, params: Record<string, unknown>): ThreadItem {
  if (method === "item/agentMessage/delta" && item.type === "agentMessage") {
    return { ...item, text: item.text + delta };
  }
  if (method === "item/commandExecution/outputDelta" && item.type === "commandExecution") {
    return { ...item, aggregatedOutput: (item.aggregatedOutput ?? "") + delta };
  }
  if (method === "item/plan/delta" && item.type === "plan") {
    return { ...item, text: item.text + delta };
  }
  if (method === "item/reasoning/summaryTextDelta" && item.type === "reasoning") {
    const index = typeof params.summaryIndex === "number" ? params.summaryIndex : item.summary.length - 1;
    const summary = [...item.summary];
    while (summary.length <= index) {
      summary.push("");
    }
    summary[index] = (summary[index] ?? "") + delta;
    return { ...item, summary };
  }
  return item;
}

function updateTurn(transcript: Transcript, turnId: string, update: (turn: TurnView) => TurnView): Transcript {
  const index = transcript.turns.findIndex((turn) => turn.id === turnId);
  if (index < 0) {
    return transcript;
  }
  return { ...transcript, turns: replaceAt(transcript.turns, index, update(transcript.turns[index]!)) };
}

function replaceAt<T>(values: readonly T[], index: number, value: T): T[] {
  const copy = [...values];
  copy[index] = value;
  return copy;
}
