// git 작업의 자료이다 (plan 27 G3). Gateway 의 작업 엔진이 계획을 만들고, 기대 상태를 다시 본 뒤 실행하고, 일지에
// 적고, 되돌린다. 여기서는 요청을 보내고, 답을 읽고, 화면에 쓸 글을 만든다. git 을 직접 부르지 않는다.
import type { GatewayRequest } from "../work/notionHub";
import { type GitRef, type GitWorktree, samePath, shortSha } from "./gitGraph";

export const GIT_PLAN_METHOD = "akra/git/plan";
export const GIT_EXECUTE_METHOD = "akra/git/execute";
export const GIT_OPERATIONS_METHOD = "akra/git/operations";
export const GIT_UNDO_METHOD = "akra/git/undo";

/** 작업 하나이다. 커밋은 전체 sha 이다. */
export type GitOperation =
  | { kind: "reset"; target: string }
  | { kind: "move_branch"; branch: string; target: string }
  | { kind: "checkout_branch"; branch: string }
  | { kind: "checkout_commit"; target: string };

export interface GitHeadPosition {
  commit: string;
  /** 브랜치 없이 체크아웃한 자리는 `null` 이다. */
  branch: string | null;
}

export interface GitOperationImpact {
  /** 브랜치에서 빠지는 커밋 수이다. */
  dropped: number;
  /** 빠지는 커밋 가운데 다른 브랜치와 태그가 닿지 못하는 수이다. 백업 ref 만 이 커밋을 지킨다. */
  unreachable: number;
  /** 브랜치에 새로 들어오는 커밋 수이다. */
  gained: number;
}

export interface GitOperationBlocker {
  code: string;
  /** Gateway 가 쓴 영어 한 문장이다. 모르는 `code` 일 때 보인다. */
  message: string;
}

export interface GitOperationPlan {
  operation: GitOperation;
  /** 계획을 만들 때 본 상태이다. 내용을 보지 않고 실행 요청에 그대로 돌려보낸다. */
  expectation: unknown;
  branch: string | null;
  before: GitHeadPosition | null;
  after: GitHeadPosition | null;
  impact: GitOperationImpact | null;
  /** 비어 있어야 실행할 수 있다. */
  blockers: GitOperationBlocker[];
}

export type GitOperationStatus = "started" | "done" | "failed";

export interface GitOperationRecord {
  opId: string;
  operation: GitOperation;
  /** 작업을 실행한 폴더이다. reset 과 체크아웃은 이 작업 트리의 HEAD 를 바꿨다. */
  worktree: string;
  before: GitHeadPosition;
  after: GitHeadPosition;
  backupRef: string | null;
  status: GitOperationStatus;
  /** Unix 초이다. */
  startedAt: number;
  error: string | null;
  /** 이 작업이 되돌린 작업의 id 이다. */
  undoOf: string | null;
  /** 이 작업을 되돌린 작업의 id 이다. */
  undoneBy: string | null;
}

export type GitOperationOutcome =
  | { status: "done"; record: GitOperationRecord }
  | { status: "failed"; record: GitOperationRecord }
  | { status: "stale"; plan: GitOperationPlan }
  | { status: "refused"; message: string };

function text(value: unknown): string | null {
  return typeof value === "string" && value ? value : null;
}

function count(value: unknown): number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 ? value : 0;
}

function operationFrom(value: unknown): GitOperation | null {
  const item = (value ?? {}) as Record<string, unknown>;
  const target = text(item.target);
  const branch = text(item.branch);
  if (item.kind === "reset" && target) return { kind: "reset", target };
  if (item.kind === "move_branch" && branch && target) return { kind: "move_branch", branch, target };
  if (item.kind === "checkout_branch" && branch) return { kind: "checkout_branch", branch };
  if (item.kind === "checkout_commit" && target) return { kind: "checkout_commit", target };
  return null;
}

function positionFrom(value: unknown): GitHeadPosition | null {
  const item = (value ?? {}) as Record<string, unknown>;
  const commit = text(item.commit);
  return commit ? { commit, branch: text(item.branch) } : null;
}

