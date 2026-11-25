import { useEffect, useRef, useState } from "react";
import { TaskEvidenceView } from "./TaskEvidenceView";
import type { PlanningEvidenceFreshness } from "./kernelConnection";
import type { TaskAuthoringController, TaskResultAnswer } from "./taskAuthoring";
import "./taskResult.css";

const STATUS = { ready: "대기", proposed: "제안", awaiting_user: "사용자 응답 대기", in_progress: "진행 중", blocked: "막힘", done: "완료", cancelled: "취소" };

export function TaskResultView({ workspace, taskId, revision, controller, onOpenTask }: {
  workspace: string;
  taskId: string;
  revision: number | null | undefined;
  controller: TaskAuthoringController;
  onOpenTask?: (taskId: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [reload, setReload] = useState(0);
  const [snapshot, setSnapshot] = useState<{ scope: string; answer: TaskResultAnswer } | null>(null);
  const scope = controller.resultScope(workspace);
  const [freshness, setFreshness] = useState<PlanningEvidenceFreshness>("refreshing");
  const [error, setError] = useState<string | null>(null);
  const sequence = useRef(0);
  useEffect(() => {
    const current = ++sequence.current;
    if (!open) return;
    if (!scope) { setFreshness("stale"); return; }
    setFreshness("refreshing");
    setError(null);
    void controller.readResult(workspace, taskId).then((answer) => {
      if (current !== sequence.current) return;
      setSnapshot({ scope, answer });
      setFreshness("current");
    }).catch((failure: unknown) => {
      if (current !== sequence.current) return;
      setFreshness("stale");
      setError(failure instanceof Error ? failure.message : String(failure));
    });
    return () => { sequence.current++; };
  }, [controller, workspace, taskId, revision, open, reload, scope]);
  const result = scope && snapshot?.scope === scope && snapshot.answer.task_id === taskId ? snapshot.answer : null;
  const task = result?.task;
  const evidenceTask = result?.evidence.unavailable ? undefined
    : result?.evidence.tasks.find((entry) => entry.scope.task_id === taskId);
  const evidenceKnown = freshness === "current" && !!evidenceTask && !evidenceTask.unavailable;
  const criteria = evidenceTask?.criteria ?? [];
  const hostPass = criteria.filter((entry) => entry.host_verification?.state === "passed" && !entry.host_verification.unavailable).length;
  const hostFail = criteria.filter((entry) => entry.host_verification?.state === "failed" && !entry.host_verification.unavailable).length;
  const workerPass = criteria.filter((entry) => entry.state === "pass").length;
  const waived = criteria.filter((entry) => entry.state === "waived").length;
  const ownPrs = result?.pull_requests.filter((pr) => pr.relation === "source") ?? [];

  return <details className="task-result" open={open} onToggle={(event) => setOpen(event.currentTarget.open)} data-testid="task-result" data-task-id={taskId}>
    <summary>실행 · 검증 · PR 결과</summary>
    {open && <>
      <div className="task-result-heading">
        <code>{taskId}</code>
        <button type="button" onClick={() => setReload((value) => value + 1)} disabled={!scope || freshness === "refreshing"} data-testid="task-result-refresh">결과 다시 읽기</button>
      </div>
      <p className={freshness === "current" ? "muted" : "warning"}>
        {!scope ? "이 저장소 커널에 연결한 뒤 결과를 다시 읽으세요."
          : freshness === "current" ? "저장소에서 마지막으로 읽은 사실입니다. 각 상태는 별도로 표시합니다."
          : freshness === "refreshing" ? "결과를 읽는 중입니다. 이전 조회 내용은 현재 판정으로 사용하지 않습니다."
            : "현재 상태를 확인하지 못했습니다. 남아 있는 내용은 마지막 조회입니다."}
      </p>
      {error && <p role="alert" className="error">{error}</p>}
      {result?.status === "missing" && <p className="warning">현재 작업 원본에서 이 ID를 찾지 못했습니다.</p>}
      {result?.status === "unavailable" && <p className="warning">작업 원본을 읽을 수 없습니다.</p>}
      {task && result?.status === "available" && <>
        <dl className="task-result-facts" data-testid="task-result-facts" data-current={freshness === "current"}>
          <dt>작업 상태</dt><dd>{STATUS[task.fields.status ?? "awaiting_user"]}</dd>
          <dt>호스트 검증</dt><dd>{evidenceKnown ? "통과 " + hostPass + "개 · 실패 " + hostFail + "개 / 전체 기준 " + criteria.length + "개" : "현재 상태 미확인"}</dd>
          <dt>작업자 보고</dt><dd>{evidenceKnown ? "통과 보고 " + workerPass + "개" : "현재 상태 미확인"}</dd>
          <dt>사람 면제</dt><dd>{evidenceKnown ? waived + "개" : "현재 상태 미확인"}</dd>
          <dt>이 작업의 PR 머지</dt><dd>{result.pr_lineage_complete
            ? new Set(ownPrs.filter((pr) => pr.merge_sha).map((pr) => pr.url)).size + "개에 머지 SHA가 기록됨"
            : "PR 연결을 모두 확인하지 못함"}</dd>
          <dt>외부 이슈 종료</dt><dd>확인되지 않음</dd>
        </dl>
        <TaskEvidenceView taskId={taskId} snapshot={result.evidence} freshness={freshness} />
        {!result.lineage_available && <p className="warning">작업 계보를 읽지 못했습니다.</p>}
        {result.work_order && <section>
          <h3>연결된 작업</h3>
          <p>Work order: <code>{result.work_order.work_order_id}</code></p>
          {result.work_order.external_reference_url && <p className="task-result-url">외부 참조: {result.work_order.external_reference_url}</p>}
          <p className="muted">외부 이슈의 실제 종료 여부는 이 조회에 포함되지 않습니다.</p>
          {result.work_order.related_task_ids.length > 0 && <ul>{result.work_order.related_task_ids.map((id) =>
            <li key={id}><button type="button" disabled={!onOpenTask} onClick={() => onOpenTask?.(id)}>{id} 열기</button></li>)}</ul>}
        </section>}
        <section>
          <h3>실행과 전달</h3>
          {!result.runtime_available ? <p className="warning">실행·전달 기록을 읽지 못했습니다.</p> : <>
            {!result.sessions.length && <p className="muted">보존된 실행 기록이 없습니다.</p>}
            {result.dispatch_blocks.map((block, index) => <p className="warning" key={index}>실행 차단: {block.reason} · {block.blocked_at}</p>)}
            {result.sessions.map((session) => <details key={session.session_key}>
              <summary>{session.agent_id} · {session.state} · {session.completion_state}</summary>
              <dl className="task-result-facts">
                <dt>실행 ID</dt><dd><code>{session.session_key}</code></dd>
                <dt>브랜치</dt><dd>{session.branch}</dd>
                <dt>스레드</dt><dd>{session.thread_id ?? "기록 없음"}</dd>
                <dt>작업자 검증 보고</dt><dd>{session.worker_validation_summary || "보고 없음"}</dd>
                <dt>전달 결과</dt><dd>{session.delivery_outcome ?? "기록 없음"}</dd>
                <dt>갱신</dt><dd>{session.updated_at}</dd>
              </dl>
            </details>)}
            {!result.deliveries.length && <p className="muted">현재 보존된 전달 대기 기록이 없습니다. 완료된 PR 기록은 아래에서 별도로 확인합니다.</p>}
            {result.deliveries.map((delivery) => <details key={delivery.queue_item_id} data-testid="task-result-delivery">
              <summary>전달 {delivery.state} · {delivery.integration_state || "통합 상태 미제공"}</summary>
              <dl className="task-result-facts">
                <dt>전달 ID</dt><dd><code>{delivery.queue_item_id}</code></dd>
                <dt>브랜치</dt><dd>{delivery.branch}</dd>
                <dt>원본 커밋</dt><dd><code>{delivery.source_commit_sha || "미제공"}</code></dd>
                <dt>통합 커밋</dt><dd><code>{delivery.integration_commit_sha ?? "미제공"}</code></dd>
                <dt>대기 단계</dt><dd>{delivery.evidence_wait ?? "기록 없음"}</dd>
              </dl>
              {delivery.note && <p>{delivery.note}</p>}
              {delivery.pull_request_url && <button type="button" onClick={() => window.akra.github.openExternal(delivery.pull_request_url!)}>전달 PR #{delivery.pull_request_number ?? "?"} 열기</button>}
              {!delivery.evidence_available && <p className="warning">전달 증거의 작업 ID가 일치하지 않아 표시하지 않았습니다.</p>}
              {delivery.evidence_observations.map((observation) => <div key={observation.stage}>
                <p>증거 관찰: {observation.stage} · {observation.mode} · {observation.observed_at}</p>
                {observation.reasons.length > 0 && <p className="warning">차단 이유: {observation.reasons.join(", ")}</p>}
                <ul>{observation.criteria.map((criterion) => <li key={criterion.criterion_id}>{criterion.criterion_id}: {criterion.state}{criterion.reason ? " · " + criterion.reason : ""}</li>)}</ul>
              </div>)}
            </details>)}
          </>}
        </section>
        <section>
          <h3>PR 기록</h3>
          {!result.pr_lineage_complete && <p className="warning" data-testid="task-result-lineage-unknown">일부 PR의 작업 계보를 확인하지 못했습니다. PR이 없다고 판정할 수 없습니다.</p>}
          {!result.pull_requests.length && result.pr_lineage_complete && <p className="muted" data-testid="task-result-no-pr">이 작업 ID에 연결된 PR 기록이 없습니다.</p>}
          {result.pull_requests.map((pr) => <article key={pr.record_id} data-testid="task-result-pr" className="task-result-pr">
            <h4><button type="button" onClick={() => window.akra.github.openExternal(pr.url)}>PR #{pr.number} 열기</button></h4>
            <p>{pr.relation === "source" ? "이 작업에서 전달한 PR" : "이 작업이 보완하는 PR"} · 원 작업 <code>{pr.source_task_id}</code></p>
            <p>{pr.current === false ? "후속 기록이 있는 이전 관찰" : pr.current == null ? "현재 관찰 여부 미확인" : freshness === "current" ? "현재 관찰" : "마지막 조회의 관찰"} · {pr.state} · {pr.phase}</p>
            <p>{pr.reason}</p>
            {pr.blocker && <p className="warning">차단 이유: {pr.blocker}</p>}
            <p>다음 확인: {pr.next_action}</p>
            <dl className="task-result-facts">
              <dt>PR 머지 사실</dt><dd>{pr.merge_sha ? <code>{pr.merge_sha}</code> : "머지 SHA 미확인"}</dd>
              <dt>통합 방식</dt><dd>{pr.integration_method ?? "기록 없음"}</dd>
              <dt>통합 증거 커밋</dt><dd><code>{pr.integration_evidence_sha ?? "기록 없음"}</code></dd>
              <dt>검증 기록</dt><dd><code>{pr.record_id}</code></dd>
              {pr.successor_record_id && <><dt>후속 기록</dt><dd><code>{pr.successor_record_id}</code></dd></>}
            </dl>
          </article>)}
        </section>
      </>}
    </>}
  </details>;
}
