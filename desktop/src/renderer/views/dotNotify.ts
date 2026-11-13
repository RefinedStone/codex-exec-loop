// dot 이 답했다는 운영체제 알림이다 (D-19). 답의 글은 넣지 않는다: 알림은 잠금 화면과 알림 기록에 남는다.
// 알림 권한이 없거나 알림을 쓸 수 없으면 아무것도 하지 않는다. 사이드바의 "새 답" 표시는 그대로 보인다.

/** `onOpen` 은 사용자가 알림을 눌렀을 때 부른다. */
export function notifyDotReply(dotName: string, onOpen: () => void): void {
  if (typeof Notification === "undefined" || Notification.permission === "denied") return;
  try {
    const notification = new Notification(dotName, { body: "새 답이 왔습니다.", silent: false });
    notification.onclick = () => {
      window.focus();
      onOpen();
      notification.close();
    };
  } catch {
    // 알림을 만들지 못했다. 사이드바의 표시가 남는다.
  }
}
