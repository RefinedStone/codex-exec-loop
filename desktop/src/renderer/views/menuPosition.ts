// 클릭한 곳에 여는 `position: fixed` 메뉴를 창의 표시 영역 안에 둔다. 오른쪽·아래 끝에서는 클릭한 곳의 반대쪽으로 연다.
// 메뉴가 CSS `zoom` 영역(`zoomStyle`) 안에 있으면 `left`·`top` 도 그 배율로 곱해지므로, 화면 좌표를 배율로 나눈다.
import { type RefObject, useLayoutEffect, useState } from "react";

export interface MenuPoint {
  x: number;
  y: number;
}

export interface MenuSize {
  width: number;
  height: number;
}

function fitAxis(start: number, size: number, limit: number): number {
  const flipped = start + size > limit ? start - size : start;
  return Math.max(0, Math.min(flipped, limit - size));
}

/**
 * 메뉴의 `left`·`top` 이다. 넘치는 축은 반대쪽으로 열고, 그래도 넘치면 창 끝에 붙인다. `point`·`menu`·`viewport` 는 화면
 * 좌표이고, 결과는 메뉴에 걸린 배율 `zoom` 의 좌표이다.
 */
export function fitMenuPosition(point: MenuPoint, menu: MenuSize, viewport: MenuSize, zoom = 1): MenuPoint {
  return { x: fitAxis(point.x, menu.width, viewport.width) / zoom, y: fitAxis(point.y, menu.height, viewport.height) / zoom };
}

/** 그린 메뉴의 크기를 재고 그리기 전에 위치를 고친다. `point` 는 `clientX`·`clientY` 이다. */
export function useFittedMenuPosition(menu: RefObject<HTMLElement | null>, point: MenuPoint): MenuPoint {
  const [position, setPosition] = useState(point);
  useLayoutEffect(() => {
    const element = menu.current;
    if (!element) return;
    // getBoundingClientRect 는 배율을 곱한 화면 크기이다.
    const { width, height } = element.getBoundingClientRect();
    const viewport = { width: window.innerWidth, height: window.innerHeight };
    setPosition(fitMenuPosition(point, { width, height }, viewport, element.currentCSSZoom || 1));
  }, [menu, point.x, point.y]);
  return position;
}
