import type { IssueDecisionFactRecord, IssueDecisionRequestState, IssueDetailAnswer, IssueEventReference } from "../../src/renderer/kernel/protocol";
import type { IssueArbiterFinishedRecord } from "../../src/renderer/kernel/issueArbiter";

export function arbiterRequest(issue: IssueEventReference = {
  provider: "github", server_id: "github.com", repository_id: "R_repo", issue_id: "I_1", project_scope: { workspace_dir: "/repo" },
}): IssueDecisionRequestState {
  return { id: "decision-arbiter", kind: "start_approval", gate: "start", state: "open", revision: 3,
    subject: { kind: "issue", issue }, basis: { policy_version: "a".repeat(64), gate_revision: 1 },
    question: "공유된 계획으로 시작할까요?", options: [{ id: "approve", label: "시작 승인", effect: "현재 계획으로 시작합니다." }],
    recommended: "approve", on_timeout: "arbiter", due_at: "2026-11-23T09:00:00Z" };
}

export function arbiterFact(request = arbiterRequest()): IssueDecisionFactRecord {
  return { decision_id: request.id, subject: request.subject, kind: request.kind, gate: request.gate,
    basis: request.basis, state: request.state, answer_by: null, option: null };
}

export function arbiterEvent(request = arbiterRequest(), revision = 8): IssueArbiterFinishedRecord {
  return { kind: "issue_decision_arbiter_finished", event_id: "arbiter-event-1", decision: arbiterFact(request),
    outcome: { kind: "waiting", reason: "low_confidence" }, at: "2026-11-23T09:00:01Z", revision };
}

export function arbiterDetail(request = arbiterRequest(), revision = 8): Extract<IssueDetailAnswer, { status: "ready" }> {
  return { type: "issue", status: "ready", revision,
    item: { issue: request.subject.issue, binding_id: "binding", current: { status: "not_yet_observed" },
      presentation: { number: null, title: null, url: null, status: { code: "awaiting_decision", label: "결정 대기" },
        reason: { code: "workflow", summary: "사람의 답변을 기다립니다." } } },
    open_decisions: [request], decision_history: [], actions: [], conversation: { kind: "unavailable", reason: "아직 대화가 없습니다." } };
}
