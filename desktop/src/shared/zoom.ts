// 영역별 확대 값이다 (#2626, plan 26 U-3 을 바꾼다). 창의 페이지 배율은 늘 100% 이다. renderer 가 영역마다 CSS `zoom` 을
// 건다. main 이 값을 저장하고 모든 창에 알린다.
export const ZOOM_STEPS = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2] as const;

export type ZoomDirection = "in" | "out" | "reset";

/** 본문 보기이다. 스레드마다 따로 두지 않는다: 모든 스레드가 `thread` 값을 쓴다. */
export const BODY_ZOOM_SCOPES = ["home", "fleet", "thread", "remote"] as const;
/** 사이드바와 위 막대는 단축키나 휠로 바뀌지 않는다. 설정 창에서만 바꾼다. */
export const SHELL_ZOOM_SCOPES = ["sidebar", "topbar"] as const;
/** 터미널은 xterm 글자 크기에 곱한다. `browser-page` 는 브라우저 패널 안의 웹 페이지 자체이다. */
export const OTHER_ZOOM_SCOPES = ["terminal", "dot", "dot-window", "chat", "browser-page"] as const;

type FixedZoomScope = (typeof BODY_ZOOM_SCOPES)[number] | (typeof SHELL_ZOOM_SCOPES)[number] | (typeof OTHER_ZOOM_SCOPES)[number];
/** 오른쪽 패널은 탭(패널 id)마다 값을 둔다. 확장 패널도 같다. */
export type ZoomScope = FixedZoomScope | `panel:${string}`;
/** 값이 없는 영역은 100% 이다. */
export type ZoomFactors = Partial<Record<ZoomScope, number>>;

const FIXED_SCOPES: readonly string[] = [...BODY_ZOOM_SCOPES, ...SHELL_ZOOM_SCOPES, ...OTHER_ZOOM_SCOPES];
const PANEL_SCOPE = /^panel:[a-z0-9][a-z0-9._-]{0,63}$/;

export function isZoomDirection(value: unknown): value is ZoomDirection {
  return value === "in" || value === "out" || value === "reset";
}

export function isZoomScope(value: unknown): value is ZoomScope {
  return typeof value === "string" && (FIXED_SCOPES.includes(value) || PANEL_SCOPE.test(value));
}

export function panelZoomScope(panelId: string): ZoomScope {
  return `panel:${panelId}`;
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

/**
 * 저장 파일의 값을 읽는다. 모르는 영역, 100%, 숫자가 아닌 값은 버린다. `zooms` 가 없고 옛 `zoom` 이 있으면 옛 값을
 * 본문 보기에만 한 번 옮긴다: 사이드바와 위 막대는 100% 로 돌아간다.
 */
export function parseZoomFactors(zooms: unknown, legacyZoom: unknown): ZoomFactors {
  const factors: ZoomFactors = {};
  if (zooms && typeof zooms === "object" && !Array.isArray(zooms)) {
    for (const [scope, value] of Object.entries(zooms)) {
      const factor = validZoom(value);
      if (isZoomScope(scope) && typeof value === "number" && factor !== 1) factors[scope] = factor;
    }
    return factors;
  }
  const legacy = validZoom(legacyZoom);
  if (legacy !== 1) for (const scope of BODY_ZOOM_SCOPES) factors[scope] = legacy;
  return factors;
}

/** 한 영역을 한 단계 바꾼 새 값이다. 다른 영역은 그대로이다. */
export function stepZoomFactors(factors: ZoomFactors, scope: ZoomScope, direction: ZoomDirection): ZoomFactors {
  const next = { ...factors };
  const factor = nextZoom(factors[scope] ?? 1, direction);
  if (factor === 1) delete next[scope];
  else next[scope] = factor;
  return next;
}

/** 배율 알림과 설정 창에 쓰는 영역 이름이다. 패널은 패널 이름을 받아 붙인다. */
export function zoomScopeLabel(scope: ZoomScope, panelLabel?: string): string {
  if (scope.startsWith("panel:")) return panelLabel ?? scope.slice("panel:".length);
  const labels: Record<FixedZoomScope, string> = {
    home: "홈", fleet: "Fleet", thread: "본문", remote: "클라우드 본문", sidebar: "사이드바", topbar: "위 막대",
    terminal: "터미널", dot: "dot 옆 칸", "dot-window": "dot 창", chat: "ChatGPT 옆 칸", "browser-page": "웹 페이지",
  };
  return labels[scope as FixedZoomScope];
}
