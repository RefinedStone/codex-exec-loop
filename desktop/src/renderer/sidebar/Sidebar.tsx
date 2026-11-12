// 사이드바: 작업 폴더(스레드 cwd)별 스레드 목록이다 (D-16). 스레드마다 상태를 표시한다: 진행 중, 승인 대기, 실패, 읽지 않음.
import { useMemo, useState } from "react";

import type { ConnectionState } from "../../shared/bridge";
import { Icon } from "../design/icons";
import { useDesktopStore, type WorkspaceFolder, type WorkspaceThread } from "../session/desktopStore";
import { MOD_KEY, showHome, showView, toggleTheme, useUiStore } from "../session/uiStore";
import { allThreads, folderName, needsAttention, relativeAge, threadMark, type ThreadMark, type ThreadMarkSource } from "./threadStatus";

const PHASE_LABEL: Record<ConnectionState["phase"], string> = {
  connecting: "연결 중",
  connected: "연결됨",
  disconnected: "끊김",
};

const MARK_LABEL: Record<ThreadMark, string> = {
  running: "진행 중",
  approval: "승인 대기",
  unread: "읽지 않음",
  failed: "실패",
  idle: "",
};

export function StatusMark({ mark }: { mark: ThreadMark }) {
  return <span className={`status-mark status-mark-${mark}`} role="img" aria-label={MARK_LABEL[mark]} title={MARK_LABEL[mark]} />;
}

/** 상태 계산에 쓰는 저장소 값이다. 대화 내용이 바뀔 때는 새 객체를 만들지 않는다. */
export function useThreadMarks(): ThreadMarkSource {
  const activeThreadIds = useDesktopStore((state) => state.activeThreadIds);
  const unreadThreadIds = useDesktopStore((state) => state.unreadThreadIds);
  const pendingApprovals = useDesktopStore((state) => state.pendingApprovals);
  return useMemo(() => ({ activeThreadIds, unreadThreadIds, pendingApprovals }), [activeThreadIds, unreadThreadIds, pendingApprovals]);
}

function ThreadRow({ thread, onOpenThread }: { thread: WorkspaceThread; onOpenThread(threadId: string): void }) {
  const open = useDesktopStore((state) => state.openThreadId === thread.id);
  const selected = useUiStore((state) => state.view === "thread") && open;
  const mark = useDesktopStore((state) => threadMark(thread, state));
  return (
    <li className="thread-row">
      <button type="button" className={selected ? "thread selected" : "thread"} onClick={() => onOpenThread(thread.id)} data-testid="thread-entry">
        <StatusMark mark={mark} />
        <span className="thread-text">
          <span className={mark === "unread" ? "thread-title unread" : "thread-title"}>{thread.title}</span>
          {thread.branch && (
            <span className="thread-sub">
              <Icon name="branch" size={11} />
              <span className="thread-branch">{thread.branch}</span>
            </span>
          )}
        </span>
        <span className="thread-age">
          {mark === "approval" ? (
            <span className="badge badge-warn" data-testid="approval-badge">
              승인
            </span>
          ) : (
            relativeAge(thread.updatedAt, Date.now())
          )}
        </span>
      </button>
    </li>
  );
}

function FolderSection({ folder, connected, onOpenThread }: { folder: WorkspaceFolder; connected: boolean; onOpenThread(threadId: string): void }) {
  const [open, setOpen] = useState(true);
  return (
    <section className="project">
      <header className="project-header" title={folder.cwd}>
        <button type="button" className="project-name" onClick={() => setOpen(!open)} aria-expanded={open}>
          <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />
          <Icon name="folder" size={14} />
          <span>{folderName(folder.cwd)}</span>
        </button>
        <button type="button" className="icon-button" title="이 폴더에서 새 스레드" onClick={() => showHome(folder.cwd)} disabled={!connected} data-testid="new-thread">
          <Icon name="plus" size={14} />
        </button>
      </header>
      {open && (
        <ul>
          {folder.threads.map((thread) => (
            <ThreadRow key={thread.id} thread={thread} onOpenThread={onOpenThread} />
          ))}
        </ul>
      )}
    </section>
  );
}

export function Sidebar({ onOpenThread }: { onOpenThread(threadId: string): void }) {
  const folders = useDesktopStore((state) => state.folders);
  const connection = useDesktopStore((state) => state.connection);
  const codexVersion = useDesktopStore((state) => state.gateway?.codexVersion ?? null);
  const marks = useThreadMarks();
  const attention = useMemo(() => allThreads(folders).filter((thread) => needsAttention(threadMark(thread, marks))).length, [folders, marks]);
  const view = useUiStore((state) => state.view);
  const theme = useUiStore((state) => state.theme);
  const phase = connection?.phase ?? "connecting";

  return (
    <nav className="sidebar" data-testid="folders">
      <div className="sidebar-drag" />
      <div className="sidebar-nav">
        <button type="button" className={view === "home" ? "nav-item selected" : "nav-item"} onClick={() => showHome()} data-testid="new-folder-thread">
          <Icon name="pencil" />
          <span>새 스레드</span>
          <kbd>{MOD_KEY}N</kbd>
        </button>
        <button type="button" className={view === "fleet" ? "nav-item selected" : "nav-item"} onClick={() => showView("fleet")}>
          <Icon name="grid" />
          <span>현황판</span>
          {attention > 0 && <span className="badge badge-warn">{attention}</span>}
        </button>
        <button type="button" className="nav-item" onClick={() => useUiStore.setState({ paletteOpen: true })}>
          <Icon name="search" />
          <span>검색과 명령</span>
          <kbd>{MOD_KEY}K</kbd>
        </button>
      </div>
      <div className="sidebar-scroll">
        {folders.length === 0 && <p className="empty-row">아직 스레드가 없습니다. 새 스레드에서 작업 폴더를 고르세요.</p>}
        {folders.map((folder) => (
          <FolderSection key={folder.cwd} folder={folder} connected={phase === "connected"} onOpenThread={onOpenThread} />
        ))}
      </div>
      <footer className="sidebar-footer">
        <span className="connection" title="Gateway 연결 상태">
          <span className={`connection-dot connection-${phase}`} />
          <span data-testid="connection-phase">{PHASE_LABEL[phase]}</span>
          {phase === "connected" && codexVersion && <span data-testid="codex-version"> · {codexVersion}</span>}
          {phase === "disconnected" && connection?.retryInMs !== undefined && <span> · {Math.round(connection.retryInMs / 1000)}초 뒤 다시 연결</span>}
        </span>
        <button type="button" className="icon-button" title="단축키" onClick={() => useUiStore.setState({ shortcutsOpen: true })}>
          <Icon name="keyboard" />
        </button>
        <button type="button" className="icon-button" title="밝은 화면 / 어두운 화면" onClick={toggleTheme}>
          <Icon name={theme === "dark" ? "sun" : "moon"} />
        </button>
      </footer>
    </nav>
  );
}
