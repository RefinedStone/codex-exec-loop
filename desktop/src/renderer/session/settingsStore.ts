// 앱 설정의 renderer 쪽 사본이다 (plan 26 U2). 원본은 main 이 가지고 저장한다. 여기는 화면이 읽을 값을 둔다.
import { create } from "zustand";

import { type AppSettings, type AppSettingsPatch, DEFAULT_SETTINGS, mergeSettings } from "../../shared/settings";

export const useSettings = create<AppSettings>(() => DEFAULT_SETTINGS);

/**
 * 설정의 한 부분을 바꾼다. 화면에 먼저 반영하고 바꾼 부분만 main 에 보낸다. main 이 합치고 검증한 값은 `onChanged`
 * 로 돌아온다. 요청의 답으로는 저장소를 덮어쓰지 않는다: 빠르게 이어 누른 값이 잠깐 되돌아가지 않는다.
 */
export function updateSettings(patch: AppSettingsPatch): void {
  useSettings.setState((current) => mergeSettings(current, patch));
  window.akra.settings.set(patch).catch(() => undefined);
}
