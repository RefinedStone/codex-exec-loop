// 앱 설정이다 (plan 26 U2: S1, Z4, A2, W5). main 이 userData 에 저장하고 모든 창에 알린다.
// 파일은 사용자가 고칠 수 있고 renderer 도 값을 보내므로, 쓰기 전에 항상 `validateSettings` 를 거친다.

export type NotificationMode = "off" | "background" | "always";
export type KeepAwakeMode = "off" | "working" | "on";

/** 글자 크기를 따로 정하는 자리이다. 값은 CSS 픽셀이다. */
export type FontArea = "ui" | "reading" | "code" | "terminal";

export interface AppSettings {
  fonts: Record<FontArea, number>;
  notifications: {
    /** 끄기 / 창이 뒤에 있을 때만 / 항상 */
    mode: NotificationMode;
    sound: boolean;
  };
  /** 잠자기 방지: 끄기 / 에이전트가 일할 때만 / 항상 */
  keepAwake: KeepAwakeMode;
}

/** 부분 변경은 중첩 속성도 바꾼 값만 전달한다. */
export interface AppSettingsPatch {
  fonts?: Partial<AppSettings["fonts"]>;
  notifications?: Partial<AppSettings["notifications"]>;
  keepAwake?: KeepAwakeMode;
}

export const FONT_AREAS: readonly FontArea[] = ["ui", "reading", "code", "terminal"];
export const MIN_FONT_PX = 10;
export const MAX_FONT_PX = 24;

export const DEFAULT_SETTINGS: AppSettings = {
  fonts: { ui: 13, reading: 15, code: 13, terminal: 13 },
  notifications: { mode: "background", sound: true },
  keepAwake: "off",
};

function fontSize(value: unknown, fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(MAX_FONT_PX, Math.max(MIN_FONT_PX, Math.round(value)));
}

function oneOf<T extends string>(value: unknown, allowed: readonly T[], fallback: T): T {
  return allowed.includes(value as T) ? (value as T) : fallback;
}

/** 모르는 값은 기본값으로 바꾼다. 모르는 키는 버린다. */
export function validateSettings(input: unknown): AppSettings {
  const raw = (input ?? {}) as { fonts?: Record<string, unknown>; notifications?: Record<string, unknown>; keepAwake?: unknown };
  const fonts = raw.fonts ?? {};
  const notifications = raw.notifications ?? {};
  return {
    fonts: {
      ui: fontSize(fonts.ui, DEFAULT_SETTINGS.fonts.ui),
      reading: fontSize(fonts.reading, DEFAULT_SETTINGS.fonts.reading),
      code: fontSize(fonts.code, DEFAULT_SETTINGS.fonts.code),
      terminal: fontSize(fonts.terminal, DEFAULT_SETTINGS.fonts.terminal),
    },
    notifications: {
      mode: oneOf<NotificationMode>(notifications.mode, ["off", "background", "always"], DEFAULT_SETTINGS.notifications.mode),
      sound: typeof notifications.sound === "boolean" ? notifications.sound : DEFAULT_SETTINGS.notifications.sound,
    },
    keepAwake: oneOf<KeepAwakeMode>(raw.keepAwake, ["off", "working", "on"], DEFAULT_SETTINGS.keepAwake),
  };
}

/** 운영체제 알림을 보일지 정한다 (A2). `focused` 는 이 창에 포커스가 있다는 뜻이다. */
export function shouldNotify(mode: NotificationMode, focused: boolean): boolean {
  if (mode === "off") return false;
  return mode === "always" || !focused;
}

/** 잠자기를 막을지 정한다 (W5). `working` 은 어느 창에서든 에이전트가 턴을 실행 중이라는 뜻이다. */
export function shouldKeepAwake(mode: KeepAwakeMode, working: boolean): boolean {
  return mode === "on" || (mode === "working" && working);
}

/** main의 최신 설정에 변경한 필드만 반영한다. */
export function mergeSettings(current: AppSettings, patch: unknown): AppSettings {
  const record = (value: unknown): Record<string, unknown> =>
    typeof value === "object" && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : {};
  const changes = record(patch);
  return validateSettings({
    ...current,
    ...changes,
    fonts: { ...current.fonts, ...record(changes.fonts) },
    notifications: { ...current.notifications, ...record(changes.notifications) },
  });
}
