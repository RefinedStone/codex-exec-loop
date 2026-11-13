// dot 대화의 화면 밖 논리이다: 보내기 허락("세션 한 번 허용", D-19)과 시각 표시. React 에 의존하지 않는다.

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
