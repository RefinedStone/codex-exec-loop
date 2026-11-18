// 운영체제 알림이다 (D-19, plan 26 A1). 대화의 글은 넣지 않는다: 알림은 잠금 화면과 알림 기록에 남는다.
// 알림 권한이 없거나 알림을 쓸 수 없으면 아무것도 하지 않는다. 사이드바의 표시는 그대로 보인다.

/**
 * `onOpen` 은 사용자가 알림을 눌렀을 때 부른다. `tag` 가 같은 알림은 하나만 남는다: 같은 스레드를 여러 창이
 * 보고 있어도 알림이 겹치지 않는다.
 */
export function notifyOs(title: string, body: string, onOpen: () => void, tag?: string): void {
  if (typeof Notification === "undefined" || Notification.permission === "denied") return;
  try {
    const notification = new Notification(title, { body, silent: false, ...(tag ? { tag } : {}) });
    notification.onclick = () => {
      window.focus();
      onOpen();
      notification.close();
    };
  } catch {
    // 알림을 만들지 못했다. 사이드바의 표시가 남는다.
  }
}

/** dot 이 답했다는 알림이다. */
export function notifyDotReply(dotName: string, onOpen: () => void): void {
  notifyOs(dotName, "새 답이 왔습니다.", onOpen);
}
