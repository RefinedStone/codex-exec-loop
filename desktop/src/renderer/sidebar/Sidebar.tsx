// 사이드바: 작업 폴더(스레드 cwd)별 스레드 목록이다 (D-16). 진행 중인 스레드에는 표시를 붙인다.
import type { ConnectionState } from "../../shared/bridge";
import { useDesktopStore } from "../session/desktopStore";

const PHASE_LABEL: Record<ConnectionState["phase"], string> = {
  connecting: "연결 중",
  connected: "연결됨",
  disconnected: "끊김",
};

function folderName(cwd: string): string {
  const parts = cwd.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) ?? cwd;
}

export function Sidebar({
  onOpenThread,
  onStartThread,
  onStartThreadInNewFolder,
}: {
  onOpenThread(threadId: string): void;
  onStartThread(cwd: string): void;
  /** 폴더를 골라 새 스레드를 시작한다. 스레드가 하나도 없는 새 설치에서도 쓸 수 있는 진입점이다. */
  onStartThreadInNewFolder(): void;
}) {
  const folders = useDesktopStore((state) => state.folders);
  const openThreadId = useDesktopStore((state) => state.openThreadId);
  const activeThreadIds = useDesktopStore((state) => state.activeThreadIds);
  const pendingApprovals = useDesktopStore((state) => state.pendingApprovals);
  const connection = useDesktopStore((state) => state.connection);
  const phase = connection?.phase ?? "connecting";
  const waitingForApproval = new Set(pendingApprovals.map((approval) => approval.threadId));

  return (
    <nav className="sidebar" data-testid="folders">
      <div className="sidebar-scroll">
        <button
          type="button"
          className="new-folder-thread"
          onClick={onStartThreadInNewFolder}
          disabled={phase !== "connected"}
          data-testid="new-folder-thread"
        >
          + 폴더를 골라 새 스레드
        </button>
        {folders.length === 0 && <p className="muted">아직 스레드가 없습니다. 위 버튼으로 작업 폴더를 고르세요.</p>}
        {folders.map((folder) => (
          <section key={folder.cwd} className="folder">
            <h3 title={folder.cwd}>
              <span>{folderName(folder.cwd)}</span>
              <button
                type="button"
                className="new-thread"
                onClick={() => onStartThread(folder.cwd)}
                disabled={phase !== "connected"}
                title="이 폴더에서 새 스레드"
                data-testid="new-thread"
              >
                +
              </button>
            </h3>
            <ul>
              {folder.threads.map((thread) => (
                <li key={thread.id}>
                  <button
                    type="button"
                    className={thread.id === openThreadId ? "thread selected" : "thread"}
                    onClick={() => onOpenThread(thread.id)}
                    data-testid="thread-entry"
                  >
                    {activeThreadIds.includes(thread.id) && <span className="active-dot" aria-label="진행 중" />}
                    <span className="thread-title">{thread.title}</span>
                    {waitingForApproval.has(thread.id) && (
                      <span className="approval-badge" data-testid="approval-badge">
                        승인
                      </span>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
      <footer className="sidebar-footer">
        <span className={`status status-${phase}`} data-testid="connection-phase">
          {PHASE_LABEL[phase]}
        </span>
        {phase === "disconnected" && connection?.retryInMs !== undefined && (
          <span className="retry"> · {Math.round(connection.retryInMs / 1000)}초 뒤 다시 연결</span>
        )}
      </footer>
    </nav>
  );
}
