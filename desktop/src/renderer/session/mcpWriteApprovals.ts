// MCP hub 쓰기 도구 호출의 승인 요청이다 (#2830). Gateway 가 붙어 있는 모든 Desktop 창에 알리고, 먼저 온 답 하나로
// 끝낸다. 스레드에 묶이지 않으므로 대화의 승인 카드와 따로 둔다.
import { create } from "zustand";

export interface McpWriteApprovalRequest {
  id: string;
  server: string;
  tool: string;
  arguments: unknown;
}

export const MCP_WRITE_APPROVAL_SUBSCRIBE = "akra/mcp/writeApproval/subscribe";
export const MCP_WRITE_APPROVAL_REQUESTED = "akra/mcp/writeApproval/requested";
export const MCP_WRITE_APPROVAL_RESOLVED = "akra/mcp/writeApproval/resolved";
export const MCP_WRITE_APPROVAL_ANSWER = "akra/mcp/writeApproval/answer";

export const useMcpWriteApprovals = create<{ requests: McpWriteApprovalRequest[] }>(() => ({ requests: [] }));

/** Gateway 알림을 목록에 반영한다. 다루는 알림이면 true 이다. */
export function applyMcpWriteApprovalNotification(method: string, params: unknown): boolean {
  const fields = params !== null && typeof params === "object" ? params as Record<string, unknown> : null;
  if (method === MCP_WRITE_APPROVAL_REQUESTED) {
    if (fields && typeof fields.id === "string" && typeof fields.server === "string" && typeof fields.tool === "string") {
      const request: McpWriteApprovalRequest = { id: fields.id, server: fields.server, tool: fields.tool, arguments: fields.arguments ?? null };
      useMcpWriteApprovals.setState((state) => ({ requests: [...state.requests.filter((item) => item.id !== request.id), request] }));
    }
    return true;
  }
  if (method === MCP_WRITE_APPROVAL_RESOLVED) {
    if (fields && typeof fields.id === "string") removeMcpWriteApproval(fields.id);
    return true;
  }
  return false;
}

export function removeMcpWriteApproval(id: string): void {
  useMcpWriteApprovals.setState((state) => ({ requests: state.requests.filter((item) => item.id !== id) }));
}

/** 접속이 끊기면 Gateway 쪽 요청도 이 창에서는 답할 수 없다. 목록을 비운다. */
export function clearMcpWriteApprovals(): void {
  useMcpWriteApprovals.setState({ requests: [] });
}