/** Gateway 의 계획을 읽는다. 작업이나 막는 이유의 목록을 읽지 못하면 오류이다. */
export function planFrom(result: unknown): GitOperationPlan {
  const plan = (result ?? {}) as Record<string, unknown>;
  const operation = operationFrom(plan.operation);
  if (!operation || !Array.isArray(plan.blockers)) throw new Error("git 작업 계획의 형식을 알 수 없습니다.");
  const impact = plan.impact as Record<string, unknown> | null | undefined;
  return {
    operation,
    expectation: plan.expectation ?? null,
    branch: text(plan.branch),
    before: positionFrom(plan.before),
    after: positionFrom(plan.after),
    impact: impact ? { dropped: count(impact.dropped), unreachable: count(impact.unreachable), gained: count(impact.gained) } : null,
    blockers: plan.blockers.flatMap((entry) => {
      const blocker = (entry ?? {}) as Record<string, unknown>;
      const code = text(blocker.code);
      return code ? [{ code, message: text(blocker.message) ?? "" }] : [];
    }),
  };
}

const STATUSES: readonly GitOperationStatus[] = ["started", "done", "failed"];

/** 일지의 기록 하나를 읽는다. 읽지 못한 기록은 `null` 이다. */
export function recordFrom(value: unknown): GitOperationRecord | null {
  const item = (value ?? {}) as Record<string, unknown>;
  const opId = text(item.opId);
  const operation = operationFrom(item.operation);
  const before = positionFrom(item.before);
  const after = positionFrom(item.after);
  const status = STATUSES.find((candidate) => candidate === item.status);
  if (!opId || !operation || !before || !after || !status) return null;
  return { opId, operation, worktree: text(item.worktree) ?? "", before, after, backupRef: text(item.backupRef), status, startedAt: count(item.startedAt), error: text(item.error), undoOf: text(item.undoOf), undoneBy: text(item.undoneBy) };
}

/** 실행이나 되돌리기의 답을 읽는다. */
export function outcomeFrom(result: unknown): GitOperationOutcome {
  const outcome = (result ?? {}) as Record<string, unknown>;
  if (outcome.status === "stale") return { status: "stale", plan: planFrom(outcome.plan) };
  if (outcome.status === "refused") return { status: "refused", message: text(outcome.message) ?? "" };
  const record = recordFrom(outcome.record);
  if ((outcome.status === "done" || outcome.status === "failed") && record) return { status: outcome.status, record };
  throw new Error("git 작업 결과의 형식을 알 수 없습니다.");
}

export async function fetchPlan(request: GatewayRequest, cwd: string, operation: GitOperation): Promise<GitOperationPlan> {
  return planFrom(await request(GIT_PLAN_METHOD, { cwd, operation }));
}

/** 계획을 실행한다. Gateway 는 계획의 기대 상태가 지금 상태와 같을 때만 실행한다. */
export async function executePlan(request: GatewayRequest, cwd: string, plan: GitOperationPlan): Promise<GitOperationOutcome> {
  return outcomeFrom(await request(GIT_EXECUTE_METHOD, { cwd, operation: plan.operation, expectation: plan.expectation }));
}

/** 일지의 최근 작업들이다. 새 것이 앞이다. */
export async function fetchOperations(request: GatewayRequest, cwd: string): Promise<GitOperationRecord[]> {
  const result = (await request(GIT_OPERATIONS_METHOD, { cwd })) as Record<string, unknown> | null;
  return Array.isArray(result?.operations) ? result.operations.flatMap((entry) => recordFrom(entry) ?? []) : [];
}

export async function undoOperation(request: GatewayRequest, cwd: string, opId: string): Promise<GitOperationOutcome> {
  return outcomeFrom(await request(GIT_UNDO_METHOD, { cwd, opId }));
}

/**
 * 되돌리기 단추를 보일 기록이다. 일지는 저장소의 모든 작업 트리가 함께 쓴다. reset 과 체크아웃은 그 작업을 한 작업
 * 트리(`cwd` 와 같은 폴더)에서만 되돌린다. 브랜치 이동은 어느 작업 트리에서나 되돌린다. 저장소가 그 뒤에 바뀌었는지는
 * Gateway 가 본다.
 */
export function canUndo(record: GitOperationRecord, cwd: string): boolean {
  const here = record.operation.kind === "move_branch" || samePath(record.worktree, cwd);
  return here && record.status === "done" && record.undoneBy === null && record.undoOf === null;
}

