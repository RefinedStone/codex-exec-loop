import { samePath, fetchFirstGraphPage } from "../git/gitGraph";
import type { GatewayRequest } from "../work/notionHub";

export interface PreparedWorkspace {
  requestId: string;
  repositoryDir: string;
  workspaceDir: string;
  branch: string;
  baseCommit: string;
  state: "preparing" | "prepared" | "preserved";
  reason: string | null;
}

interface Attempt {
  requestId: string;
  baseCommit: string;
  prepared?: PreparedWorkspace;
  startRequested: boolean;
}

interface WorkspaceThreadIdentity {
  readonly id: string;
  readonly cwd: string;
  readonly gitInfo?: { readonly branch: string | null } | null;
}

/** Retry only preparation with its original coordinates. Never replay an uncertain thread/start. */
export class IsolatedWorkspaceStarter {
  private readonly attempts = new Map<string, Attempt>();
  // A live Desktop session can verify accepted bindings across Gateway reconnects.
  // The recovery inventory does not prove ownership after a Desktop restart.
  private readonly bindings = new Map<string, PreparedWorkspace>();
  private readonly verifying = new Set<string>();
  private busy = false;

  constructor(private readonly request: GatewayRequest, private readonly newId: () => string = () => crypto.randomUUID()) {}

  bind(thread: WorkspaceThreadIdentity, workspace: PreparedWorkspace): void {
    if (!thread.id || !samePath(thread.cwd, workspace.workspaceDir)
      || (thread.gitInfo?.branch != null && thread.gitInfo.branch !== workspace.branch)) {
      throw new Error("격리 공간과 스레드의 연결을 확인하지 못했습니다. 준비된 공간은 보존됩니다.");
    }
    this.bindings.set(thread.id, workspace);
  }

  beginResume(threadId: string | null): void {
    if (threadId !== null && this.bindings.has(threadId)) this.verifying.add(threadId);
  }

  completeResume(threadId: string, thread: WorkspaceThreadIdentity): void {
    const workspace = this.bindings.get(threadId);
    if (!workspace) return;
    if (thread.id !== threadId) throw new Error("격리 공간의 스레드 식별자가 일치하지 않습니다.");
    this.bind(thread, workspace);
    this.verifying.delete(threadId);
  }

  assertVerified(threadId: string): void {
    if (this.verifying.has(threadId)) throw new Error("격리 공간 연결을 확인한 뒤 입력을 보내세요.");
  }

  branchFor(thread: WorkspaceThreadIdentity): string | null {
    const workspace = this.bindings.get(thread.id);
    return workspace && samePath(thread.cwd, workspace.workspaceDir) ? workspace.branch : null;
  }

  async start(cwd: string, open: (workspace: PreparedWorkspace, markStartRequested: () => void) => Promise<string>): Promise<string> {
    if (this.busy) throw new Error("격리 공간 요청이 진행 중입니다.");
    this.busy = true;
    try {
      let attempt = this.attempts.get(cwd);
      if (attempt?.startRequested) {
        throw new Error(`스레드 생성 결과가 불명입니다. 자동으로 다시 만들지 않습니다. 작업 공간: ${attempt.prepared?.workspaceDir ?? cwd}`);
      }
      if (!attempt) {
        const graph = await fetchFirstGraphPage(this.request, cwd, 1);
        if (!graph.repository || !graph.head || !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(graph.head)) {
          throw new Error("격리 공간에는 커밋된 Git HEAD가 필요합니다.");
        }
        attempt = { requestId: this.newId(), baseCommit: graph.head, startRequested: false };
        this.attempts.set(cwd, attempt);
      }
      const coordinates = { cwd, requestId: attempt.requestId };
      // Read the durable record first after a failed or lost prepare response.
      let record = await this.request("akra/conversationWorkspace/get", coordinates) as PreparedWorkspace | null;
      if (record === null || record.state === "preparing") {
        record = await this.request("akra/conversationWorkspace/prepare", { ...coordinates, baseCommit: attempt.baseCommit }) as PreparedWorkspace;
      }
      if (!record || record.requestId !== attempt.requestId || record.baseCommit !== attempt.baseCommit
        || typeof record.repositoryDir !== "string" || typeof record.workspaceDir !== "string"
        || !record.workspaceDir || samePath(record.workspaceDir, cwd)
        || samePath(record.workspaceDir, record.repositoryDir) || typeof record.branch !== "string" || !record.branch) {
        throw new Error("격리 공간 준비 결과를 검증하지 못했습니다.");
      }
      if (record.state !== "prepared") throw new Error(`격리 공간 준비 미완료: ${record.reason ?? record.state}`);
      attempt.prepared = record;
      const preparedAttempt = attempt;
      // The caller marks dispatch after its last cancellation/write-permission check.
      const threadId = await open(record, () => { preparedAttempt.startRequested = true; });
      this.attempts.delete(cwd);
      return threadId;
    } finally {
      this.busy = false;
    }
  }
}
