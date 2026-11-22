// 운영체제 알림이다 (D-19, plan 26 A1). 대화의 글은 넣지 않는다: 알림은 잠금 화면과 알림 기록에 남는다.
// 알림 설정(plan 26 A2)이 끄기이면 보이지 않는다. "창이 뒤에 있을 때만"이면 스레드의 알림은 앱의 창이 모두 뒤에
// 있을 때만 보인다.
// 알림 권한이 없거나 알림을 쓸 수 없으면 아무것도 하지 않는다. 사이드바의 표시는 그대로 보인다.
import { shouldNotify } from "../../shared/settings";
import { useSettings } from "../session/settingsStore";

function show(title: string, body: string, onOpen: () => void, sound: boolean, tag?: string): void {
  try {
    const notification = new Notification(title, { body, silent: !sound, ...(tag ? { tag } : {}) });
    notification.onclick = () => {
      window.focus();
      onOpen();
      notification.close();
    };
  } catch {
    // 알림을 만들지 못했다. 사이드바의 표시가 남는다.
  }
}

function available(): boolean {
  return typeof Notification !== "undefined" && Notification.permission !== "denied";
}

/**
 * 스레드의 알림이다. `onOpen` 은 사용자가 알림을 눌렀을 때 부른다. `tag` 가 같은 알림은 하나만 남는다: 같은 스레드를
 * 여러 창이 보고 있어도 알림이 겹치지 않는다. 포커스는 main 에 묻는다: 이 앱의 다른 창이나 브라우저 패널을 쓰는
 * 중이면 "뒤에 있다"가 아니다.
 */
export function notifyOs(title: string, body: string, onOpen: () => void, tag?: string): void {
  const { mode, sound } = useSettings.getState().notifications;
  if (mode === "off" || !available()) return;
  const gated = (focused: boolean) => {
    if (shouldNotify(mode, focused)) show(title, body, onOpen, sound, tag);
  };
  window.akra.isAppFocused().then(gated, () => gated(document.hasFocus()));
}

/**
 * dot 이 답했다는 알림이다. 언제 알릴지는 부르는 쪽이 정한다 (그 대화를 보고 있지 않을 때). 알림 설정이 끄기일
 * 때만 막는다.
 */
export function notifyDotReply(dotName: string, onOpen: () => void): void {
  const { mode, sound } = useSettings.getState().notifications;
  if (mode !== "off" && available()) show(dotName, "새 답이 왔습니다.", onOpen, sound);
}

/** 이슈 결정 알림에는 저장소명·질문·권고·본문·메모를 넣지 않고, 앱이 뒤에 있을 때만 보여 준다. */
export function notifyIssueDecision(onOpen: () => void, tag: string): void {
  const { mode, sound } = useSettings.getState().notifications;
  if (mode === "off" || !available()) return;
  const gated = (focused: boolean) => {
    if (!focused && useSettings.getState().notifications.mode !== "off") show("Akra", "이슈에 새 결정 요청이 있습니다. 이슈 탭에서 확인해 주세요.", onOpen, sound, tag);
  };
  window.akra.isAppFocused().then(gated, () => gated(document.hasFocus()));
}
