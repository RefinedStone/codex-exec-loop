// 아이템 종류별 표시이다 (P3 §6.3). 모르는 종류도 오류 없이 종류와 상태만 보여 준다.
import type { ThreadItem } from "../../shared/protocol/generated/v2/ThreadItem";

function userText(item: Extract<ThreadItem, { type: "userMessage" }>): string {
  return item.content
    .map((input) => (input.type === "text" ? input.text : `[${input.type}]`))
    .join("\n");
}

function statusOf(item: ThreadItem): string | null {
  const status = (item as { status?: unknown }).status;
  return typeof status === "string" ? status : null;
}

export function ItemView({ item }: { item: ThreadItem }) {
  switch (item.type) {
    case "userMessage":
      return (
        <div className="item item-user" data-testid="item-user">
          {userText(item)}
        </div>
      );
    case "agentMessage":
      return (
        <div className="item item-agent" data-testid="item-agent">
          {item.text}
        </div>
      );
    case "reasoning":
      return item.summary.length > 0 ? (
        <details className="item item-reasoning">
          <summary>생각</summary>
          {item.summary.join("\n")}
        </details>
      ) : null;
    case "commandExecution":
      return (
        <div className="item item-tool" data-testid="item-command">
          <div className="tool-line">
            <span className="tool-label">명령</span>
            <code>{item.command}</code>
            <span className={`tool-status status-${item.status}`}>{item.status}</span>
            {item.exitCode !== null && <span className="muted">exit {item.exitCode}</span>}
          </div>
          {item.aggregatedOutput && (
            <details>
              <summary>출력</summary>
              <pre>{item.aggregatedOutput}</pre>
            </details>
          )}
        </div>
      );
    case "fileChange":
      return (
        <div className="item item-tool" data-testid="item-file-change">
          <div className="tool-line">
            <span className="tool-label">파일 변경</span>
            <span className={`tool-status status-${item.status}`}>{item.status}</span>
          </div>
          <ul>
            {item.changes.map((change) => (
              <li key={change.path}>
                <code>{change.path}</code> <span className="muted">{JSON.stringify(change.kind)}</span>
              </li>
            ))}
          </ul>
        </div>
      );
    case "mcpToolCall":
      return (
        <div className="item item-tool">
          <div className="tool-line">
            <span className="tool-label">도구</span>
            <code>
              {item.server}.{item.tool}
            </code>
            <span className={`tool-status status-${item.status}`}>{item.status}</span>
          </div>
        </div>
      );
    case "plan":
      return (
        <div className="item item-plan">
          <span className="tool-label">계획</span>
          <pre>{item.text}</pre>
        </div>
      );
    default: {
      const status = statusOf(item);
      return (
        <div className="item item-other muted">
          {item.type}
          {status ? ` · ${status}` : ""}
        </div>
      );
    }
  }
}
