/** 활성 작업의 완료 기준과 E2 읽기 metadata다. 판정과 쓰기 권한은 커널에 남긴다. */
import type { PlanningEvidenceFreshness } from "./kernelConnection";
import { TaskVerificationView, type TaskVerificationActions } from "./TaskVerificationView";
import type {
  PlanningCriterionEvidenceState, PlanningState, TaskAcceptanceCriterion, TaskCriterionEvidenceState,
  TaskEvidenceError, TaskEvidenceTaskState, TaskEvidenceTree,
} from "./protocol";

export interface TaskEvidenceViewProps extends TaskVerificationActions {
  taskId: string;
  snapshot: PlanningState["task_evidence"];
  freshness: PlanningEvidenceFreshness;
}

const CHANNEL_LABEL: Record<TaskAcceptanceCriterion["channel"], string> = {
  command: "명령", data: "데이터", http: "HTTP", screen: "화면",
};

const STATE_LABEL: Record<TaskCriterionEvidenceState, string> = {
  pending: "보고 대기",
  pass: "작업자 보고: 통과",
  fail: "작업자 보고: 실패",
  blocked: "작업자 보고: 막힘",
  stale: "이전 범위의 보고",
  waived: "사람 면제",
};

const ERROR_LABEL: Record<TaskEvidenceError, string> = {
  invalid: "증거 정보의 형식이 올바르지 않습니다.",
  missing: "필요한 증거 정보를 찾지 못했습니다.",
  artifact_missing: "증거 파일을 찾지 못했습니다.",
  stale: "조회 기준이 바뀌어 현재 상태를 확인하지 못했습니다.",
  scope_changed: "완료 기준의 범위가 바뀌었습니다.",
  lease_changed: "실행 중인 작업의 소유권이 바뀌었습니다.",
  conflict: "증거 정보가 충돌하여 현재 상태를 확인하지 못했습니다.",
  unavailable: "증거 정보를 읽지 못했습니다.",
  limit: "조회 한도를 넘어 증거 정보를 읽지 못했습니다.",
  human_confirmation_required: "사람의 확인이 필요한 상태입니다.",
};

const FRESHNESS_LABEL: Record<PlanningEvidenceFreshness, string> = {
  current: "최근 커널 조회 기준입니다.",
  refreshing: "다시 확인하는 중입니다. 이전 조회 정보를 표시합니다.",
  stale: "현재 상태를 확인하지 못했습니다. 마지막 조회 정보를 표시합니다.",
};

function Unavailable({ reason }: { reason: TaskEvidenceError }) {
  return (
    <p className="warning" data-testid="task-evidence-unavailable">
      현재 상태를 확인할 수 없습니다. {ERROR_LABEL[reason]} <code>({reason})</code>
    </p>
  );
}

function TreeMetadata({ tree }: { tree: TaskEvidenceTree }) {
  return (
    <>
      <dt>커밋</dt><dd><code>{tree.head_commit_sha}</code></dd>
      <dt>파일 트리</dt><dd><code>{tree.tree_sha}</code></dd>
      <dt>작업 트리 상태</dt><dd>{tree.worktree_clean ? "변경 없음" : "변경 있음"}</dd>
    </>
  );
}

function CriterionRow({ entry, current, unavailable, task, snapshot, freshness, ...actions }: {
  entry: PlanningCriterionEvidenceState;
  current: boolean;
  unavailable: boolean;
  task: TaskEvidenceTaskState;
  snapshot: NonNullable<PlanningState["task_evidence"]>;
  freshness: PlanningEvidenceFreshness;
} & TaskVerificationActions) {
  const state = unavailable ? null : entry.state;
  const label = state == null ? "현재 상태 확인 불가" : STATE_LABEL[state];
  const reported = state === "pass" || state === "fail" || state === "blocked";
  const evidence = entry.evidence?.evidence;
  const waiver = entry.waiver?.waiver;
  const currentReport = current && reported;
  const currentWaiver = current && state === "waived";
  const tone = !current || unavailable ? "work-badge muted"
    : state === "pass" ? "badge badge-ok"
      : state === "fail" ? "badge badge-bad"
        : state === "blocked" || state === "stale" || state == null ? "badge badge-warn" : "work-badge";
  return (
    <li className="task-evidence-criterion" data-testid="task-evidence-criterion">
      <div className="work-meta">
        <span>{entry.criterion.essential ? "필수" : "선택"}</span>
        <span className="muted">{CHANNEL_LABEL[entry.criterion.channel]}</span>
        <span className={tone} data-testid="task-evidence-status" data-state={state ?? "unavailable"} data-current={current && !unavailable && state != null && state !== "stale"}>
          {!current && state != null ? "마지막 조회: " : ""}{label}
        </span>
      </div>
      <p className="task-evidence-scenario">{entry.criterion.scenario}</p>
      <p className="muted">기준 ID: <code>{entry.criterion.id}</code></p>
      <TaskVerificationView entry={entry} task={task} snapshot={snapshot} freshness={freshness} {...actions} />
      {evidence && (
        <details className="task-evidence-record">
          <summary>{currentReport ? "작업자 보고 정보" : "이전 작업자 보고"}</summary>
          <dl className="task-evidence-metadata">
            <dt>{currentReport ? "보고 결과" : "이전 보고 결과"}</dt><dd>{STATE_LABEL[evidence.status]}</dd>
            <dt>작업자</dt><dd>{evidence.by}</dd>
            <dt>기록 시각</dt><dd><time dateTime={evidence.recorded_at}>{evidence.recorded_at}</time></dd>
            <dt>시도</dt><dd>{evidence.attempt}회</dd>
            <dt>기준 판</dt><dd>{evidence.scope.acceptance_revision}</dd>
            <dt>범위 ID</dt><dd><code>{evidence.scope.scope_id}</code></dd>
            <dt>파일 SHA-256</dt><dd><code>{evidence.artifact.sha256}</code></dd>
            <dt>파일 크기</dt><dd>{evidence.artifact.size_bytes} 바이트</dd>
            <TreeMetadata tree={evidence.recorded_tree} />
          </dl>
        </details>
      )}
      {waiver && (
        <details className="task-evidence-record">
          <summary>{currentWaiver ? "사람 면제 기록" : "이전 사람 면제 기록"}</summary>
          <dl className="task-evidence-metadata">
            <dt>면제 주체</dt><dd>사람</dd>
            <dt>기록 시각</dt><dd><time dateTime={waiver.recorded_at}>{waiver.recorded_at}</time></dd>
            <dt>기준 판</dt><dd>{waiver.scope.acceptance_revision}</dd>
            <dt>범위 ID</dt><dd><code>{waiver.scope.scope_id}</code></dd>
            <dt>사유 SHA-256</dt><dd><code>{waiver.reason_digest}</code></dd>
          </dl>
          <p className="muted">면제 사유 원문은 제공되지 않습니다.</p>
        </details>
      )}
    </li>
  );
}

