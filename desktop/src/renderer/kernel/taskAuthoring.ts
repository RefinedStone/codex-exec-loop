import { JsonRpcError } from "../rpc/jsonRpcClient";
import { KERNEL_ERROR, KERNEL_METHOD, type CommandRequest, type CommandResponse, type KernelQueryAnswer, type TaskFields } from "./protocol";

export type TaskListAnswer = Extract<KernelQueryAnswer, { type: "list_tasks" }>;
type OperationAnswer = Extract<KernelQueryAnswer, { type: "operation" }>;
type HistoryAnswer = Extract<KernelQueryAnswer, { type: "history" }>;
export type TaskSaveOutcome =
  | { kind: "saved"; taskId: string; planningRevision: number }
  | { kind: "rejected"; message: string }
  | { kind: "uncertain"; message: string };

export interface TaskAuthoringContext {
  workspace: string | null;
  kernel: string | null;
  attached: boolean;
  epoch: number;
}
interface Rpc {
  request<T>(method: string, params: unknown): Promise<T>;
}
interface SaveAttempt {
  workspace: string;
  kernel: string;
  request: CommandRequest;
  commandJson: string;
  outcome: TaskSaveOutcome | null;
  operationId: number | null;
  running: Promise<TaskSaveOutcome> | null;
}

const UNCERTAIN = "저장 결과를 확인하지 못했습니다. 같은 요청으로 결과를 다시 확인하세요.";
const SCOPE_CHANGED = "저장소 연결이 바뀌었습니다. 원래 저장소에서 결과를 확인하세요.";

export function taskCriteriaError(fields: TaskFields): string | null {
  if (!fields.title.trim()) return "작업 제목을 입력하세요.";
  const ids = new Set<string>();
  for (const criterion of fields.acceptance ?? []) {
    if (!criterion.id.trim() || !criterion.scenario.trim()) return "완료 기준 내용을 입력하세요.";
    if (ids.has(criterion.id.trim())) return "완료 기준 ID가 중복되었습니다.";
    ids.add(criterion.id.trim());
    if (criterion.verification_recipe && criterion.channel !== "command") return "호스트 검증은 명령 채널에서만 지정할 수 있습니다.";
  }
  return null;
}

/** 작업 초안은 renderer에만 있다. 단일 논리 요청과 키는 응답 유실·재접속에도 유지한다. */
export class TaskAuthoringController {
  private readonly attempts = new Map<string, SaveAttempt>();

  constructor(private readonly rpc: Rpc, private readonly context: () => TaskAuthoringContext,
    private readonly timeoutMs = 20_000) {}

  pending(workspace: string): TaskSaveOutcome | null {
    const attempt = this.attempts.get(workspace);
    if (!attempt) return null;
    return attempt.outcome ?? { kind: "uncertain", message: "저장 요청의 결과를 기다리는 중입니다." };
  }

  /** 사용자가 별개의 새 초안을 연다. 결과가 미확정인 이전 쓰기는 버리지 않는다. */
  beginDraft(workspace: string): void {
    const attempt = this.attempts.get(workspace);
    if (attempt && (attempt.running || !attempt.outcome || attempt.outcome.kind === "uncertain")) return;
    this.attempts.delete(workspace);
  }

  async read(workspace: string): Promise<TaskListAnswer> {
    const before = this.requireContext(workspace);
    const answer = await this.bounded(this.rpc.request<TaskListAnswer>(KERNEL_METHOD.query, { query: { type: "list_tasks" } }));
    this.requireSameContext(before);
    if (answer.type !== "list_tasks" || !answer.available || answer.planning_revision === null) {
      throw new Error("작업 원본을 읽을 수 없습니다. 저장소 상태를 확인하고 다시 읽으세요.");
    }
    return answer;
  }

  save(workspace: string, taskId: string | null, fields: TaskFields, planningRevision: number | null): Promise<TaskSaveOutcome> {
    const context = this.requireContext(workspace);
    const validation = taskCriteriaError(fields);
    if (validation) return Promise.resolve({ kind: "rejected", message: validation });
    if (fields.status === "in_progress") return Promise.resolve({ kind: "rejected", message: "진행 중인 작업은 완료 기준을 수정할 수 없습니다." });
    if (taskId && planningRevision === null) return Promise.resolve({ kind: "rejected", message: "최신 작업을 다시 읽으세요." });
    const command = taskId
      ? { type: "update_task" as const, task_id: taskId, task: structuredClone(fields), expected_planning_revision: planningRevision! }
      : { type: "create_task" as const, task: { ...structuredClone(fields), status: "awaiting_user" as const } };
    const commandJson = JSON.stringify(command);
    const prior = this.attempts.get(workspace);
    if (prior && (!prior.outcome || prior.outcome.kind === "uncertain" || prior.running || prior.commandJson === commandJson)) {
      if (prior.commandJson !== commandJson) return Promise.resolve({ kind: "uncertain", message: UNCERTAIN });
      return this.execute(prior);
    }
    const attempt: SaveAttempt = {
      workspace, kernel: context.kernel!, commandJson,
      request: { command, idempotency_key: crypto.randomUUID() },
      outcome: null, operationId: null, running: null,
    };
    this.attempts.set(workspace, attempt);
    return this.execute(attempt);
  }

  recover(workspace: string): Promise<TaskSaveOutcome> {
    const attempt = this.attempts.get(workspace);
    if (!attempt) return Promise.resolve({ kind: "rejected", message: "확인할 저장 요청이 없습니다." });
    return this.execute(attempt);
  }

