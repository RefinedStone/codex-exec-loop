/** 추가된 조작 DTO의 모양만 확인한다. 권한과 실행 조건은 서버가 정한다. */
type ObjectValue = Record<string, unknown>;
const record = (value: unknown): value is ObjectValue => value !== null && typeof value === "object" && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const optionalText = (value: unknown): boolean => value === undefined || value === null || text(value);
const revision = (value: unknown): boolean => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const named = (value: unknown, names: readonly string[]): boolean => text(value) && names.includes(value);
const levels = ["off", "observe", "propose", "gated", "auto"];
const actions = ["hold_issue", "release_issue", "steer_issue", "take_over_issue", "retriage_issue", "cancel_issue_work", "track_issue", "untrack_issue"];
const states = ["open", "answered", "expired", "withdrawn", "superseded"];

function sameCoordinates(left: unknown, right: ObjectValue): boolean {
  if (!record(left) || !record(left.project_scope) || !record(right.project_scope)) return false;
  return ["provider", "server_id", "repository_id", "issue_id"].every((key) => text(left[key]) && left[key] === right[key])
    && left.project_scope.workspace_dir === right.project_scope.workspace_dir;
}

function decision(value: unknown, issue: ObjectValue): boolean {
  if (!record(value) || !text(value.id) || !value.id || !text(value.question) || !revision(value.revision)
    || !named(value.state, states) || !record(value.subject) || !sameCoordinates(value.subject.issue, issue)
    || !Array.isArray(value.options) || !optionalText(value.recommended)) return false;
  const ids = new Set<string>();
  for (const option of value.options) {
    if (!record(option) || !text(option.id) || !option.id || ids.has(option.id) || !text(option.label) || !text(option.effect)) return false;
    ids.add(option.id);
  }
  return ids.size > 0 && (value.recommended == null || ids.has(value.recommended as string));
}

export function hasValidIssueInteractions(value: ObjectValue, issue: ObjectValue): boolean {
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
  if (autonomy.status !== "ready") return named(autonomy.status, ["policy_blocked", "unavailable"]) && text(autonomy.reason);
  if ((autonomy.requested_level !== null && !named(autonomy.requested_level, ["off", "observe", "propose"]))
    || !named(autonomy.effective_level, levels) || !named(autonomy.cap, levels) || !revision(autonomy.revision)
    || !text(autonomy.policy_version) || typeof autonomy.pending !== "boolean" || !Array.isArray(autonomy.choices)) return false;
  const seen = new Set<string>();
  return autonomy.choices.every((choice: unknown) => {
    if (!record(choice) || !named(choice.level, ["off", "observe", "propose"]) || seen.has(choice.level as string)
      || typeof choice.enabled !== "boolean" || typeof choice.requires_confirmation !== "boolean" || !optionalText(choice.reason)) return false;
    seen.add(choice.level as string);
    return true;
  }) && seen.size === 3;
}
