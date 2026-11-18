// 앱 전체 확대를 창에 연결한다 (plan 26 Z1–Z3). 값은 main 이 가진다. 여기는 배율을 CSS 에 알리고, 휠과 핀치를
// main 에 전한다. 메뉴와 단축키(⌘+ / ⌘− / ⌘0)는 main 이 받는다.
import { useEffect } from "react";

import { showZoomNotice } from "../session/uiStore";

/** 휠이나 핀치가 이만큼 움직이면 한 단계 바꾼다. 마우스 휠의 한 칸은 100 쯤이다. */
const WHEEL_ZOOM_STEP_PX = 60;

export function useZoom(): void {
  useEffect(() => {
    // macOS 창 단추의 자리는 확대해도 그대로이다. CSS 가 이 값으로 그 자리를 비운다.
    const apply = (factor: number) => document.documentElement.style.setProperty("--zoom", String(factor));
    window.akra.zoom.get().then(apply, () => undefined);
    const stop = window.akra.zoom.onChanged((factor) => {
      apply(factor);
      showZoomNotice(factor);
    });
    // ⌘(Ctrl) + 휠이다. 트랙패드 핀치도 `ctrlKey` 가 켜진 휠 이벤트로 온다.
    let travelled = 0;
    const onWheel = (event: WheelEvent) => {
      if (!event.ctrlKey && !event.metaKey) return;
      event.preventDefault();
      travelled += event.deltaY;
      if (Math.abs(travelled) < WHEEL_ZOOM_STEP_PX) return;
      window.akra.zoom.step(travelled < 0 ? "in" : "out");
      travelled = 0;
    };
    window.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      stop();
      window.removeEventListener("wheel", onWheel);
    };
  }, []);
}
