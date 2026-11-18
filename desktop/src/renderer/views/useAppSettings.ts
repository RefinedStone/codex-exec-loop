// 앱 설정을 창에 연결한다 (plan 26 U2): main 에서 읽고, 바뀌면 따라가고, 글자 크기를 CSS 토큰에 쓴다.
import { useEffect } from "react";

import { useSettings } from "../session/settingsStore";
import { fontTokens } from "../session/settingsView";

export function useAppSettings(): void {
  useEffect(() => {
    window.akra.settings.get().then(
      (settings) => useSettings.setState(settings),
      () => undefined,
    );
    return window.akra.settings.onChanged((settings) => useSettings.setState(settings));
  }, []);
  const fonts = useSettings((state) => state.fonts);
  useEffect(() => {
    for (const [token, value] of Object.entries(fontTokens(fonts))) {
      document.documentElement.style.setProperty(token, value);
    }
  }, [fonts]);
}
