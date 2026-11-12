// 사이드바와 현황판이 스레드 옆에 그리는 상태이다. 저장소의 값에서 계산한다.
import type { DesktopState, WorkspaceFolder, WorkspaceThread } from "../session/desktopStore";

export type ThreadMark = "approval" | "running" | "failed" | "unread" | "idle";

export type ThreadMarkSource = Pick<DesktopState, "activeThreadIds" | "unreadThreadIds" | "pendingApprovals">;

/** 사용자가 먼저 봐야 하는 것이 앞이다: 승인 대기, 진행 중, 실패, 읽지 않음. */
export function threadMark(thread: WorkspaceThread, source: ThreadMarkSource): ThreadMark {
  if (source.pendingApprovals.some((approval) => approval.threadId === thread.id)) return "approval";
  if (source.activeThreadIds.includes(thread.id)) return "running";
  if (thread.failed) return "failed";
  if (source.unreadThreadIds.includes(thread.id)) return "unread";
  return "idle";
}

export function needsAttention(mark: ThreadMark): boolean {
  return mark === "approval" || mark === "failed";
}

export function folderName(cwd: string): string {
  return cwd.split(/[\\/]/).filter(Boolean).at(-1) ?? cwd;
}

export interface ListedThread extends WorkspaceThread {
  cwd: string;
}

/** 모든 폴더의 스레드를 최근 순서로 놓는다. */
export function allThreads(folders: readonly WorkspaceFolder[]): ListedThread[] {
  return folders.flatMap((folder) => folder.threads.map((thread) => ({ ...thread, cwd: folder.cwd }))).sort((left, right) => right.updatedAt - left.updatedAt);
}

const MINUTE = 60;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** `updatedAt`(Unix 초)에서 지금까지 지난 시간을 짧게 쓴다. */
export function relativeAge(updatedAtSeconds: number, nowMs: number): string {
  const seconds = Math.max(0, Math.floor(nowMs / 1000) - updatedAtSeconds);
  if (seconds < MINUTE) return "방금";
  if (seconds < HOUR) return `${Math.floor(seconds / MINUTE)}분`;
  if (seconds < DAY) return `${Math.floor(seconds / HOUR)}시간`;
  return `${Math.floor(seconds / DAY)}일`;
}
