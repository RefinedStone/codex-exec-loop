// 사이드바: 작업 폴더(스레드 cwd)별 스레드 목록이다 (D-16). 스레드마다 상태를 표시한다: 진행 중, 승인 대기, 실패, 읽지 않음.
import { useMemo, useState } from "react";

import type { ConnectionState } from "../../shared/bridge";
import { Icon } from "../design/icons";
import { remoteHostKey, remoteHostStore, useDesktopStore, type WorkspaceFolder, type WorkspaceThread } from "../session/desktopStore";
import { MOD_KEY, showDot, showHome, showView, toggleTheme, useUiStore } from "../session/uiStore";
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

type DotRow = { paused: boolean; status: string; messagingRoomId: string | null };

/** dot 줄의 표시이다: 답을 만드는 중이면 진행, 안 본 답이 있으면 안 읽음이다. */
function dotRowMark(dot: DotRow, live: Record<string, { connected: boolean; working: boolean }>, unread: readonly string[]): ThreadMark {
  const room = dot.messagingRoomId;
  if (room !== null && live[room]?.connected && live[room].working) return "running";
  return room !== null && unread.includes(room) ? "unread" : "idle";
}

function dotRowNote(dot: DotRow, live: Record<string, { connected: boolean; working: boolean }>, unread: readonly string[]): string {
  if (dot.paused) return "멈춤";
  const mark = dotRowMark(dot, live, unread);
  return mark === "running" ? "답하는 중" : mark === "unread" ? "새 답" : dot.status;
}

function RemoteThreadRow({ thread, selected, onOpen, testId, title }: { thread: WorkspaceThread; selected: boolean; onOpen(): void; testId: string; title: string }) {
  return (
    <li className="thread-row">
      <button type="button" className={selected ? "thread selected" : "thread"} onClick={onOpen} title={title} data-testid={testId}>
        <StatusMark mark="idle" />
        <span className="thread-text">
          <span className="thread-title">{thread.title}</span>
        </span>
        <span className="thread-age">{relativeAge(thread.updatedAt, Date.now())}</span>
      </button>
    </li>
  );
}

/**
 * 원격 호스트(클라우드 에이전트, plan 25) 하나의 구역이다. 그 호스트의 저장소를 읽는다. 원격의 작업 폴더는 이 컴퓨터에
 * 없으므로 폴더로 묶지 않고 최근 대화부터 보인다.
 */
function RemoteSection({ host, onOpenThread }: { host: string; onOpenThread(host: string, threadId: string): void }) {
  const useStore = remoteHostStore(host);
  const [open, setOpen] = useState(true);
  const phase = useStore((state) => state.connection?.phase ?? "connecting");
  const problem = useStore((state) => (state.connection?.phase === "connected" ? state.sessionError : (state.connection?.detail ?? null)));
  const folders = useStore((state) => state.folders);
  const dots = useStore((state) => state.dots);
  const dotLive = useStore((state) => state.dotLive);
  const dotUnreadRooms = useStore((state) => state.dotUnreadRooms);
  const openThreadId = useStore((state) => state.openThreadId);
  const showing = useUiStore((state) => state.view === "remote" && state.remoteHost === host);
  const shownDotId = useUiStore((state) => (state.view === "dot" && state.remoteHost === host ? state.remoteDotId : null));
  const threads = useMemo(() => folders.flatMap((folder) => folder.threads).sort((left, right) => right.updatedAt - left.updatedAt), [folders]);
  const agentThreads = useMemo(() => threads.filter((thread) => thread.agentThread === true), [threads]);
  const cloudThreads = useMemo(() => threads.filter((thread) => thread.agentThread !== true), [threads]);
  return (
    <section className="project" data-testid="remote-host">
      <header className="project-header" title={`Gateway 호스트 ${host} (읽기 전용)`}>
        <button type="button" className="project-name" onClick={() => setOpen(!open)} aria-expanded={open}>
          <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />
          <Icon name="globe" size={14} />
          <span>{host === "cloud" ? "클라우드" : host}</span>
        </button>
        <span className="connection" title="이 호스트의 연결 상태">
          <span className={`connection-dot connection-${phase}`} />
          <span data-testid="remote-phase">{PHASE_LABEL[phase]}</span>
        </span>
      </header>
      {open && problem && <p className="empty-row error">{problem}</p>}
      {/* 에이전트(dot)와 사용자가 만든 클라우드 스레드는 다른 것이다. 같은 호스트에 있어도 따로 보인다. */}
      {open && (dots.length > 0 || agentThreads.length > 0) && (
        <>
          <p className="section-label empty-row">에이전트</p>
          <ul data-testid="remote-dots">
            {dots.map((dot) => (
              <li className="thread-row" key={dot.id}>
                <button
                  type="button"
                  className={shownDotId === dot.id ? "thread selected" : "thread"}
                  title={`${dot.displayName} 과의 대화를 엽니다.`}
                  onClick={() => showDot(host, dot.id)}
                  data-testid="remote-dot-entry"
                >
                  <StatusMark mark={dotRowMark(dot, dotLive, dotUnreadRooms)} />
                  <span className="thread-text">
                    <span className="thread-title">{dot.displayName}</span>
                  </span>
                  <span className="thread-age">{dotRowNote(dot, dotLive, dotUnreadRooms)}</span>
                </button>
              </li>
            ))}
            {agentThreads.map((thread) => (
              <RemoteThreadRow key={thread.id} thread={thread} selected={showing && openThreadId === thread.id} onOpen={() => onOpenThread(host, thread.id)} testId="remote-agent-thread-entry" title="에이전트가 만든 작업 대화" />
            ))}
          </ul>
        </>
      )}
      {open && !problem && phase === "connected" && (
        <>
          <p className="section-label empty-row">클라우드 스레드</p>
          {cloudThreads.length === 0 && <p className="empty-row">대화가 없습니다.</p>}
          <ul>
            {cloudThreads.map((thread) => (
              <RemoteThreadRow key={thread.id} thread={thread} selected={showing && openThreadId === thread.id} onOpen={() => onOpenThread(host, thread.id)} testId="remote-thread-entry" title="사용자가 만든 클라우드 대화" />
            ))}
          </ul>
        </>
      )}
    </section>
  );
}

export function Sidebar({ onOpenThread, onOpenRemoteThread }: { onOpenThread(threadId: string): void; onOpenRemoteThread(host: string, threadId: string): void }) {
  const folders = useDesktopStore((state) => state.folders);
  // 원격 구역은 사용자가 켰고 Gateway 에 원격 호스트가 있을 때만 보인다.
  const remoteAgents = useUiStore((state) => state.remoteAgents);
  const remoteHosts = useDesktopStore(remoteHostKey);
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
        {/* 원격 구역은 맨 위에 둔다. 로컬 스레드가 많으면 아래에 둔 구역은 스크롤해야 보여서 없는 것처럼 보인다. */}
        {remoteAgents && remoteHosts && remoteHosts.split(",").map((host) => <RemoteSection key={host} host={host} onOpenThread={onOpenRemoteThread} />)}
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