  private execute(attempt: SaveAttempt): Promise<TaskSaveOutcome> {
    const before = this.requireContext(attempt.workspace);
    if (before.kernel !== attempt.kernel) return Promise.reject(new Error(SCOPE_CHANGED));
    if (attempt.running) return attempt.running;
    if (attempt.outcome && attempt.outcome.kind !== "uncertain") return Promise.resolve(attempt.outcome);
    const running = this.perform(attempt).then((outcome) => {
      attempt.outcome = outcome;
      this.requireSameContext(before);
      return outcome;
    }).finally(() => { attempt.running = null; });
    attempt.running = running;
    return running;
  }

  private async perform(attempt: SaveAttempt): Promise<TaskSaveOutcome> {
    try {
      // 보존 범위 밖의 멱등 키는 커널도 잊는다. 이미 제출한 미확정 요청은 읽기로만 복구한다.
      if (attempt.outcome?.kind === "uncertain") {
        const opId = attempt.operationId ?? await this.findRecordedOperation(attempt);
        if (opId === null) return { kind: "uncertain", message: "기록에서 이전 저장 요청을 확인하지 못했습니다. 작업 목록을 확인하세요. 중복 생성을 막기 위해 저장 요청을 다시 보내지 않았습니다." };
        attempt.operationId = opId;
        return await this.recoverOperation(attempt, opId);
      }
      const response = await this.bounded(this.rpc.request<CommandResponse>(KERNEL_METHOD.command, attempt.request));
      if (response.rejection) return { kind: "rejected", message: response.rejection.message };
      if (response.processed && response.operation) attempt.operationId = response.operation.op_id;
      const result = response.result;
      const expectedType = attempt.request.command.type === "create_task" ? "task_created" : "task_updated";
      if (response.processed === true && result?.type === expectedType
        && (result.type === "task_created" || result.type === "task_updated")) {
        const expected = attempt.request.command.type === "update_task" ? attempt.request.command.task_id : null;
        if (!result.task_id || !Number.isSafeInteger(result.planning_revision)
          || (expected && result.task_id !== expected)) return { kind: "uncertain", message: UNCERTAIN };
        return { kind: "saved", taskId: result.task_id, planningRevision: result.planning_revision };
      }
      if (response.operation && response.processed) return await this.recoverOperation(attempt, response.operation.op_id);
      return { kind: "uncertain", message: response.failure ?? UNCERTAIN };
    } catch (error) {
      if (error instanceof JsonRpcError && new Set<number>([KERNEL_ERROR.invalid_params, KERNEL_ERROR.method_not_found]).has(error.code)) {
        return { kind: "rejected", message: error.message };
      }
      return { kind: "uncertain", message: UNCERTAIN };
    }
  }

  /** 별도 저장소를 만들지 않고 기존 결정 기록에서 원 요청의 좌표를 찾는다. 누락은 실패로 단정하지 않는다. */
  private async findRecordedOperation(attempt: SaveAttempt): Promise<number | null> {
    const context = this.requireContext(attempt.workspace);
    if (context.kernel !== attempt.kernel) return null;
    let after = 0;
    for (let page = 0; page < 32; page++) {
      const answer = await this.bounded(this.rpc.request<HistoryAnswer>(KERNEL_METHOD.query, {
        query: { type: "history", after, limit: 500 },
      }));
      this.requireSameContext(context);
      if (answer.type !== "history") return null;
      for (const { sequence, event } of answer.records) {
        if (event.kind === "command_received" && event.command === attempt.request.command.type
          && event.idempotency_key === attempt.request.idempotency_key && event.op_id === sequence) return event.op_id;
      }
      if (!answer.records.length || answer.next_after <= after) return null;
      after = answer.next_after;
    }
    return null;
  }

  private async recoverOperation(attempt: SaveAttempt, opId: number): Promise<TaskSaveOutcome> {
    const context = this.requireContext(attempt.workspace);
    if (context.kernel !== attempt.kernel) return { kind: "uncertain", message: SCOPE_CHANGED };
    const answer = await this.bounded(this.rpc.request<OperationAnswer>(KERNEL_METHOD.query, { query: { type: "operation", op_id: opId } }));
    this.requireSameContext(context);
    if (answer.type !== "operation" || answer.op_id !== opId) return { kind: "uncertain", message: UNCERTAIN };
    const admitted = answer.records.find(({ event }) => event.kind === "command_received" && event.op_id === opId
      && event.command === attempt.request.command.type && event.idempotency_key === attempt.request.idempotency_key);
    if (!admitted) return { kind: "uncertain", message: UNCERTAIN };
    for (const { sequence, event } of answer.records) {
      if (sequence <= admitted.sequence || event.kind !== "command_completed" || event.op_id !== opId) continue;
      if (event.result !== "done" || !event.task_mutation) return { kind: "uncertain", message: event.detail ?? UNCERTAIN };
      const expected = attempt.request.command.type === "update_task" ? attempt.request.command.task_id : null;
      if (expected && expected !== event.task_mutation.task_id) return { kind: "uncertain", message: UNCERTAIN };
      return { kind: "saved", taskId: event.task_mutation.task_id, planningRevision: event.task_mutation.planning_revision };
    }
    return { kind: "uncertain", message: UNCERTAIN };
  }

  private requireContext(workspace: string): TaskAuthoringContext {
    const context = this.context();
    if (!context.attached || context.workspace !== workspace || !context.kernel) throw new Error("이 저장소 커널에 연결한 뒤 다시 시도하세요.");
    return context;
  }

  private requireSameContext(before: TaskAuthoringContext): void {
    const after = this.context();
    if (!after.attached || after.workspace !== before.workspace || after.kernel !== before.kernel || after.epoch !== before.epoch) {
      throw new Error(SCOPE_CHANGED);
    }
  }

  private async bounded<T>(request: Promise<T>): Promise<T> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      return await Promise.race([request, new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error(UNCERTAIN)), this.timeoutMs); })]);
    } finally {
      clearTimeout(timer);
    }
  }
}
