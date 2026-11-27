import type { GatewayRequest } from "../work/notionHub";

interface WorkspaceInventoryEntry {
  readonly record: {
    readonly requestId: string;
    readonly workspaceDir: string;
    readonly repositoryDir: string;
    readonly branch: string;
    readonly baseCommit: string;
    readonly state: "preparing" | "prepared" | "preserved";
    readonly reason: string | null;
  };
  readonly currentHead: string | null;
  readonly reasons: readonly string[];
  readonly cleanupAllowed: false;
}

export interface WorkspaceInventoryPage {
  readonly items: readonly WorkspaceInventoryEntry[];
  readonly nextAfter: string | null;
}

class WorkspaceInventoryError extends Error {
  constructor() { super("격리 공간 목록 응답을 검증하지 못했습니다. 공간은 그대로 보존합니다."); }
}

function isEntry(value: unknown): value is WorkspaceInventoryEntry {
  if (typeof value !== "object" || value === null || !("record" in value)) return false;
  const record = value.record;
  return typeof record === "object" && record !== null
    && "requestId" in record && typeof record.requestId === "string" && /^[A-Za-z0-9_-]{1,128}$/.test(record.requestId)
    && "workspaceDir" in record && typeof record.workspaceDir === "string" && record.workspaceDir.length > 0
    && "repositoryDir" in record && typeof record.repositoryDir === "string"
    && "branch" in record && typeof record.branch === "string"
    && "baseCommit" in record && typeof record.baseCommit === "string"
    && "state" in record && ["preparing", "prepared", "preserved"].some(state => state === record.state)
    && "reason" in record && (record.reason === null || typeof record.reason === "string")
    && "currentHead" in value && (value.currentHead === null || typeof value.currentHead === "string")
    && "reasons" in value && Array.isArray(value.reasons) && value.reasons.every(reason => typeof reason === "string")
    && "cleanupAllowed" in value && value.cleanupAllowed === false;
}

export async function readWorkspaceInventory(
  request: GatewayRequest,
  query: { readonly cwd: string; readonly after: string | null },
): Promise<WorkspaceInventoryPage> {
  const value = await request<unknown>("akra/conversationWorkspace/inventory", query);
  if (typeof value !== "object" || value === null
    || !("items" in value) || !Array.isArray(value.items) || !value.items.every(isEntry)
    || !("nextAfter" in value) || !(value.nextAfter === null || typeof value.nextAfter === "string")) {
    throw new WorkspaceInventoryError();
  }
  if (value.nextAfter !== null && (!/^[A-Za-z0-9_-]{1,128}$/.test(value.nextAfter)
    || value.nextAfter === query.after || value.nextAfter !== value.items.at(-1)?.record.requestId)) {
    throw new WorkspaceInventoryError();
  }
  return { items: value.items, nextAfter: value.nextAfter };
}

export function workspaceRecoveryReason(reason: string): string {
  switch (reason) {
    case "thread_ownership_unconfirmed": return "대화 소유권을 확인하지 못했습니다.";
    case "remote_retention_unconfirmed": return "원격 보관 상태를 확인하지 못했습니다.";
    case "preparation_not_confirmed": return "공간 준비가 확정되지 않았습니다.";
    case "workspace_missing": return "작업 공간 경로가 없습니다. 준비 기록은 보존합니다.";
    case "workspace_identity_unconfirmed":
    case "created_workspace_identity_unconfirmed": return "작업 공간의 저장소 신원을 확인하지 못했습니다.";
    case "branch_changed": return "준비한 브랜치와 현재 브랜치가 다릅니다.";
    case "head_changed": return "준비한 커밋 이후 HEAD가 바뀌었습니다.";
    case "tracked_changes": return "커밋되지 않은 파일 변경이 있습니다.";
    case "git_operation_in_progress": return "Git 작업이 진행 중입니다.";
    case "untracked_files_unconfirmed": return "추적하지 않는 파일의 보존 상태를 확인하지 못했습니다.";
    case "workspace_status_unconfirmed": return "작업 공간의 현재 상태를 확인하지 못했습니다.";
    case "worktree_creation_incomplete": return "작업 공간 생성이 완료되지 않았습니다.";
    default: return "알 수 없는 보존 사유가 있습니다. 정리를 보류합니다.";
  }
}
