// 앱 전체 확대 값이다 (plan 26 U-3). 본 창과 dot 창이 같은 값을 쓴다. main 이 저장하고 모든 창에 적용한다.
export const ZOOM_STEPS = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2] as const;

export type ZoomDirection = "in" | "out" | "reset";

export function isZoomDirection(value: unknown): value is ZoomDirection {
  return value === "in" || value === "out" || value === "reset";
}

/** 저장된 값이나 renderer 가 보낸 값을 쓸 수 있는 배율로 바꾼다. 숫자가 아니면 1 이다. */
export function validZoom(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return 1;
  return Math.min(ZOOM_STEPS[ZOOM_STEPS.length - 1] ?? 1, Math.max(ZOOM_STEPS[0] ?? 1, value));
}

/** 다음 단계의 배율이다. 끝 단계에서는 그대로이다. */
export function nextZoom(current: number, direction: ZoomDirection): number {
  if (direction === "reset") return 1;
  // 부동 소수 오차 때문에 같은 단계를 한 번 더 고르지 않게 한다.
  const epsilon = 0.001;
  if (direction === "in") return ZOOM_STEPS.find((step) => step > current + epsilon) ?? current;
  return ZOOM_STEPS.findLast((step) => step < current - epsilon) ?? current;
}
