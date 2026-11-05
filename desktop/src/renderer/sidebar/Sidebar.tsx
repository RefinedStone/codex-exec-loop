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

export function Sidebar({ onOpenThread }: { onOpenThread(threadId: string): void }) {
  const folders = useDesktopStore((state) => state.folders);
  const openThreadId = useDesktopStore((state) => state.openThreadId);
  const activeThreadIds = useDesktopStore((state) => state.activeThreadIds);
  const connection = useDesktopStore((state) => state.connection);
  const phase = connection?.phase ?? "connecting";

  return (
    <nav className="sidebar" data-testid="folders">
      <div className="sidebar-scroll">
        {folders.length === 0 && <p className="muted">스레드가 없습니다.</p>}
        {folders.map((folder) => (
          <section key={folder.cwd} className="folder">
            <h3 title={folder.cwd}>{folderName(folder.cwd)}</h3>
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
