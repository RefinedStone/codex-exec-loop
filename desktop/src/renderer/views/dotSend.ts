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
