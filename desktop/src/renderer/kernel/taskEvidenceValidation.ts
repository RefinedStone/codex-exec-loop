/** E2 읽기 metadata만 검증한다. 상태의 판정과 native Human 권한은 서버가 소유한다. */
import type { TaskEvidenceState } from "./protocol";

type ObjectValue = Record<string, unknown>;
const record = (value: unknown): value is ObjectValue => value !== null && typeof value === "object" && !Array.isArray(value);
const keys = (value: ObjectValue, allowed: readonly string[]) => Object.keys(value).every((key) => allowed.includes(key));
const text = (value: unknown): value is string => typeof value === "string";
const identifier = (value: unknown): value is string => text(value) && value.length > 0 && value.length <= 4096 && value.trim() === value;
const boundedId = (value: unknown): value is string => identifier(value) && value.length <= 128 && !/\p{Cc}/u.test(value);
const revision = (value: unknown): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const positive = (value: unknown): value is number => revision(value) && value > 0;
const token = (value: unknown): value is string => text(value) && value.length === 64 && /^[a-f0-9]{64}$/.test(value);
const oid = (value: unknown): boolean => text(value) && (value.length === 40 || value.length === 64) && !/[^a-f0-9]/.test(value);
const named = (value: unknown, names: readonly string[]) => text(value) && names.includes(value);
const timestamp = (value: unknown) => text(value) && Number.isFinite(Date.parse(value));
const errors = ["invalid", "missing", "artifact_missing", "stale", "scope_changed", "lease_changed", "conflict", "unavailable", "limit", "human_confirmation_required"];
const optionalError = (value: unknown) => value == null || named(value, errors);

function scope(value: unknown): value is ObjectValue {
  return record(value) && keys(value, ["task_id", "scope_id", "acceptance_revision", "criteria_sha256"])
    && identifier(value.task_id) && token(value.scope_id) && revision(value.acceptance_revision) && token(value.criteria_sha256);
}

function sameScope(left: ObjectValue, right: ObjectValue): boolean {
  return ["task_id", "scope_id", "acceptance_revision", "criteria_sha256"].every((key) => left[key] === right[key]);
}

function tree(value: unknown): boolean {
  return record(value) && keys(value, ["head_commit_sha", "tree_sha", "worktree_clean"])
    && oid(value.head_commit_sha) && oid(value.tree_sha) && typeof value.worktree_clean === "boolean";
}

function evidenceReceipt(value: unknown): value is ObjectValue & { evidence: ObjectValue & { scope: ObjectValue } } {
  if (!record(value) || !keys(value, ["event_id", "evidence_revision", "evidence"])
    || !boundedId(value.event_id) || !positive(value.evidence_revision) || !record(value.evidence)) return false;
  const evidence = value.evidence;
  return keys(evidence, ["scope", "criterion_id", "attempt_id", "attempt", "status", "artifact", "recorded_tree", "by", "recorded_at"])
    && scope(evidence.scope) && positive(evidence.scope.acceptance_revision)
    && identifier(evidence.criterion_id) && token(evidence.attempt_id)
    && positive(evidence.attempt) && evidence.attempt <= 64
    && named(evidence.status, ["pass", "fail", "blocked"])
    && record(evidence.artifact) && keys(evidence.artifact, ["sha256", "size_bytes"])
    && token(evidence.artifact.sha256) && positive(evidence.artifact.size_bytes) && evidence.artifact.size_bytes <= 8 * 1024 * 1024
    && tree(evidence.recorded_tree) && boundedId(evidence.by) && timestamp(evidence.recorded_at);
}

function waiverReceipt(value: unknown): value is ObjectValue & { waiver: ObjectValue & { scope: ObjectValue } } {
  if (!record(value) || !keys(value, ["event_id", "evidence_revision", "waiver"])
    || !boundedId(value.event_id) || !positive(value.evidence_revision) || !record(value.waiver)) return false;
  const waiver = value.waiver;
  return keys(waiver, ["scope", "criterion_id", "reason_digest", "by", "recorded_at"])
    && scope(waiver.scope) && positive(waiver.scope.acceptance_revision) && identifier(waiver.criterion_id)
    && token(waiver.reason_digest) && waiver.by === "human" && timestamp(waiver.recorded_at);
}

