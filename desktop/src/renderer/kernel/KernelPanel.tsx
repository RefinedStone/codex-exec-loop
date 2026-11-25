// 커널 패널이다 (plan 18 K3-6). 열린 스레드의 저장소 커널이 하는 일(일이 남은 대화, 자동 후속, 병렬 작업,
// planning 큐)을 보여 준다. 풀의 실행·중단은 명시적인 커널 명령으로 요청하고 적용 상태는 커널에서 읽는다.
import { useState } from "react";

import { useDesktopStore } from "../session/desktopStore";
import type { KernelView, ParallelControlView, PlanningEvidenceFreshness } from "./kernelConnection";
import { TaskEvidenceView, type TaskEvidenceViewProps } from "./TaskEvidenceView";
import type { TaskVerificationActions } from "./TaskVerificationView";
import type { ParallelAgent, ParallelState, PlanningState, PlanningTask, ThreadActivity } from "./protocol";
import { WORKER_STATUS_LABEL, WORKER_STEP, workerStatus, type WorkerStatus } from "./workerStatus";
import { folderName } from "../sidebar/threadStatus";

const PLANNING_STATUS: Record<PlanningState["status"], string> = {
  uninitialized: "planning 없음 (또는 아직 읽지 않음)",
  invalid: "planning 오류",
  ready_without_task: "실행할 작업 없음",
  ready_with_task: "실행할 작업 있음",
};

const TASK_STATUS: Record<PlanningTask["status"], string> = {
  ready: "대기",
  blocked: "막힘",
  in_progress: "진행 중",
  done: "완료",
  cancelled: "취소",
  awaiting_user: "사용자 응답 대기",
  proposed: "제안",
};

const SUPERVISOR_STATE: Record<NonNullable<ParallelState["supervisor"]>["state"], string> = {
  prepare: "준비",
  supervise: "감독 중",
  recover: "복구 필요",
};

function ThreadRow({ activity, title }: { activity: ThreadActivity; title: string }) {
  return (
    <li className="work-item" data-testid="kernel-thread">
      <span>{title}</span>
      <div className="work-meta">
        {activity.running && <span className="work-badge">턴 진행 중</span>}
        {activity.auto_follow_active && <span className="work-badge">자동 후속</span>}
        {activity.approval && (
          <span className="work-badge" title={activity.approval.details.join("\n")}>
            승인 대기: {activity.approval.summary}
          </span>
        )}
      </div>
    </li>
  );
}

function TaskRow({ task, snapshot, freshness, ...actions }: { task: PlanningTask } & Omit<TaskEvidenceViewProps, "taskId">) {
  return (
    <li className="work-item" data-testid="kernel-task">
      <span>
        {task.rank}. {task.title}
      </span>
      <div className="work-meta">
        <span className="muted">{task.direction_title}</span>
        <span className="work-badge">{TASK_STATUS[task.status]}</span>
      </div>
      <TaskEvidenceView taskId={task.task_id} snapshot={snapshot} freshness={freshness} {...actions} />
    </li>
  );
}

function PlanningSection({ planning, freshness, ...actions }: { planning: PlanningState | null; freshness: PlanningEvidenceFreshness } & TaskVerificationActions) {
  if (!planning) {
    return <p className="muted">읽는 중…</p>;
  }
  return (
    <>
      <p data-testid="kernel-planning-status">
        {PLANNING_STATUS[planning.status]}
        {planning.queue_summary ? ` · ${planning.queue_summary}` : ""}
      </p>
      {planning.failure_reason && <p className="error">{planning.failure_reason}</p>}
      {planning.auto_follow_pause_reason && <p className="warning">자동 후속 멈춤: {planning.auto_follow_pause_reason}</p>}
      <ul>
        {planning.active_tasks.map((task) => (
          <TaskRow key={task.task_id} task={task} snapshot={planning.task_evidence} freshness={freshness} {...actions} />
        ))}
      </ul>
      {planning.proposed_tasks.length > 0 && <p className="muted">받아들이지 않은 제안 {planning.proposed_tasks.length}건</p>}
      {planning.skipped_tasks.length > 0 && (
        <details>
          <summary className="muted">제외된 작업 {planning.skipped_tasks.length}건</summary>
          <ul>
            {planning.skipped_tasks.map((task) => (
              <li key={task.task_id} className="muted">
                {task.title}: {task.reason}
              </li>
            ))}
          </ul>
        </details>
      )}
    </>
  );
}

const WORKER_BADGE: Record<WorkerStatus, string> = {
  waiting: "work-badge",
  running: "work-badge",
  blocked: "badge badge-warn",
  done: "badge badge-ok",
  failed: "badge badge-bad",
  unknown: "badge badge-warn",
};

