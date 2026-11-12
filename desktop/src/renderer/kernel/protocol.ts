// Akra 커널 응용 프로토콜 v0 (`akra/kernel/*`, plan 18 K3 §3.3) 중 Desktop 이 쓰는 부분이다. 원본은 Rust
// `src/adapter/inbound/kernel_protocol/mod.rs` 이다. 명령 스펙(K4)이 생기면 생성한 타입으로 바꾼다.

export const KERNEL_METHOD = {
  attach: "akra/kernel/attach",
  command: "akra/kernel/command",
  query: "akra/kernel/query",
  event: "akra/kernel/event",
  detach: "akra/kernel/detach",
} as const;

/** 다른 프로세스(TUI)가 그 저장소의 커널을 돌리거나, 커널에 닿지 못했다. */
export const KERNEL_UNAVAILABLE = -32012;

export interface AttachResponse {
  client_id: string;
  protocol_version: number;
  /** 저장소 신원이다. 같은 저장소의 worktree 들은 같은 값이다. */
  kernel: string;
}

export interface CommandResponse {
  processed: boolean;
  rejection?: { reason: string; message: string };
}

export interface ApprovalRequest {
  approval_id: string;
  server_request_id: string;
  summary: string;
  details: string[];
}

export interface ThreadActivity {
  thread_id: string;
  running: boolean;
  active_turn_id?: string;
  approval?: ApprovalRequest;
  auto_follow_active: boolean;
}

export interface AutoFollowSettings {
  max_turns: number;
  paused: boolean;
}

export type PlanningTaskStatus = "ready" | "blocked" | "in_progress" | "done" | "cancelled" | "awaiting_user" | "proposed";

export interface PlanningTask {
  rank: number;
  task_id: string;
  direction_title: string;
  title: string;
  status: PlanningTaskStatus;
  priority: number;
}

export interface SkippedPlanningTask {
  task_id: string;
  title: string;
  status: PlanningTaskStatus;
  reason: string;
}

export interface PlanningState {
  status: "uninitialized" | "invalid" | "ready_without_task" | "ready_with_task";
  revision?: number;
  queue_summary?: string;
  failure_reason?: string;
  auto_follow_pause_reason?: string;
  next_task?: PlanningTask;
  active_tasks: PlanningTask[];
  proposed_tasks: PlanningTask[];
  skipped_tasks: SkippedPlanningTask[];
}

export interface ParallelAgent {
  agent_id: string;
  thread_id?: string;
  task_title: string;
  slot_id: string;
  branch: string;
  state: string;
  duration: string;
  summary: string;
}

export interface ParallelSupervisor {
  state: "prepare" | "supervise" | "recover";
  pool: {
    configured: number;
    idle: number;
    leased: number;
    running: number;
    awaiting_cleanup: number;
    blocked: number;
    missing: number;
    unavailable: number;
    exhausted: boolean;
  };
  agents: ParallelAgent[];
  merge_head: string;
  merge_queue: Array<{ task_title: string; state: string; branch: string; commit: string }>;
  pr_validation?: { pull_request_number: number; url: string; state: string; phase: string; reason: string; blocker?: string };
  notice?: string;
}

export interface ParallelState {
  enabled: boolean;
  status?: string;
  supervisor?: ParallelSupervisor;
}

export interface PromptSubmissionState {
  thread_id: string;
  status: "preparing" | "submitted" | "queued" | "rejected" | "failed" | "dropped";
  reason?: string;
  task_id?: string;
  message?: string;
}

export type KernelEvent =
  | ({ type: "sessions" } & Record<string, unknown>)
  | ({ type: "auto_follow" } & AutoFollowSettings)
  | { type: "thread_activity"; threads: ThreadActivity[] }
  | ({ type: "prompt_submission" } & PromptSubmissionState)
  | ({ type: "planning" } & PlanningState)
  | ({ type: "parallel" } & ParallelState)
  | { type: "resync" };

/** Desktop 이 붙은 뒤 읽는 조회이다. 조회 결과는 같은 이름의 이벤트와 모양이 같다. */
export const KERNEL_VIEW_QUERIES = ["thread_activity", "auto_follow", "planning", "parallel"] as const;
export type KernelViewQuery = (typeof KERNEL_VIEW_QUERIES)[number];
