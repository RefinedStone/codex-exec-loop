import type { PlanningEvidenceFreshness } from "./kernelConnection";
import type { PlanningCriterionEvidenceState, PlanningState, TaskEvidenceTaskState, TaskVerificationCurrentState, TaskVerificationExecutionOutcome, TaskVerificationNotRunReason } from "./protocol";
import { verificationBlockReason, verificationTarget, verificationTargetKey, type TaskVerificationRequests, type TaskVerificationTarget } from "./taskVerificationActions";

export interface TaskVerificationActions {
  verificationRequests?: TaskVerificationRequests;
  onRequestVerification?(target: TaskVerificationTarget): Promise<void>;
}

const HOST_LABEL: Record<TaskVerificationCurrentState, string> = {
  pending: "호스트 검증 대기", running: "호스트 검증 중", passed: "호스트 검증 통과", failed: "호스트 검증 실패",
  not_run: "호스트 검증 미실행", stale: "이전 범위의 호스트 검증", unavailable: "호스트 검증 확인 불가",
};
const NOT_RUN_LABEL: Record<TaskVerificationNotRunReason, string> = {
  unavailable: "실행 환경을 사용할 수 없음", invalid_input: "검증 입력이 올바르지 않음", stale: "조회 기준 변경",
  scope_changed: "완료 기준 범위 변경", lease_changed: "실행 작업의 소유권 변경", configuration_changed: "검증 설정 변경",
  spawn_failed: "검증 프로세스를 시작하지 못함", cancelled: "실행 전 취소",
};
function outcomeLabel(outcome: TaskVerificationExecutionOutcome): string {
  switch (outcome.kind) {
    case "exited": return `종료 코드 ${outcome.exit_code}`;
    case "timed_out": return "시간 초과";
    case "cancelled": return "실행 취소";
    case "output_limit": return "출력 한도 초과";
    case "signalled": return "프로세스 강제 종료";
    case "io_failure": return "입출력 오류";
  }
}

export function TaskVerificationView({ entry, task, snapshot, freshness, verificationRequests, onRequestVerification }: {
  entry: PlanningCriterionEvidenceState;
  task: TaskEvidenceTaskState;
  snapshot: NonNullable<PlanningState["task_evidence"]>;
  freshness: PlanningEvidenceFreshness;
} & TaskVerificationActions) {
  const target = verificationTarget(task, entry.criterion.id);
  const request = verificationRequests?.[verificationTargetKey(target)];
  const host = entry.host_verification;
  const fact = host?.receipt?.verification;
  const current = freshness === "current" && !snapshot.unavailable && !task.unavailable;
  const state = host?.state ?? (entry.criterion.verification_recipe ? "pending" : null);
  const interrupted = state === "not_run" && fact?.result.kind === "interrupted";
  const label = state ? (interrupted ? "호스트 검증 중단" : HOST_LABEL[state]) : "호스트 검증 미설정";
  const blocked = verificationBlockReason(snapshot, freshness, target, request);
  const executed = fact?.result.kind === "executed" ? fact.result.observation : null;
  const tone = !current || state === "stale" ? "work-badge muted"
    : state === "passed" ? "badge badge-ok" : state === "failed" ? "badge badge-bad"
      : state === "unavailable" || state === "not_run" ? "badge badge-warn" : "work-badge";
  return (
    <section className="task-evidence-record" aria-label="호스트 검증">
      <div className="work-meta">
        <span className={tone} data-testid="task-host-status" data-state={state ?? "not_configured"}
          data-current={current && state != null && state !== "stale" && state !== "unavailable"}>
          {!current ? "마지막 조회: " : ""}{label}
        </span>
        {onRequestVerification && entry.criterion.verification_recipe && (
          <button type="button" data-testid="task-host-request" disabled={blocked !== null} title={blocked ?? undefined}
            onClick={() => { void onRequestVerification(target); }}>
            {request?.status === "sending" ? "요청 중…" : request?.status === "unknown" ? "같은 요청 다시 확인" : "호스트 검증 요청"}
          </button>
        )}
      </div>
      {(entry.criterion.verification_recipe === "git_diff_check_v1" || host?.attempt?.recipe === "git_diff_check_v1") && (
        <p className="muted">Git 변경의 공백 오류를 검사합니다. 빌드·기능 테스트의 통과 여부는 별도로 확인해야 합니다.</p>
      )}
      {entry.criterion.verification_recipe === "repo_git_grep_v1" && (
        <p className="muted">호스트 확인 창에서 명령, 작업 경로와 소스를 확인해야 선언된 문자열 검사를 한 번 실행합니다.</p>
      )}
      {request && <p className={request.status === "unknown" || request.status === "rejected" ? "warning" : "muted"}
        role="status" data-testid="task-host-request-status">{request.message}</p>}
      {host?.unavailable && <p className="warning">현재 호스트 검증을 확인할 수 없습니다. <code>({host.unavailable})</code></p>}
      {fact?.result.kind === "not_run" && <p className="muted">{NOT_RUN_LABEL[fact.result.reason]}</p>}
      {interrupted && <p className="muted">실행 결과를 확정하기 전에 중단되었습니다.</p>}
      {host?.attempt && (
        <details className="task-evidence-record">
          <summary>{current && state !== "stale" ? "호스트 검증 정보" : "이전 호스트 검증 정보"}</summary>
          <dl className="task-evidence-metadata">
            <dt>검증</dt><dd>{host.attempt.recipe}</dd>
            <dt>시도</dt><dd>{host.attempt.attempt}회</dd>
            <dt>시도 ID</dt><dd><code>{host.attempt.attempt_id}</code></dd>
            <dt>기준 판</dt><dd>{host.attempt.scope.acceptance_revision}</dd>
            <dt>범위 ID</dt><dd><code>{host.attempt.scope.scope_id}</code></dd>
            <dt>요청 시각</dt><dd><time dateTime={host.attempt.requested_at}>{host.attempt.requested_at}</time></dd>
            {executed && <>
              <dt>실행 결과</dt><dd>{outcomeLabel(executed.outcome)}</dd>
              <dt>소요 시간</dt><dd>{executed.duration_ms} ms</dd>
              <dt>표준 출력</dt><dd>{executed.stdout.size_bytes} 바이트 · {executed.stdout.complete ? "수집 완료" : "불완전"} · <code>{executed.stdout.sha256}</code></dd>
              <dt>표준 오류</dt><dd>{executed.stderr.size_bytes} 바이트 · {executed.stderr.complete ? "수집 완료" : "불완전"} · <code>{executed.stderr.sha256}</code></dd>
            </>}
            {fact?.finished_at && <><dt>종료 시각</dt><dd><time dateTime={fact.finished_at}>{fact.finished_at}</time></dd></>}
            {fact?.manifest && <>
              <dt>기준 커밋</dt><dd><code>{fact.manifest.base_commit_sha}</code></dd>
              <dt>검증한 커밋</dt><dd><code>{fact.manifest.source.head_commit_sha}</code></dd>
              <dt>검증한 파일 트리</dt><dd><code>{fact.manifest.source.tree_sha}</code></dd>
            </>}
            {fact?.artifact && <>
              <dt>증거 SHA-256</dt><dd><code>{fact.artifact.sha256}</code></dd>
              <dt>증거 크기</dt><dd>{fact.artifact.size_bytes} 바이트</dd>
            </>}
          </dl>
        </details>
      )}
    </section>
  );
}
