// 워커 상태판의 계산이다 (#2378). 커널의 roster 상태를 화면의 상태 여섯 가지로 줄인다. 순수 함수이다.
import type { ParallelAgentState } from "./protocol";

export type WorkerStatus = "waiting" | "running" | "blocked" | "done" | "failed" | "unknown";

export const WORKER_STATUS_LABEL: Record<WorkerStatus, string> = {
  waiting: "대기",
  running: "실행 중",
  blocked: "차단",
  done: "완료",
  failed: "실패",
  unknown: "확인 못 함",
};

/** 워커의 현재 단계이다. */
export const WORKER_STEP: Record<ParallelAgentState, string> = {
  starting: "시작 준비",
  running: "작업 중",
  reported_complete: "완료 보고 확인",
  ledger_refreshing: "장부 갱신",
  commit_ready: "커밋 준비",
  merge_queued: "병합 큐 대기",
  pushing: "push",
  pr_pending: "PR 대기",
  merge_pending: "병합 대기",
  integrating: "통합",
  cleanup_pending: "slot 정리 대기",
  official_refresh_recovery_needed: "복구 필요",
  failed: "실패",
  unknown: "알 수 없음",
};

const STATUS: Record<ParallelAgentState, WorkerStatus> = {
  starting: "waiting",
  running: "running",
  reported_complete: "running",
  ledger_refreshing: "running",
  commit_ready: "running",
  merge_queued: "waiting",
  pushing: "running",
  pr_pending: "waiting",
  merge_pending: "waiting",
  integrating: "running",
  // branch 가 기준 branch 에 통합된 뒤의 상태이다. 이 상태만 완료로 본다.
  cleanup_pending: "done",
  official_refresh_recovery_needed: "blocked",
  failed: "failed",
  unknown: "unknown",
};

/** 표에 없는 상태(더 새로운 커널)는 완료로 보지 않는다. */
export function workerStatus(state: ParallelAgentState): WorkerStatus {
  return STATUS[state] ?? "unknown";
}