/** 워커 한 줄이다 (#2378): 작업, 상태, 현재 단계, worktree(slot 과 branch). 스레드가 있으면 제목을 눌러 그 스레드로 간다. */
function WorkerRow({ agent, onOpenThread }: { agent: ParallelAgent; onOpenThread(threadId: string): void }) {
  const status = workerStatus(agent.state);
  const threadId = agent.thread_id;
  return (
    <li className="work-item" data-testid="kernel-agent" data-status={status}>
      {threadId ? (
        <button type="button" className="work-link" title="워커의 스레드 열기" onClick={() => onOpenThread(threadId)}>
          {agent.task_title}
        </button>
      ) : (
        <span>{agent.task_title}</span>
      )}
      <div className="work-meta">
        <span className={WORKER_BADGE[status]}>{WORKER_STATUS_LABEL[status]}</span>
        <span>
          {WORKER_STEP[agent.state] ?? agent.state} {agent.duration}
        </span>
        <span className="muted" title="worktree slot · branch">
          {agent.slot_id} · {agent.branch}
        </span>
      </div>
      {agent.summary && <div className="muted">{agent.summary}</div>}
    </li>
  );
}

function ParallelSection({ parallel, control, canControl, onSetEnabled, onOpenThread }: {
  parallel: ParallelState | null;
  control: ParallelControlView;
  canControl: boolean;
  onSetEnabled(enabled: boolean): Promise<void>;
  onOpenThread(threadId: string): void;
}) {
  if (!parallel) {
    return <p className="muted">읽는 중…</p>;
  }
  const supervisor = parallel.supervisor;
  return (
    <>
      <p data-testid="kernel-parallel-mode">
        {control.pending ? "병렬 상태 확인 중" : control.uncertain ? "병렬 상태 확인 필요" : "병렬 모드"} {parallel.enabled ? "켜짐" : "꺼짐"}
        {(control.uncertain || control.pending) && " (마지막 확인)"}
        {supervisor ? ` · ${SUPERVISOR_STATE[supervisor.state]}` : ""}
      </p>
      <button type="button" data-testid="kernel-parallel-control" disabled={!canControl || control.pending !== null || control.uncertain}
        onClick={() => { void onSetEnabled(!parallel.enabled); }}>
        {control.pending === "enable" ? "시작 요청 중…" : control.pending === "disable" ? "중단 요청 중…"
          : parallel.enabled ? "병렬 실행 중단" : "병렬 실행 시작"}
      </button>
      <p className="muted">이 저장소의 준비된 작업을 실행합니다. 이슈별 승인과 전달 조건은 유지됩니다.</p>
      {control.notice && <p className="muted" data-testid="kernel-parallel-notice">{control.notice}</p>}
      {control.error && <p className="error" role="alert" data-testid="kernel-parallel-error">{control.error}</p>}
      {parallel.status && <p className="muted" data-testid="kernel-parallel-status">{parallel.status}</p>}
      {supervisor && (
        <>
          {supervisor.notice && <p className="warning">{supervisor.notice}</p>}
          <p className="muted">
            slot {supervisor.pool.configured}개: 실행 {supervisor.pool.running} · 대기 {supervisor.pool.idle} · 정리 대기{" "}
            {supervisor.pool.awaiting_cleanup} · 막힘 {supervisor.pool.blocked}
          </p>
          <ul>
            {supervisor.agents.map((agent) => (
              <WorkerRow key={`${agent.agent_id}-${agent.slot_id}`} agent={agent} onOpenThread={onOpenThread} />
            ))}
          </ul>
          {supervisor.merge_queue.length > 0 && (
            <>
              <h4>병합 큐 · {supervisor.merge_head}</h4>
              <ul>
                {supervisor.merge_queue.map((item) => (
                  <li key={`${item.branch}-${item.commit}`} className="work-item" data-testid="kernel-merge-item">
                    <span>{item.task_title}</span>
                    <div className="work-meta">
                      <span className="muted">{item.branch}</span>
                      <span className="work-badge">{item.state}</span>
                    </div>
                  </li>
                ))}
              </ul>
            </>
          )}
          {supervisor.pr_validation && (
            <p data-testid="kernel-pr-validation">
              PR #{supervisor.pr_validation.pull_request_number} 검증: {supervisor.pr_validation.state} ({supervisor.pr_validation.phase})
              {supervisor.pr_validation.blocker ? ` · ${supervisor.pr_validation.blocker}` : ""}
            </p>
          )}
        </>
      )}
    </>
  );
}

