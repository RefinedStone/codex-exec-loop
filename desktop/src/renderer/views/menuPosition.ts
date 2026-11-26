// 클릭한 곳에 여는 `position: fixed` 메뉴를 창의 표시 영역 안에 둔다. 오른쪽·아래 끝에서는 클릭한 곳의 반대쪽으로 연다.
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

/** 메뉴의 왼쪽 위 점이다. 넘치는 축은 반대쪽으로 열고, 그래도 넘치면 창 끝에 붙인다. */
export function fitMenuPosition(point: MenuPoint, menu: MenuSize, viewport: MenuSize): MenuPoint {
  return { x: fitAxis(point.x, menu.width, viewport.width), y: fitAxis(point.y, menu.height, viewport.height) };
}

/** 그린 메뉴의 크기를 재고 그리기 전에 위치를 고친다. */
export function useFittedMenuPosition(menu: RefObject<HTMLElement | null>, point: MenuPoint): MenuPoint {
  const [position, setPosition] = useState(point);
  useLayoutEffect(() => {
    const element = menu.current;
    if (!element) return;
    const { width, height } = element.getBoundingClientRect();
    setPosition(fitMenuPosition(point, { width, height }, { width: window.innerWidth, height: window.innerHeight }));
  }, [menu, point.x, point.y]);
  return position;
}
