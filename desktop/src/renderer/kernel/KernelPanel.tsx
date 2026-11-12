// 커널 패널이다 (plan 18 K3-6). 열린 스레드의 저장소 커널이 하는 일(일이 남은 대화, 자동 후속, 병렬 작업,
// planning 큐)을 보여 준다. 대화와 승인 답은 대화 화면이 Gateway 로 직접 한다. 이 패널은 보기만 한다.
import { useState } from "react";

import { useDesktopStore } from "../session/desktopStore";
import type { KernelView } from "./kernelConnection";
import type { ParallelState, PlanningState, PlanningTask, ThreadActivity } from "./protocol";
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

function TaskRow({ task }: { task: PlanningTask }) {
  return (
    <li className="work-item" data-testid="kernel-task">
      <span>
        {task.rank}. {task.title}
      </span>
      <div className="work-meta">
        <span className="muted">{task.direction_title}</span>
        <span className="work-badge">{TASK_STATUS[task.status]}</span>
      </div>
    </li>
  );
}

function PlanningSection({ planning }: { planning: PlanningState | null }) {
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
          <TaskRow key={task.task_id} task={task} />
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

function ParallelSection({ parallel }: { parallel: ParallelState | null }) {
  if (!parallel) {
    return <p className="muted">읽는 중…</p>;
  }
  const supervisor = parallel.supervisor;
  return (
    <>
      <p data-testid="kernel-parallel-mode">
        병렬 모드 {parallel.enabled ? "켜짐" : "꺼짐"}
        {supervisor ? ` · ${SUPERVISOR_STATE[supervisor.state]}` : ""}
      </p>
      {parallel.status && <p className="muted">{parallel.status}</p>}
      {supervisor && (
        <>
          {supervisor.notice && <p className="warning">{supervisor.notice}</p>}
          <p className="muted">
            slot {supervisor.pool.configured}개: 실행 {supervisor.pool.running} · 대기 {supervisor.pool.idle} · 정리 대기{" "}
            {supervisor.pool.awaiting_cleanup} · 막힘 {supervisor.pool.blocked}
          </p>
          <ul>
            {supervisor.agents.map((agent) => (
              <li key={`${agent.agent_id}-${agent.slot_id}`} className="work-item" data-testid="kernel-agent">
                <span>{agent.task_title}</span>
                <div className="work-meta">
                  <span className="muted">
                    {agent.slot_id} · {agent.branch}
                  </span>
                  <span className="work-badge">
                    {agent.state} {agent.duration}
                  </span>
                </div>
                {agent.summary && <div className="muted">{agent.summary}</div>}
              </li>
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

function KernelBody({ kernel, threadTitle }: { kernel: KernelView; threadTitle(threadId: string): string }) {
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
        <ParallelSection parallel={kernel.parallel} />
      </section>
      <section className="work-section">
        <h3>planning 큐</h3>
        <PlanningSection planning={kernel.planning} />
      </section>
    </div>
  );
}

export function KernelPanel({ onRefresh }: { onRefresh(): Promise<void> }) {
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
        <button type="button" onClick={refresh} disabled={kernel.phase !== "attached" || refreshing}>
          {refreshing ? "읽는 중…" : "새로 고침"}
        </button>
      </div>
      <KernelBody kernel={kernel} threadTitle={threadTitle} />
    </aside>
  );
}
