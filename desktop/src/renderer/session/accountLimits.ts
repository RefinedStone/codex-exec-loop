// 계정 사용 한도이다 (#2821). app-server 의 `account/rateLimits/read` 와 `account/rateLimits/updated` 를 그대로 보인다.
// 서버가 주지 않은 값은 0 으로 채우지 않는다: 창(window)이 없으면 그 줄을 보이지 않는다.
import { create } from "zustand";

import type { RateLimitSnapshot } from "../../shared/protocol/generated/v2/RateLimitSnapshot";
import type { RateLimitWindow } from "../../shared/protocol/generated/v2/RateLimitWindow";

export type AccountLimitsStatus = "unknown" | "unsupported" | "unavailable" | "ready";

export interface AccountLimitsState {
  status: AccountLimitsStatus;
  snapshot: RateLimitSnapshot | null;
}

export const useAccountLimits = create<AccountLimitsState>(() => ({ status: "unknown", snapshot: null }));

export const ACCOUNT_LIMITS_READ = "account/rateLimits/read";
export const ACCOUNT_LIMITS_UPDATED = "account/rateLimits/updated";

function isSnapshot(value: unknown): value is RateLimitSnapshot {
  return value !== null && typeof value === "object" && "primary" in value && "secondary" in value;
}

/** 읽기 응답을 반영한다. 모양이 맞지 않으면 "알 수 없음" 으로 둔다. */
export function applyAccountLimitsRead(response: unknown): void {
  const snapshot = response !== null && typeof response === "object" ? (response as { rateLimits?: unknown }).rateLimits : undefined;
  useAccountLimits.setState(isSnapshot(snapshot) ? { status: "ready", snapshot } : { status: "unavailable", snapshot: null });
}

/** 갱신 알림을 반영한다. 다루는 알림이면 true 이다. */
export function applyAccountLimitsNotification(method: string, params: unknown): boolean {
  if (method !== ACCOUNT_LIMITS_UPDATED) return false;
  const snapshot = params !== null && typeof params === "object" ? (params as { rateLimits?: unknown }).rateLimits : undefined;
  if (isSnapshot(snapshot)) useAccountLimits.setState({ status: "ready", snapshot });
  return true;
}

export function setAccountLimitsStatus(status: AccountLimitsStatus): void {
  useAccountLimits.setState({ status, snapshot: null });
}

/** 창 길이의 이름이다. 300분은 "5시간", 10080분은 "주간" 이다. */
export function windowLabel(window: RateLimitWindow): string {
  const minutes = window.windowDurationMins;
  if (minutes === null) return "한도";
  if (minutes === 10_080) return "주간";
  if (minutes % 1_440 === 0) return `${minutes / 1_440}일`;
  if (minutes % 60 === 0) return `${minutes / 60}시간`;
  return `${minutes}분`;
}

/** 초기화 시각이다. 하루 안이면 시각만, 그 뒤면 날짜도 쓴다. 서버가 주지 않았으면 null 이다. */
export function resetLabel(window: RateLimitWindow, now: Date = new Date()): string | null {
  if (window.resetsAt === null) return null;
  const at = new Date(window.resetsAt * 1_000);
  const time = at.toLocaleTimeString("ko-KR", { hour: "2-digit", minute: "2-digit" });
  return at.getTime() - now.getTime() < 86_400_000 ? time : `${at.toLocaleDateString("ko-KR", { month: "numeric", day: "numeric" })} ${time}`;
}

/** 보일 창들이다. 서버가 준 것만이다. */
export function limitWindows(snapshot: RateLimitSnapshot | null): RateLimitWindow[] {
  return snapshot ? [snapshot.primary, snapshot.secondary].filter((window): window is RateLimitWindow => window !== null) : [];
}
