/** 추가된 조작 DTO의 모양만 확인한다. 권한과 실행 조건은 서버가 정한다. */
import { isIssueArbiterWaitReason } from "./issueArbiter";
type ObjectValue = Record<string, unknown>;
const record = (value: unknown): value is ObjectValue => value !== null && typeof value === "object" && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const optionalText = (value: unknown): boolean => value === undefined || value === null || text(value);
const revision = (value: unknown): boolean => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const named = (value: unknown, names: readonly string[]): boolean => text(value) && names.includes(value);
const levels = ["off", "observe", "propose", "gated", "auto"];
const actions = ["hold_issue", "release_issue", "steer_issue", "take_over_issue", "retriage_issue", "cancel_issue_work", "track_issue", "untrack_issue"];
const states = ["open", "answered", "expired", "withdrawn", "superseded"];
const executionTerminalClasses = ["rework_needed", "observe_again", "contract_recovery", "identity_conflict", "pull_request_closed", "queue_recovery"];
const executionRecoveryReasons = ["attempts_exhausted", ...executionTerminalClasses];

function sameCoordinates(left: unknown, right: ObjectValue): boolean {
  if (!record(left) || !record(left.project_scope) || !record(right.project_scope)) return false;
  return ["provider", "server_id", "repository_id", "issue_id"].every((key) => text(left[key]) && left[key] === right[key])
    && left.project_scope.workspace_dir === right.project_scope.workspace_dir;
}

function decision(value: unknown, issue: ObjectValue): boolean {
  if (!record(value) || !text(value.id) || !value.id || !text(value.question) || !revision(value.revision)
    || !named(value.state, states) || !record(value.subject) || !sameCoordinates(value.subject.issue, issue)
    || !Array.isArray(value.options) || !optionalText(value.recommended) || !deliveryReview(value)) return false;
  if (value.arbiter_wait != null && (!record(value.arbiter_wait)
    || Object.keys(value.arbiter_wait).some((key) => key !== "reason")
    || !isIssueArbiterWaitReason(value.arbiter_wait.reason)
    || value.state !== "open" || value.on_timeout !== "arbiter")) return false;
  const ids = new Set<string>();
  for (const option of value.options) {
    if (!record(option) || !text(option.id) || !option.id || ids.has(option.id) || !text(option.label) || !text(option.effect)) return false;
    ids.add(option.id);
  }
  return ids.size > 0 && (value.recommended == null || ids.has(value.recommended as string));
}

function deliveryReview(value: ObjectValue): boolean {
  const basis = record(value.basis) ? value.basis : null;
  const delivery = basis?.delivery;
  const review = value.delivery_review;
  const workBound = value.kind === "risk_exception" && value.gate === "delivery"
    && typeof basis?.work_order_id === "string" && basis.work_order_id.length > 0;
  if (!workBound) return delivery == null && review == null;
  if (!record(delivery) || !record(review) || value.on_timeout !== "wait") return false;
  const { base_commit_sha: base, source_commit_sha: source } = delivery;
  const sha = (value: unknown): value is string => typeof value === "string" && /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(value);
  if (!sha(base) || !sha(source) || base.length !== source.length || base === source
    || !revision(review.changed_files) || (review.changed_files as number) < 1 || (review.changed_files as number) > 4_096
    || !Array.isArray(review.reasons) || review.reasons.length === 0
    || !review.reasons.every((reason: unknown) => named(reason, ["protected_path", "diff_too_large", "binary_diff"]))
    || new Set(review.reasons).size !== review.reasons.length) return false;
  return review.reasons.includes("binary_diff") ? review.changed_lines === null : revision(review.changed_lines);
}

export function hasValidIssueInteractions(value: ObjectValue, issue: ObjectValue): boolean {
  if (value.as_of_unix_ms != null && (!revision(value.as_of_unix_ms) || (value.as_of_unix_ms as number) > 8_640_000_000_000_000)) return false;
  if (value.open_decisions != null && (!Array.isArray(value.open_decisions) || !value.open_decisions.every((item: unknown) => decision(item, issue)))) return false;
  if (value.control != null) {
    if (!record(value.control) || !["tracked", "held", "taken_over", "cancelled"].every((key) => typeof (value.control as ObjectValue)[key] === "boolean")
      || !revision(value.control.authorization_epoch)) return false;
  }
  if (value.actions != null && (!Array.isArray(value.actions) || !value.actions.every((item: unknown) => record(item)
    && named(item.command, actions) && typeof item.enabled === "boolean" && optionalText(item.reason)))) return false;
  if (value.decision_history != null && (!Array.isArray(value.decision_history) || !value.decision_history.every((item: unknown) => {
    if (!record(item) || !record(item.decision) || !text(item.decision.decision_id) || !named(item.decision.state, states)
      || !text(item.decision.kind) || !record(item.decision.subject) || !sameCoordinates(item.decision.subject.issue, issue)) return false;
    if (item.decision.answer_by != null && !named(item.decision.answer_by, ["human", "agent", "default", "arbiter"])) return false;
    if (item.request != null && (!decision(item.request, issue) || (item.request as ObjectValue).id !== item.decision.decision_id)) return false;
    return item.resolution_reason == null || (record(item.resolution_reason)
      && named(item.resolution_reason.code, ["answered", "expired", "withdrawn", "basis_changed"]) && text(item.resolution_reason.summary));
  }))) return false;
  const target = value.conversation;
  if (target == null) return true;
  if (!record(target)) return false;
  if (target.kind === "thread") return text(target.thread_id) && !!target.thread_id && !/[\s\p{Cc}]/u.test(target.thread_id);
  if (target.kind === "draft") return record(target.shared_copy) && text(target.shared_copy.title) && text(target.shared_copy.body);
  return target.kind === "unavailable" && text(target.reason);
}

