// 영역별 확대를 창에 연결한다 (#2626). 값은 main 이 가진다. 여기는 값을 `uiStore` 에 두고, 메뉴·단축키·휠이 바꿀 영역을
// 고른다. 영역은 `data-zoom-scope` 를 단 요소이고, 그 요소가 CSS `zoom` 을 건다 (`zoomStyle`).
import { useEffect } from "react";

import { panels } from "../extensions/panelDefinitions";
import { showZoomNotice, useUiStore } from "../session/uiStore";
import { isZoomScope, zoomScopeLabel, type ZoomScope } from "../../shared/zoom";

/** 휠이나 핀치가 이만큼 움직이면 한 단계 바꾼다. 마우스 휠의 한 칸은 100 쯤이다. */
const WHEEL_ZOOM_STEP_PX = 60;

/** 사이드바와 위 막대는 단축키나 휠로 바꾸지 않는다. 설정 창에서만 바꾼다. */
const isShellScope = (scope: ZoomScope) => scope === "sidebar" || scope === "topbar";

/** 요소가 속한 영역이다. 영역 밖이면 null 이다. */
export function zoomScopeOf(target: EventTarget | null): ZoomScope | null {
  const scope = target instanceof Element ? target.closest<HTMLElement>("[data-zoom-scope]")?.dataset.zoomScope : undefined;
  return isZoomScope(scope) ? scope : null;
}

export function zoomTargetLabel(scope: ZoomScope): string {
  return zoomScopeLabel(scope, panels.find((panel) => `panel:${panel.id}` === scope)?.label);
}

/**
 * 영역 요소에 거는 style 이다. 100% 이면 아무것도 걸지 않는다. CSS zoom 은 vh 에도 곱하므로, 영역 안의 CSS 는
 * `calc(55vh / var(--area-zoom, 1))` 처럼 이 값으로 나눈다.
 */
export function zoomStyle(factor: number): React.CSSProperties | undefined {
  return factor === 1 ? undefined : ({ zoom: factor, "--area-zoom": factor } as React.CSSProperties);
}

export function useZoomFactor(scope: ZoomScope): number {
  return useUiStore((state) => state.zooms[scope] ?? 1);
}

/** `fallback` 은 포커스가 영역 밖이거나 사이드바·위 막대일 때 단축키가 바꿀 영역이다 (본 창은 지금 본문 보기). */
export function useZoom(fallback: () => ZoomScope): void {
  useEffect(() => {
    window.akra.zoom.get().then((zooms) => useUiStore.setState({ zooms }), () => undefined);
    const stopChanged = window.akra.zoom.onChanged((zooms, scope, here) => {
      useUiStore.setState({ zooms });
      // 바꾼 창에만 알린다. 다른 창은 값만 따른다.
      if (here) showZoomNotice(`${zoomTargetLabel(scope)} ${Math.round((zooms[scope] ?? 1) * 100)}%`);
    });
    const stopCommand = window.akra.zoom.onCommand((direction) => {
      const focused = zoomScopeOf(document.activeElement);
      window.akra.zoom.step(focused && !isShellScope(focused) ? focused : fallback(), direction);
    });
    // ⌘(Ctrl) + 휠이다. 트랙패드 핀치도 `ctrlKey` 가 켜진 휠 이벤트로 온다. 포인터 아래 영역을 바꾼다.
    let travelled = 0;
    const onWheel = (event: WheelEvent) => {
      if (!event.ctrlKey && !event.metaKey) return;
      event.preventDefault();
      const scope = zoomScopeOf(event.target);
      if (!scope || isShellScope(scope)) return;
      travelled += event.deltaY;
      if (Math.abs(travelled) < WHEEL_ZOOM_STEP_PX) return;
      window.akra.zoom.step(scope, travelled < 0 ? "in" : "out");
      travelled = 0;
    };
    window.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      stopChanged();
      stopCommand();
      window.removeEventListener("wheel", onWheel);
    };
  }, [fallback]);
}