/** 고른 커밋에서 할 수 있는 작업들이다. */
export interface CommitActions {
  /** 현재 브랜치를 이 커밋으로 옮긴다. 현재 브랜치가 없거나 이미 이 커밋이면 `null` 이다. */
  reset: { operation: GitOperation; branch: string } | null;
  /** 이 커밋에 있는 로컬 브랜치의 체크아웃이다. 지금 체크아웃한 브랜치는 뺀다. */
  checkoutBranches: { operation: GitOperation; branch: string }[];
  /** 이 커밋을 브랜치 없이 체크아웃한다. 이미 그 자리이면 `null` 이다. */
  checkoutCommit: GitOperation | null;
  /** 이 커밋으로 옮길 수 있는 브랜치들이다. 어느 작업 트리도 체크아웃하지 않았고 다른 커밋에 있다. */
  movableBranches: string[];
}

/** 병렬 작업 슬롯의 브랜치이다. 작업 엔진이 고치지 않는다. */
function isAgentBranch(name: string): boolean {
  return name.startsWith("akra-agent/");
}

export function commitActions(commit: string, refs: readonly GitRef[], head: string | null, worktrees: readonly GitWorktree[]): CommitActions {
  const locals = refs.filter((ref) => ref.kind === "local_branch" && !isAgentBranch(ref.name));
  const current = refs.find((ref) => ref.kind === "local_branch" && ref.head) ?? null;
  const checkedOut = new Set(worktrees.flatMap((worktree) => (worktree.branch ? [worktree.branch] : [])));
  const currentIsAgent = current !== null && isAgentBranch(current.name);
  return {
    reset: current && !currentIsAgent && head !== commit ? { operation: { kind: "reset", target: commit }, branch: current.name } : null,
    checkoutBranches: currentIsAgent ? [] : locals.filter((ref) => ref.target === commit && !ref.head && !checkedOut.has(ref.name)).map((ref) => ({ operation: { kind: "checkout_branch", branch: ref.name }, branch: ref.name })),
    checkoutCommit: currentIsAgent || (current === null && head === commit) ? null : { kind: "checkout_commit", target: commit },
    movableBranches: locals.filter((ref) => ref.target !== commit && !ref.head && !checkedOut.has(ref.name)).map((ref) => ref.name),
  };
}

/** 작업을 한 줄로 쓴다: "작업: 무엇 → 어디". 이름 뒤에 조사를 붙이지 않는다. 이름의 끝 글자를 알 수 없다. */
export function operationTitle(operation: GitOperation, branch: string | null): string {
  switch (operation.kind) {
    case "reset":
      return `reset: ${branch ?? "현재 브랜치"} → ${shortSha(operation.target)}`;
    case "move_branch":
      return `브랜치 이동: ${operation.branch} → ${shortSha(operation.target)}`;
    case "checkout_branch":
      return `체크아웃: ${operation.branch}`;
    case "checkout_commit":
      return `체크아웃: ${shortSha(operation.target)} (브랜치 없이)`;
  }
}

/** 계획의 영향을 문장들로 쓴다. 브랜치를 옮기지 않는 작업은 자리의 변화만 쓴다. */
export function planSummary(plan: GitOperationPlan): string[] {
  const lines: string[] = [];
  if (plan.before && plan.after) {
    const place = (position: GitHeadPosition) => (plan.branch === null && position.branch ? `${position.branch} (${shortSha(position.commit)})` : shortSha(position.commit));
    lines.push(`${plan.branch ?? "HEAD"}: ${place(plan.before)} → ${place(plan.after)}`);
  }
  if (plan.impact) {
    const { dropped, unreachable, gained } = plan.impact;
    if (dropped > 0) {
      lines.push(`브랜치에서 커밋 ${dropped}개가 빠집니다.`);
      lines.push(unreachable > 0 ? `그중 ${unreachable}개는 다른 브랜치와 태그가 닿지 못합니다. 백업 ref 가 지킵니다.` : "빠지는 커밋은 모두 다른 브랜치나 태그에서 닿습니다.");
    }
    if (gained > 0) lines.push(`브랜치에 커밋 ${gained}개가 들어옵니다.`);
  }
  if (plan.operation.kind === "reset") lines.push("작업 트리의 파일도 그 커밋으로 바뀝니다. 추적하지 않는 파일은 그대로 둡니다.");
  if (plan.operation.kind === "move_branch") lines.push("작업 트리의 파일은 바뀌지 않습니다.");
  if (plan.operation.kind === "checkout_commit") lines.push("브랜치가 없는 자리입니다. 여기서 만든 커밋은 브랜치를 만들어야 남습니다.");
  return lines;
}

