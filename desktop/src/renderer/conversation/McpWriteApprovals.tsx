// MCP hub 의 쓰기 도구 호출 승인 카드이다 (#2830). 스레드와 상관없이 창 아래쪽에 보인다. 허용하면 Gateway 가 그
// 호출 한 번을 실행하고, 거부하거나 답하지 않으면 실행하지 않는다.
import { useState } from "react";

import { Icon } from "../design/icons";
import { useMcpWriteApprovals } from "../session/mcpWriteApprovals";

const ARGUMENT_PREVIEW_CHARS = 600;

/** 짧은 미리보기를 보이고, 길면 허용하기 전에 전체 인수를 펼쳐 볼 수 있게 한다. */
export function ArgumentsView({ value }: { value: unknown }) {
  const text = JSON.stringify(value, null, 2) ?? "";
  if (text.length <= ARGUMENT_PREVIEW_CHARS) return <pre className="approval-command">{text}</pre>;
  return (
    <>
      <pre className="approval-command">{`${text.slice(0, ARGUMENT_PREVIEW_CHARS)}…`}</pre>
      <details className="mcp-write-arguments" data-testid="mcp-write-arguments">
        <summary>전체 인수 보기 ({text.length.toLocaleString()}자)</summary>
        <pre className="approval-command">{text}</pre>
      </details>
    </>
  );
}

export function McpWriteApprovals({ onAnswer }: { onAnswer(id: string, approve: boolean): Promise<void> }) {
  const requests = useMcpWriteApprovals((state) => state.requests);
  const [error, setError] = useState<string | null>(null);
  if (requests.length === 0) return null;
  const answer = (id: string, approve: boolean) => {
    setError(null);
    onAnswer(id, approve).catch((cause: unknown) => setError(cause instanceof Error ? cause.message : String(cause)));
  };
  return (
    <section className="mcp-write-approvals" data-testid="mcp-write-approvals">
      {requests.map((request) => (
        <div key={request.id} className="approval-card" role="alertdialog" aria-label="MCP 쓰기 도구 승인" data-testid="mcp-write-approval">
          <header>
            <Icon name="alert" />
            <strong>MCP 쓰기 도구 승인</strong>
            <span className="approval-cwd">{request.server} / {request.tool}</span>
          </header>
          <p>에이전트가 외부 서비스에 쓰는 도구를 부르려 합니다. 허용하면 이 호출 한 번만 실행합니다.</p>
          <ArgumentsView value={request.arguments} />
          <div className="approval-buttons">
            <button type="button" className="button primary" onClick={() => answer(request.id, true)} data-testid="mcp-write-approve">허용</button>
            <button type="button" className="button" onClick={() => answer(request.id, false)} data-testid="mcp-write-decline">거부</button>
          </div>
        </div>
      ))}
      {error && <p className="muted">{error}</p>}
    </section>
  );
}
