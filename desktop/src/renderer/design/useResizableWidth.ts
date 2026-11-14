// 왼쪽 가장자리의 손잡이를 끌어 폭을 바꾸는 오른쪽 칸의 폭이다. 오른쪽 패널과 dot 옆 칸이 쓴다.
import { type PointerEvent as ReactPointerEvent, useState } from "react";

export function useResizableWidth(initial: number, min: number, max: number): [number, (event: ReactPointerEvent<HTMLDivElement>) => void] {
  const [width, setWidth] = useState(initial);
  const startResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    // 포인터를 손잡이에 잡아 둔다. 브라우저 페이지 위로 끌어도 이동과 놓기 이벤트를 여기서 받는다.
    const handle = event.currentTarget;
    handle.setPointerCapture(event.pointerId);
    const startX = event.clientX;
    const startWidth = width;
    const onMove = (move: PointerEvent) => setWidth(Math.min(max, Math.max(min, startWidth + startX - move.clientX)));
    const onUp = () => {
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onUp);
      handle.removeEventListener("pointercancel", onUp);
    };
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onUp);
    handle.addEventListener("pointercancel", onUp);
  };
  return [width, startResize];
}