function KernelBody({
  kernel,
  threadTitle,
  onSetParallelEnabled,
  onOpenThread,
  onRequestVerification,
}: {
  kernel: KernelView;
  threadTitle(threadId: string): string;
  onSetParallelEnabled(enabled: boolean): Promise<void>;
  onOpenThread(threadId: string): void;
} & TaskVerificationActions) {
  if (kernel.workspace === null) {
    return <p className="muted work-section">스레드를 열면 그 폴더의 저장소 커널을 보여 줍니다.</p>;
  }
  if (kernel.phase === "failed") {
    return (
      <p className="error work-section" data-testid="kernel-error">
        {kernel.message}
      </p>
    );
  }
  // 같은 커널에 다시 붙는 동안에는 보던 상태를 그대로 보인다.
  const reattaching = kernel.phase === "attaching" && kernel.kernel !== null;
  if (kernel.phase !== "attached" && !reattaching) {
    return <p className="muted work-section">{kernel.message ?? "커널에 붙는 중…"}</p>;
  }
  return (
    <div className="work-scroll">
      {reattaching && <p className="muted work-section">다시 붙는 중…</p>}
      {kernel.phase === "attached" && kernel.message && (
        <p className="warning work-section" data-testid="kernel-warning">
          {kernel.message}
        </p>
      )}
      <section className="work-section">
        <h3>일이 남은 대화 ({kernel.threads.length})</h3>
        {kernel.threads.length === 0 && <p className="muted">진행 중인 턴, 승인, 자동 후속이 없습니다.</p>}
        <ul>
          {kernel.threads.map((activity) => (
            <ThreadRow key={activity.thread_id} activity={activity} title={threadTitle(activity.thread_id)} />
          ))}
        </ul>
        {kernel.autoFollow && (
          <p className="muted" data-testid="kernel-auto-follow">
            {kernel.autoFollow.max_turns === 0 ? "자동 후속 꺼짐" : `자동 후속 최대 ${kernel.autoFollow.max_turns}턴`}
            {kernel.autoFollow.paused ? " · 멈춤" : ""}
          </p>
        )}
      </section>
      <section className="work-section">
        <h3>병렬 작업</h3>
        <ParallelSection parallel={kernel.parallel} control={kernel.parallelControl} canControl={kernel.phase === "attached"}
          onSetEnabled={onSetParallelEnabled} onOpenThread={onOpenThread} />
      </section>
      <section className="work-section">
        <h3>planning 큐</h3>
        <PlanningSection planning={kernel.planning} freshness={kernel.planningEvidenceFreshness}
          verificationRequests={kernel.taskVerificationRequests} onRequestVerification={kernel.phase === "attached" ? onRequestVerification : undefined} />
      </section>
    </div>
  );
}

export function KernelPanel({ onRefresh, onSetParallelEnabled, onOpenThread, onEditTasks, onRequestVerification }: {
  onRefresh(): Promise<void>;
  onSetParallelEnabled(enabled: boolean): Promise<void>;
  onOpenThread(threadId: string): void;
  onEditTasks?(): void;
} & TaskVerificationActions) {
  const kernel = useDesktopStore((state) => state.kernel);
  const folders = useDesktopStore((state) => state.folders);
  const [refreshing, setRefreshing] = useState(false);
  const threadTitle = (threadId: string) =>
    folders.flatMap((folder) => folder.threads).find((thread) => thread.id === threadId)?.title ?? threadId;
  const refresh = () => {
    setRefreshing(true);
    void onRefresh()
      .catch(() => undefined)
      .finally(() => setRefreshing(false));
  };
  return (
    <aside className="work-panel" data-testid="kernel-panel">
      <div className="work-header">
        <strong>Akra 커널</strong>
        {kernel.kernel && kernel.workspace && (
          <span className="muted" title={`${kernel.workspace}\n${kernel.kernel}`} data-testid="kernel-identity" data-kernel={kernel.kernel}>
            {folderName(kernel.workspace)}
          </span>
        )}
        {onEditTasks && <button type="button" onClick={onEditTasks} disabled={kernel.phase !== "attached"} data-testid="kernel-edit-tasks">작업 편집</button>}
        <button type="button" onClick={refresh} disabled={kernel.phase !== "attached" || refreshing}>
          {refreshing ? "읽는 중…" : "새로 고침"}
        </button>
      </div>
      <KernelBody kernel={kernel} threadTitle={threadTitle} onSetParallelEnabled={onSetParallelEnabled} onOpenThread={onOpenThread} onRequestVerification={onRequestVerification} />
    </aside>
  );
}
