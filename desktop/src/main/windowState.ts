// 창의 크기, 위치, 최대화와 앱 확대 값을 userData 의 JSON 파일에 둔다 (plan 26 W1, U-3).
// 파일은 사용자가 고칠 수 있으므로 읽을 때마다 검증한다.
import { readFileSync, renameSync, writeFileSync } from "node:fs";

import { validZoom } from "../shared/zoom";

export interface WindowBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface WindowState {
  zoom: number;
  /** 최대화하기 전의 크기와 위치이다. 저장한 적이 없으면 null 이다. */
  bounds: WindowBounds | null;
  maximized: boolean;
}

export const DEFAULT_WINDOW_STATE: WindowState = { zoom: 1, bounds: null, maximized: false };

const MIN_WIDTH = 480;
const MIN_HEIGHT = 320;
/** 창의 이만큼이 화면 안에 있어야 사용자가 창을 잡아 옮길 수 있다. */
const MIN_VISIBLE_PX = 80;

function parseBounds(input: unknown): WindowBounds | null {
  const raw = (input ?? {}) as Record<string, unknown>;
  const values = [raw.x, raw.y, raw.width, raw.height];
  if (!values.every((value): value is number => typeof value === "number" && Number.isFinite(value))) return null;
  const [x, y, width, height] = values.map(Math.round) as [number, number, number, number];
  return width >= MIN_WIDTH && height >= MIN_HEIGHT ? { x, y, width, height } : null;
}

export function parseWindowState(input: unknown): WindowState {
  const raw = (input ?? {}) as Record<string, unknown>;
  return { zoom: validZoom(raw.zoom), bounds: parseBounds(raw.bounds), maximized: raw.maximized === true };
}

/**
 * 저장된 자리가 지금 있는 화면에 보이면 그 자리를 돌려준다. 화면을 뺐거나 해상도가 바뀌어서 창이 화면 밖이면 null 이다:
 * 그때는 기본 자리에 연다. `workAreas` 는 화면마다의 작업 영역이다.
 */
export function visibleBounds(bounds: WindowBounds | null, workAreas: readonly WindowBounds[]): WindowBounds | null {
  if (!bounds) return null;
  const visible = workAreas.some((area) => {
    const width = Math.min(bounds.x + bounds.width, area.x + area.width) - Math.max(bounds.x, area.x);
    // 제목 막대(창의 위쪽)가 화면 안에 있어야 한다.
    const top = bounds.y >= area.y && bounds.y <= area.y + area.height - MIN_VISIBLE_PX;
    return width >= MIN_VISIBLE_PX && top;
  });
  return visible ? bounds : null;
}

export function loadWindowState(path: string): WindowState {
  try {
    return parseWindowState(JSON.parse(readFileSync(path, "utf8")));
  } catch {
    return { ...DEFAULT_WINDOW_STATE };
  }
}

/** 임시 파일에 쓰고 바꿔 넣는다. 쓰지 못해도 앱은 계속 동작한다: 다음 실행에서 이전 값이나 기본값을 쓴다. */
export function saveWindowState(path: string, state: WindowState): void {
  try {
    const temporary = `${path}.tmp`;
    writeFileSync(temporary, `${JSON.stringify(parseWindowState(state), null, 2)}\n`, "utf8");
    renameSync(temporary, path);
  } catch {
    // 저장하지 못했다.
  }
}
