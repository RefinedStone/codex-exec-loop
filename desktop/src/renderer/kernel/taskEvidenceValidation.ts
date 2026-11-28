/** 읽기 metadata만 검증한다. 작업자 보고와 호스트 실행을 분리하며 현재 판정은 서버가 소유한다. */
import type { TaskEvidenceState } from "./protocol";
import { isTaskVerificationRecipe } from "./taskVerificationRecipe";

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

type VerificationAttempt = ObjectValue & { scope: ObjectValue };
type VerificationFact = ObjectValue & { attempt: VerificationAttempt; result: ObjectValue };

function verificationAttempt(value: unknown): value is VerificationAttempt {
  return record(value) && keys(value, ["attempt_id", "scope", "criterion_id", "recipe", "attempt", "requested_at"])
    && token(value.attempt_id) && scope(value.scope) && positive(value.scope.acceptance_revision)
    && identifier(value.criterion_id) && isTaskVerificationRecipe(value.recipe)
    && positive(value.attempt) && value.attempt <= 64 && timestamp(value.requested_at);
}

function sameVerificationAttempt(left: VerificationAttempt, right: VerificationAttempt): boolean {
  return sameScope(left.scope, right.scope)
    && ["attempt_id", "criterion_id", "recipe", "attempt", "requested_at"].every((key) => left[key] === right[key]);
}

function verificationManifest(value: unknown): value is ObjectValue & { source: ObjectValue } {
  if (!record(value) || !keys(value, ["recipe", "base_commit_sha", "base_tree_sha", "source", "git_binary_sha256", "command_sha256", "environment_sha256", "configuration_sha256"])
    || !isTaskVerificationRecipe(value.recipe) || !oid(value.base_commit_sha) || !oid(value.base_tree_sha)
    || !record(value.source) || !tree(value.source)
    || ![value.git_binary_sha256, value.command_sha256, value.environment_sha256, value.configuration_sha256].every(token)) return false;
  const width = (value.base_commit_sha as string).length;
  return [value.base_tree_sha, value.source.head_commit_sha, value.source.tree_sha].every((value) => text(value) && value.length === width);
}

