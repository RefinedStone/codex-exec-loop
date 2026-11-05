// 브라우저 패널의 프로필과 프리셋이다 (R3). 사용자 설정 폴더(userData) 의 JSON 파일에만 둔다. 사내 주소를 저장소에 넣지
// 않는다. 파일 내용은 사용자가 고칠 수 있으므로 읽을 때마다 검증한다.
import { readFile, rename, writeFile } from "node:fs/promises";

import type { BrowserPreset, BrowserProfile, BrowserSettings } from "../shared/bridge";

const PROFILE_ID = /^[a-z0-9][a-z0-9-]{0,31}$/;
const MAX_NAME = 60;
const MAX_ITEMS = 100;

export const DEFAULT_SETTINGS: BrowserSettings = { profiles: [{ id: "default", name: "기본" }], presets: [] };

/** 세션 저장소 이름이다. 프로필마다 쿠키, localStorage, 캐시가 따로 있다. 앱 화면의 세션과도 다르다. */
export function profilePartition(profileId: string): string {
  if (!PROFILE_ID.test(profileId)) {
    throw new Error("프로필 id 가 올바르지 않습니다.");
  }
  return `persist:akra-browser-${profileId}`;
}

/**
 * 주소창 입력을 열 수 있는 주소로 바꾼다. http(s) 와 about:blank 만 연다. 다른 scheme(file:, javascript: 등)은 막는다.
 * 주소 형태가 아니면 null 이다.
 */
export function normalizeUrl(input: string): string | null {
  const text = input.trim();
  if (text === "about:blank") return text;
  let candidate = text;
  // `localhost:5173` 처럼 host:port 이면 scheme 이 아니다.
  const hasScheme = /^[a-z][a-z0-9+.-]*:/i.test(text) && !/^[^\s/:]+:\d+(\/|$)/.test(text);
  if (!hasScheme) {
    if (/\s/.test(text) || !(text.includes(".") || /^localhost(:\d+)?(\/|$)/i.test(text))) return null;
    const local = /^(localhost|127\.|\d+\.\d+\.\d+\.\d+)/i.test(text);
    candidate = `${local ? "http" : "https"}://${text}`;
  }
  try {
    const url = new URL(candidate);
    return url.protocol === "http:" || url.protocol === "https:" ? url.toString() : null;
  } catch {
    return null;
  }
}

function text(value: unknown, max: number): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value.trim().slice(0, max) : null;
}

/** 파일이나 renderer 에서 온 설정을 검증한다. 잘못된 항목은 버리고, 프로필이 없으면 기본 프로필을 둔다. */
export function validateSettings(input: unknown): BrowserSettings {
  const raw = (input ?? {}) as { profiles?: unknown; presets?: unknown };
  const profiles: BrowserProfile[] = [];
  for (const item of Array.isArray(raw.profiles) ? raw.profiles.slice(0, MAX_ITEMS) : []) {
    const id = (item as { id?: unknown })?.id;
    const name = text((item as { name?: unknown })?.name, MAX_NAME);
    if (typeof id === "string" && PROFILE_ID.test(id) && name && !profiles.some((profile) => profile.id === id)) {
      profiles.push({ id, name });
    }
  }
  if (profiles.length === 0) profiles.push(...DEFAULT_SETTINGS.profiles);
  const presets: BrowserPreset[] = [];
  for (const item of Array.isArray(raw.presets) ? raw.presets.slice(0, MAX_ITEMS) : []) {
    const name = text((item as { name?: unknown })?.name, MAX_NAME);
    const url = typeof (item as { url?: unknown })?.url === "string" ? normalizeUrl((item as { url: string }).url) : null;
    const profileId = (item as { profileId?: unknown })?.profileId;
    if (name && url && typeof profileId === "string" && profiles.some((profile) => profile.id === profileId)) {
      presets.push({ name, url, profileId });
    }
  }
  return { profiles, presets };
}

export async function loadSettings(path: string): Promise<BrowserSettings> {
  try {
    return validateSettings(JSON.parse(await readFile(path, "utf8")));
  } catch {
    return DEFAULT_SETTINGS;
  }
}

/** 검증한 설정을 임시 파일에 쓰고 바꿔 넣는다. 쓰는 도중에 앱이 끝나도 이전 파일이 남는다. */
export async function saveSettings(path: string, input: unknown): Promise<BrowserSettings> {
  const settings = validateSettings(input);
  const temporary = `${path}.tmp`;
  await writeFile(temporary, `${JSON.stringify(settings, null, 2)}\n`, "utf8");
  await rename(temporary, path);
  return settings;
}
