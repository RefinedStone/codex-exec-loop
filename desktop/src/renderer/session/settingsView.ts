// 설정 화면의 계산이다 (plan 26 U2). 화면과 저장소를 읽지 않으므로 그대로 시험한다.
import { type AppSettings, DEFAULT_SETTINGS } from "../../shared/settings";

/**
 * 글자 크기 설정을 CSS 토큰으로 바꾼다 (Z4). UI 크기는 기본값과의 차이만큼 UI 토큰 넷을 함께 옮긴다. 대화와 코드는
 * 토큰이 하나씩이다 (plan 26 U-1).
 */
export function fontTokens(fonts: AppSettings["fonts"]): Record<string, string> {
  const shift = fonts.ui - DEFAULT_SETTINGS.fonts.ui;
  return {
    "--text-xs": `${12 + shift}px`,
    "--text-s": `${12 + shift}px`,
    "--text-m": `${13 + shift}px`,
    "--text-l": `${14 + shift}px`,
    "--text-reading": `${fonts.reading}px`,
    "--text-code": `${fonts.code}px`,
  };
}

/** 이름, 설명, 구역 이름에 찾는 글이 있는 설정 항목인지 본다. 빈 글이면 모든 항목이다. */
export function matchesSetting(row: { section: string; label: string; hint: string }, query: string): boolean {
  const needle = query.trim().toLowerCase();
  return needle.length === 0 || `${row.section} ${row.label} ${row.hint}`.toLowerCase().includes(needle);
}