function verificationStream(value: unknown): boolean {
  return record(value) && keys(value, ["sha256", "size_bytes", "complete"])
    && token(value.sha256) && revision(value.size_bytes) && value.size_bytes <= 1024 * 1024
    && (value.size_bytes !== 0 || value.sha256 === "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    && typeof value.complete === "boolean";
}

function verificationResult(value: unknown): value is ObjectValue {
  if (!record(value)) return false;
  if (value.kind === "interrupted") return keys(value, ["kind"]);
  if (value.kind === "not_run") return keys(value, ["kind", "reason"])
    && named(value.reason, ["unavailable", "invalid_input", "stale", "scope_changed", "lease_changed", "configuration_changed", "spawn_failed", "cancelled"]);
  if (value.kind !== "executed" || !keys(value, ["kind", "observation"]) || !record(value.observation)) return false;
  const observation = value.observation;
  if (!keys(observation, ["outcome", "started_at", "finished_at", "duration_ms", "stdout", "stderr"])
    || !timestamp(observation.started_at) || !timestamp(observation.finished_at) || !revision(observation.duration_ms)
    || !verificationStream(observation.stdout) || !verificationStream(observation.stderr) || !record(observation.outcome)) return false;
  // UTC는 역행할 수 있다. duration은 호스트가 monotonic clock으로 관측한 별도 값이다.
  const outcome = observation.outcome;
  return outcome.kind === "exited"
    ? keys(outcome, ["kind", "exit_code"]) && typeof outcome.exit_code === "number"
      && Number.isInteger(outcome.exit_code) && outcome.exit_code >= -2147483648 && outcome.exit_code <= 2147483647
    : keys(outcome, ["kind"]) && named(outcome.kind, ["timed_out", "cancelled", "output_limit", "signalled", "io_failure"]);
}

function hostArtifact(value: unknown): boolean {
  return record(value) && keys(value, ["sha256", "size_bytes"]) && token(value.sha256)
    && positive(value.size_bytes) && value.size_bytes <= 8 * 1024 * 1024;
}

function verificationReceipt(value: unknown, evidenceRevision: number): value is ObjectValue & { verification: VerificationFact } {
  if (!record(value) || !keys(value, ["event_id", "evidence_revision", "verification"])
    || !positive(value.evidence_revision) || value.evidence_revision > evidenceRevision || !record(value.verification)) return false;
  const verification = value.verification;
  if (!keys(verification, ["attempt", "manifest", "result", "artifact", "finished_at"])
    || !verificationAttempt(verification.attempt) || value.event_id !== `task-verification-finished:${verification.attempt.attempt_id}`
    || !verificationResult(verification.result) || !timestamp(verification.finished_at)
    || (verification.artifact != null && !hostArtifact(verification.artifact))) return false;
  if (verification.manifest != null && (!verificationManifest(verification.manifest)
    || verification.manifest.recipe !== verification.attempt.recipe)) return false;
  return verification.result.kind === "executed"
    ? verificationManifest(verification.manifest)
    : verification.artifact == null;
}

function executionPassed(result: ObjectValue): boolean {
  return result.kind === "executed" && record(result.observation) && record(result.observation.outcome)
    && result.observation.outcome.kind === "exited" && result.observation.outcome.exit_code === 0
    && record(result.observation.stdout) && result.observation.stdout.complete === true
    && record(result.observation.stderr) && result.observation.stderr.complete === true;
}

function hostVerification(value: unknown, task: ObjectValue & { scope: ObjectValue }, criterion: ObjectValue, evidenceRevision: number): boolean {
  if (!record(value) || !keys(value, ["attempt", "phase", "receipt", "state", "unavailable"])
    || !named(value.phase, ["pending", "running", "finished"])
    || !named(value.state, ["pending", "running", "passed", "failed", "not_run", "stale", "unavailable"])
    || !optionalError(value.unavailable)) return false;
  if (value.attempt == null) return isTaskVerificationRecipe(criterion.verification_recipe) && criterion.channel === "command"
    && value.phase === "pending" && value.state === "pending" && value.receipt == null && value.unavailable == null;
  if (!verificationAttempt(value.attempt) || value.attempt.scope.task_id !== task.scope.task_id
    || value.attempt.criterion_id !== criterion.id) return false;
  if (value.receipt != null && (!verificationReceipt(value.receipt, evidenceRevision)
    || !sameVerificationAttempt(value.attempt, value.receipt.verification.attempt))) return false;
  if ((value.phase === "finished") !== (value.receipt != null)) return false;
  if (value.state === "unavailable") return value.unavailable != null;
  if (value.state === "stale") return true;
  if (task.unavailable != null && named(value.state, ["passed", "failed", "running"])) return false;
  if (value.unavailable != null || !sameScope(value.attempt.scope, task.scope)
    || value.attempt.recipe !== criterion.verification_recipe) return false;
  if (value.state === "pending" || value.state === "running") return value.phase === value.state && value.receipt == null;
  if (!verificationReceipt(value.receipt, evidenceRevision)) return false;
  const fact = value.receipt.verification;
  if (value.state === "not_run") return fact.result.kind === "not_run" || fact.result.kind === "interrupted";
  if (fact.result.kind !== "executed" || !hostArtifact(fact.artifact) || !verificationManifest(fact.manifest)
    || !record(task.source) || !record(task.source.tree) || fact.manifest.source.tree_sha !== task.source.tree.tree_sha) return false;
  return value.state === "passed" ? executionPassed(fact.result) : !executionPassed(fact.result);
}

function criterion(value: unknown, task: ObjectValue & { scope: ObjectValue }, evidenceRevision: number): boolean {
  if (!record(value) || !keys(value, ["criterion", "state", "evidence", "waiver", "host_verification"])
    || !record(value.criterion) || !keys(value.criterion, ["id", "scenario", "channel", "essential", "verification_recipe"])
    || !identifier(value.criterion.id) || !text(value.criterion.scenario) || !value.criterion.scenario.trim()
    || !named(value.criterion.channel, ["command", "data", "http", "screen"])
    || typeof value.criterion.essential !== "boolean"
    || (value.state != null && !named(value.state, ["pending", "pass", "fail", "blocked", "stale", "waived", "human_passed", "human_failed"]))) return false;
  if (value.criterion.verification_recipe != null
    && (!isTaskVerificationRecipe(value.criterion.verification_recipe) || value.criterion.channel !== "command")) return false;
  if (value.host_verification != null && !hostVerification(value.host_verification, task, value.criterion, evidenceRevision)) return false;
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
    && sameScope(value.evidence.evidence.scope, task.scope) && (value.state === value.evidence.evidence.status
      || (value.criterion.channel !== "command" && named(value.state, ["human_passed", "human_failed"])));
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
