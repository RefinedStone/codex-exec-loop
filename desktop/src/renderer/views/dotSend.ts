// dot 대화의 화면 밖 논리이다: 보내기 허락("세션 한 번 허용", D-19), 시각과 상태 표시, 읽음. React 에 의존하지 않는다.

export const DOT_SEND_WARNING =
  "이 에이전트에게 메시지를 보낼 수 있게 합니다. 보낸 메시지는 실계정의 대화에 남고 지울 수 없습니다. 에이전트가 답하면서 모델을 씁니다. 허용합니까?";

/** 이 앱을 켠 동안 사용자가 보내기를 허락한 dot 이다 ("세션 한 번 허용", D-19). 앱을 끄면 사라진다. */
const sendAllowedDots = new Set<string>();

/** 시험이 허락 상태를 지운다. */
export function resetDotSendPermissions(): void {
  sendAllowedDots.clear();
}

/** 보내기를 허락한다. 처음에는 경고를 보이고 사용자가 확인해야 허락한다. */
export function allowDotSending(dotId: string, confirm: (message: string) => boolean): boolean {
  if (sendAllowedDots.has(dotId)) return true;
  if (!confirm(DOT_SEND_WARNING)) return false;
  sendAllowedDots.add(dotId);
  return true;
}

export function isDotSendingAllowed(dotId: string): boolean {
  return sendAllowedDots.has(dotId);
}

/** 서비스의 시각 글자를 이 컴퓨터의 날짜와 시각으로 보인다. 읽을 수 없으면 그대로 보인다. */
export function messageTime(createdAt: string): string {
  const time = new Date(createdAt);
  return Number.isNaN(time.getTime()) ? createdAt : time.toLocaleString();
}

/** 보낸 뒤 dot 의 답을 기다리며 다시 읽는 간격이다. */
export const REPLY_POLL_MS = 2_000;
/** 답을 기다리는 가장 긴 시간이다. 지나면 멈추고, 사용자가 "새로 고침"으로 읽는다. 2026-11-14 실측한 답은 3~9초였다. */
export const REPLY_WAIT_LIMIT_MS = 45_000;

/**
 * 보낸 뒤에 dot 이 답했는지이다. 가장 새 메시지가 dot 의 것이고, 보내기 직전의 가장 새 메시지와 다르면 답이다.
 * 보낸 직후의 읽기는 내 글이 아직 없을 수 있다. 그때 가장 새 메시지는 보내기 전의 것이므로 답으로 보지 않는다.
 */
export function hasDotRepliedSince(messages: readonly { id: string; fromDot: boolean }[], newestIdBeforeSend: string | null): boolean {
  const newest = messages.at(-1);
  return newest !== undefined && newest.fromDot && newest.id !== newestIdBeforeSend;
}

/** 말풍선 아래의 시각이다 (시:분). 읽을 수 없으면 빈 글자이다. */
export function messageClock(createdAt: string): string {
  const time = new Date(createdAt);
  return Number.isNaN(time.getTime()) ? "" : time.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** 날짜 구분선의 글자이다. 같은 날의 메시지는 같은 글자가 된다. 읽을 수 없으면 빈 글자이다. */
export function dayLabel(createdAt: string, now: Date = new Date()): string {
  const time = new Date(createdAt);
  if (Number.isNaN(time.getTime())) return "";
  const startOfDay = (value: Date) => new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
  const days = Math.round((startOfDay(now) - startOfDay(time)) / 86_400_000);
  if (days === 0) return "오늘";
  if (days === 1) return "어제";
  return time.toLocaleDateString([], { year: "numeric", month: "long", day: "numeric" });
}

/**
 * dot 이 읽은 내 마지막 메시지의 id 이다. 그 메시지 아래에 "읽음"을 보인다. dot 이 읽은 시각(`dotReadAt`) 이전에
 * 보낸 내 메시지 가운데 가장 새 것이다. 읽은 시각을 모르거나 그런 메시지가 없으면 null 이다.
 */
export function lastReadMessageId(messages: readonly { id: string; fromDot: boolean; createdAt: string }[], dotReadAt: string | null): string | null {
  const readAt = dotReadAt === null ? Number.NaN : Date.parse(dotReadAt);
  if (Number.isNaN(readAt)) return null;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index]!;
    if (!message.fromDot && Date.parse(message.createdAt) <= readAt) return message.id;
  }
  return null;
}

export interface DotStatusLine {
  text: string;
  /** 표시의 색이다: 일하는 중, 평소, 주의. */
  tone: "working" | "idle" | "muted";
}

/** 머리줄의 한 줄 상태이다. 실시간 상태가 있으면 그것이 먼저이고, 없으면 dot 목록의 상태를 쓴다. */
export function dotStatusLine(
  dot: { status: string; paused: boolean },
  live: { connected: boolean; working: boolean; activityLabel: string | null; usingComputer: boolean } | undefined,
): DotStatusLine {
  if (dot.paused) return { text: "멈춤", tone: "muted" };
  if (live?.connected && live.working) {
    const label = live.activityLabel?.trim();
    return { text: label ? `${label}…` : live.usingComputer ? "컴퓨터로 작업하는 중…" : "답을 만드는 중…", tone: "working" };
  }
  if (live?.connected) return { text: "대기 중", tone: "idle" };
  // 실시간 통로가 없다. 대화는 읽을 수 있지만 답이 저절로 뜨지 않는다.
  return { text: "실시간 연결 끊김 · 새로 고침으로 확인하세요", tone: "muted" };
}

/**
 * 저장된 초안이 방금 보낸 글과 같은지이다. 같을 때만 전송 뒤에 초안을 지운다. 전송 답을 기다리는 사이 사용자가 쓴
 * 다른 글은 보낸 글이 아니므로 남긴다.
 */
export function isSentDraft(storedDraft: string | undefined, sentText: string): boolean {
  return (storedDraft ?? "").trim() === sentText;
}