function criterion(value: unknown, task: ObjectValue & { scope: ObjectValue }, evidenceRevision: number): boolean {
  if (!record(value) || !keys(value, ["criterion", "state", "evidence", "waiver"])
    || !record(value.criterion) || !keys(value.criterion, ["id", "scenario", "channel", "essential"])
    || !identifier(value.criterion.id) || !text(value.criterion.scenario) || !value.criterion.scenario.trim()
    || !named(value.criterion.channel, ["command", "data", "http", "screen"])
    || typeof value.criterion.essential !== "boolean"
    || (value.state != null && !named(value.state, ["pending", "pass", "fail", "blocked", "stale", "waived"]))) return false;
  if (value.evidence != null && (!evidenceReceipt(value.evidence)
    || (value.evidence.evidence_revision as number) > evidenceRevision
    || value.evidence.evidence.criterion_id !== value.criterion.id
    || value.evidence.evidence.scope.task_id !== task.scope.task_id)) return false;
  if (value.waiver != null && (!waiverReceipt(value.waiver)
    || (value.waiver.evidence_revision as number) > evidenceRevision
    || value.waiver.waiver.criterion_id !== value.criterion.id
    || value.waiver.waiver.scope.task_id !== task.scope.task_id)) return false;
  if (value.state == null) return task.unavailable != null;
  if (value.state === "waived") return waiverReceipt(value.waiver) && sameScope(value.waiver.waiver.scope, task.scope);
  if (value.state === "pending") return value.evidence == null;
  if (value.state === "stale") return value.evidence != null || value.waiver != null;
  return record(task.source) && record(task.source.tree) && task.unavailable == null && evidenceReceipt(value.evidence)
    && record(value.evidence.evidence.recorded_tree)
    && value.evidence.evidence.recorded_tree.tree_sha === task.source.tree.tree_sha
    && sameScope(value.evidence.evidence.scope, task.scope) && value.state === value.evidence.evidence.status;
}

/** 오래된 scope의 영수증은 Stale로 남을 수 있다. 현재 task scope로 덮어써서 검증하지 않는다. */
export function isTaskEvidenceState(value: unknown): value is TaskEvidenceState {
  if (!record(value) || !keys(value, ["planning_revision", "evidence_revision", "unavailable", "tasks", "truncated", "next_task_id"])
    || !optionalError(value.unavailable) || !Array.isArray(value.tasks) || value.tasks.length > 256
    || typeof value.truncated !== "boolean" || (value.next_task_id != null && !identifier(value.next_task_id))
    || (value.planning_revision != null && !revision(value.planning_revision))
    || (value.evidence_revision != null && !revision(value.evidence_revision))) return false;
  if (value.unavailable != null) return value.tasks.length === 0 && !value.truncated && value.next_task_id == null;
  if (!revision(value.planning_revision) || !revision(value.evidence_revision)) return false;
  if (value.truncated !== (value.next_task_id != null)) return false;
  const taskIds = new Set<string>();
  for (const task of value.tasks) {
    if (!record(task) || !keys(task, ["scope", "source", "unavailable", "criteria"])
      || !scope(task.scope) || !optionalError(task.unavailable) || !Array.isArray(task.criteria)
      || !identifier(task.scope.task_id) || taskIds.has(task.scope.task_id)) return false;
    taskIds.add(task.scope.task_id);
    if (task.criteria.length > 0 && !positive(task.scope.acceptance_revision)) return false;
    if (task.source != null && (!record(task.source) || !keys(task.source, ["kind", "tree"])
      || !named(task.source.kind, ["running_lease", "queued_source"]) || !tree(task.source.tree))) return false;
    const criterionIds = new Set<string>();
    for (const entry of task.criteria) {
      if (!criterion(entry, task as ObjectValue & { scope: ObjectValue }, value.evidence_revision)
        || !record(entry) || !record(entry.criterion) || !identifier(entry.criterion.id) || criterionIds.has(entry.criterion.id)) return false;
      criterionIds.add(entry.criterion.id);
    }
  }
  return !value.truncated || (value.tasks.length > 0 && value.next_task_id === value.tasks.at(-1)?.scope.task_id);
}

export function hasValidPlanningEvidence(value: ObjectValue): boolean {
  if (value.task_evidence == null) return true;
  return isTaskEvidenceState(value.task_evidence)
    && (value.task_evidence.unavailable != null || value.revision == null || value.revision === value.task_evidence.planning_revision);
}