export function hasValidIssueAutonomy(value: ObjectValue): boolean {
  const autonomy = value.autonomy;
  if (autonomy == null) return true;
  if (!record(autonomy)) return false;
  if (autonomy.status !== "ready") return named(autonomy.status, ["policy_blocked", "unavailable"]);
  if ((autonomy.requested_level != null && !named(autonomy.requested_level, levels))
    || !named(autonomy.effective_level, levels) || !named(autonomy.cap, levels) || !revision(autonomy.revision)
    || !text(autonomy.policy_version) || typeof autonomy.pending !== "boolean" || !Array.isArray(autonomy.choices)) return false;
  const seen = new Set<string>();
  return autonomy.choices.every((choice: unknown) => {
    if (!record(choice) || !named(choice.level, levels) || seen.has(choice.level as string)
      || typeof choice.enabled !== "boolean" || typeof choice.requires_confirmation !== "boolean" || !optionalText(choice.reason)) return false;
    seen.add(choice.level as string);
    return true;
  }) && ["off", "observe", "propose"].every((level) => seen.has(level))
    && (autonomy.requested_level == null || seen.has(autonomy.requested_level as string));
}

/** 실행·게시·timeline은 서버가 정한 값의 모양만 확인하고 서로의 상태를 추론하지 않는다. */
export function hasValidIssueExecutionTracking(value: ObjectValue): boolean {
  const unsignedCount = (count: unknown) => revision(count) && (count as number) <= 0xffff_ffff;
  const nullableText = (entry: unknown) => entry === null || text(entry);
  const execution = value.execution;
  if (execution !== undefined) {
    if (!record(execution)) return false;
    if (execution.status === "unavailable") {
      if (!named(execution.reason, ["read_failed", "work_id_mismatch"])) return false;
    } else {
      if (execution.status !== "ready" || !text(execution.work_order_id) || !execution.work_order_id
        || !revision(execution.revision) || !named(execution.state, ["queued", "running", "in_review", "done", "failed"])
        || !text(execution.state_label) || !unsignedCount(execution.execution_attempts)
        || !(execution.execution_attempt_limit === null || unsignedCount(execution.execution_attempt_limit))
        || !unsignedCount(execution.verification_attempts) || !nullableText(execution.review_pull_request_url)
        || !(execution.terminal_class === null || named(execution.terminal_class, executionTerminalClasses))) return false;
      const recovery = execution.recovery;
      if (recovery !== null && (!record(recovery) || !named(recovery.reason, executionRecoveryReasons)
        || !text(recovery.summary) || !text(recovery.next_summary) || typeof recovery.requires_human !== "boolean")) return false;
    }
  }
  const feedback = value.feedback;
  if (feedback != null) {
    if (!record(feedback) || !revision(feedback.revision) || !unsignedCount(feedback.publication_attempts)
      || !optionalText(feedback.comment_id)
      || (feedback.external_state !== undefined && !named(feedback.external_state, ["open", "closed", "unknown"]))
      || (feedback.reason != null && feedback.reason !== "feedback_unconfirmed")) return false;
    const integration = feedback.integration;
    if (integration != null && (!record(integration) || !text(integration.record_id)
      || !text(integration.source_sha) || !text(integration.integration_sha) || !optionalText(integration.pull_request_url))) return false;
  }
  const timeline = value.timeline;
  if (timeline !== undefined) {
    if (!record(timeline)) return false;
    if (timeline.status === "unavailable") return true;
    if (timeline.status !== "ready" || typeof timeline.has_older !== "boolean"
      || !Array.isArray(timeline.entries) || timeline.entries.length > 20) return false;
    let sequence = -1;
    for (const entry of timeline.entries) {
      if (!record(entry) || !revision(entry.sequence) || (entry.sequence as number) <= sequence
        || !text(entry.recorded_at) || !text(entry.kind) || !text(entry.summary)) return false;
      sequence = entry.sequence as number;
    }
  }
  return true;
}
