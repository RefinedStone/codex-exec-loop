// 앱 설정의 renderer 쪽 사본이다 (plan 26 U2). 원본은 main 이 가지고 저장한다. 여기는 화면이 읽을 값을 둔다.
import { create } from "zustand";

import { type AppSettings, DEFAULT_SETTINGS } from "../../shared/settings";

export const useSettings = create<AppSettings>(() => DEFAULT_SETTINGS);

/** 설정의 한 부분을 바꾼다. 화면에 먼저 반영하고 main 에 보낸다. main 이 검증한 값이 `onChanged` 로 돌아온다. */
export function updateSettings(patch: Partial<AppSettings>): void {
  const next = { ...useSettings.getState(), ...patch };
  useSettings.setState(next);
  window.akra.settings.set(next).then(
    (saved) => useSettings.setState(saved),
    () => undefined,
  );
}