const BLOCKER_TEXT: Record<string, string> = {
  not_repository: "이 폴더는 git 저장소가 아닙니다.",
  no_commits: "이 저장소에는 아직 커밋이 없습니다.",
  execution_config: "이 저장소는 Akra 가 고치지 않습니다. 파일이 필터(Git LFS 등)를 쓰거나, git 설정이 프로그램을 실행합니다. 터미널에서 git 으로 처리하십시오.",
  operation_in_progress: "끝나지 않은 git 작업(merge, rebase, cherry-pick 등)이 있습니다. 먼저 끝내거나 중단하십시오.",
  unknown_commit: "저장소에 그 커밋이 없습니다.",
  unknown_branch: "그 이름의 로컬 브랜치가 없습니다.",
  detached_head: "지금 체크아웃한 브랜치가 없습니다. 옮길 현재 브랜치가 없습니다.",
  uncommitted_changes: "커밋하지 않은 변경이 있습니다. 먼저 커밋하거나 stash 하십시오.",
  nothing_to_do: "이미 그 자리입니다.",
  branch_checked_out: "다른 작업 트리가 그 브랜치를 체크아웃했습니다. 그 작업 트리에서 하십시오.",
  agent_worktree: "병렬 작업 슬롯의 브랜치입니다. 병렬 런타임이 관리합니다.",
};

/** 막는 이유를 한국어 한 문장으로 쓴다. 모르는 종류는 Gateway 의 글을 쓴다. */
export function blockerText(blocker: GitOperationBlocker): string {
  return BLOCKER_TEXT[blocker.code] ?? (blocker.message || blocker.code);
}

/** 일지의 기록을 한 줄로 쓴다. */
export function recordTitle(record: GitOperationRecord): string {
  const branch = record.operation.kind === "reset" ? record.after.branch : null;
  const title = operationTitle(record.operation, branch);
  return record.undoOf === null ? title : `되돌리기 (${title})`;
}

const STATUS_TEXT: Record<GitOperationStatus, string> = { started: "끝을 적지 못함", done: "완료", failed: "실패" };

export function recordStatus(record: GitOperationRecord): string {
  return record.undoneBy !== null ? "되돌림" : STATUS_TEXT[record.status];
}

/** 실행이나 되돌리기 뒤에 보이는 한 줄이다. */
export interface OperationNotice {
  text: string;
  failed: boolean;
  /** 방금 끝난 작업이다. 되돌릴 수 있으면 알림에 되돌리기 단추가 있다. */
  record: GitOperationRecord | null;
}

/** 결과를 알림으로 쓴다. `undo` 는 되돌리기의 결과일 때이다. */
export function noticeFrom(outcome: GitOperationOutcome, undo: boolean): OperationNotice {
  switch (outcome.status) {
    case "done":
      return { text: `${recordTitle(outcome.record)} · 완료`, failed: false, record: outcome.record };
    case "failed":
      // git 이 파일을 쓰다가 멈췄을 수 있다 (다른 프로그램이 파일을 잡고 있었다). 작업 트리를 보게 한다.
      return { text: `${recordTitle(outcome.record)} · 실패. ${outcome.record.error ?? ""} 터미널에서 git status 로 작업 트리를 확인하십시오.`.replace(/\s+/g, " "), failed: true, record: null };
    case "stale":
      return { text: `${undo ? "되돌리지" : "실행하지"} 않았습니다. ${outcome.plan.blockers.map(blockerText).join(" ") || "저장소가 계획과 다릅니다."}`, failed: true, record: null };
    case "refused":
      return { text: "되돌리지 않았습니다. 그 작업 뒤에 저장소가 바뀌었거나, 이미 되돌린 작업입니다.", failed: true, record: null };
  }
}