function TaskCriteria({ task, snapshot, freshness, ...actions }: {
  task: TaskEvidenceTaskState;
  snapshot: NonNullable<PlanningState["task_evidence"]>;
  freshness: PlanningEvidenceFreshness;
} & TaskVerificationActions) {
  return (
    <>
      {snapshot.truncated && (
        <p className="muted" data-testid="task-evidence-truncated">
          전체 작업 중 일부를 조회했습니다. 아래는 이 작업에 반환된 기준입니다.
        </p>
      )}
      {task.unavailable && <Unavailable reason={task.unavailable} />}
      {task.criteria.length === 0 ? (
        <p className="muted" data-testid="task-evidence-empty">등록된 완료 기준이 없습니다.</p>
      ) : (
        <>
          <p className="muted">통과·실패·막힘은 작업자가 보고한 결과입니다.</p>
          <ul className="task-evidence-criteria">
            {task.criteria.map((entry) => (
              <CriterionRow key={entry.criterion.id} entry={entry} current={freshness === "current"}
                unavailable={task.unavailable != null} task={task} snapshot={snapshot} freshness={freshness} {...actions} />
            ))}
          </ul>
        </>
      )}
      <details className="task-evidence-record">
        <summary>조회 기준 정보</summary>
        <dl className="task-evidence-metadata">
          <dt>Planning 판</dt><dd>{snapshot.planning_revision ?? "미제공"}</dd>
          <dt>증거 판</dt><dd>{snapshot.evidence_revision ?? "미제공"}</dd>
          <dt>완료 기준 판</dt><dd>{task.scope.acceptance_revision}</dd>
          <dt>범위 ID</dt><dd><code>{task.scope.scope_id}</code></dd>
          <dt>기준 SHA-256</dt><dd><code>{task.scope.criteria_sha256}</code></dd>
          {task.source ? (
            <>
              <dt>조회한 소스</dt><dd>{task.source.kind === "running_lease" ? "실행 중 작업" : "대기 중 작업"}</dd>
              <TreeMetadata tree={task.source.tree} />
            </>
          ) : (
            <><dt>조회한 소스</dt><dd>정보 미제공</dd></>
          )}
        </dl>
      </details>
    </>
  );
}

export function TaskEvidenceView({ taskId, snapshot, freshness, ...actions }: TaskEvidenceViewProps) {
  const task = snapshot?.unavailable ? undefined : snapshot?.tasks.find((entry) => entry.scope.task_id === taskId);
  const summaryState = freshness === "refreshing" ? "확인 중" : freshness === "stale" ? "마지막 조회"
    : snapshot == null ? "미제공" : snapshot.unavailable || task?.unavailable ? "확인 불가"
      : !task ? (snapshot.truncated ? "조회 범위 밖" : "항목 없음") : null;
  return (
    <details className="task-evidence" data-testid="task-evidence" data-task-id={taskId} data-freshness={freshness}>
      <summary>
        완료 기준 · 증거{task ? " " + task.criteria.length + "개" : ""}
        {summaryState && (
          <span className="task-evidence-summary-state muted">{summaryState}</span>
        )}
      </summary>
      <p className={freshness === "current" ? "muted" : "warning"} data-testid="task-evidence-freshness">
        {FRESHNESS_LABEL[freshness]}
      </p>
      {snapshot == null ? (
        <p className="muted" data-testid="task-evidence-unsupported">기준·증거 정보가 제공되지 않았습니다.</p>
      ) : snapshot.unavailable ? (
        <Unavailable reason={snapshot.unavailable} />
      ) : !task ? (
        <p className="muted" data-testid="task-evidence-missing">
          {snapshot.truncated ? "이 작업은 현재 조회 범위에 포함되지 않았습니다." : "이 작업의 기준·증거 항목이 반환되지 않았습니다."}
        </p>
      ) : (
        <TaskCriteria task={task} snapshot={snapshot} freshness={freshness} {...actions} />
      )}
    </details>
  );
}
