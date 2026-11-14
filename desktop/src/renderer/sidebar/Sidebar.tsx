// 사이드바: 작업 폴더(스레드 cwd)별 스레드 목록이다 (D-16). 스레드마다 상태를 표시한다: 진행 중, 승인 대기, 실패, 읽지 않음.
import { useMemo, useState } from "react";

import type { ConnectionState } from "../../shared/bridge";
import { Icon } from "../design/icons";
import { remoteHostKey, remoteHostStore, useDesktopStore, type WorkspaceFolder, type WorkspaceThread } from "../session/desktopStore";
import { MOD_KEY, showDot, showHome, showView, toggleTheme, useUiStore } from "../session/uiStore";
import { DotAvatar } from "../views/DotChat";
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

/** dot 줄 오른쪽의 짧은 글이다. 평소에는 비운다: 이름만 보이는 편이 깔끔하다. */
function dotRowNote(dot: DotRow, mark: ThreadMark): string {
  if (dot.paused) return "멈춤";
  return mark === "running" ? "답하는 중…" : "";
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

/** 원격 호스트의 저장소에서 사이드바의 두 구역이 쓰는 값을 읽는다. */
function useRemoteHost(host: string) {
  const useStore = remoteHostStore(host);
  const phase = useStore((state) => state.connection?.phase ?? "connecting");
  const problem = useStore((state) => (state.connection?.phase === "connected" ? state.sessionError : (state.connection?.detail ?? null)));
  const folders = useStore((state) => state.folders);
  const openThreadId = useStore((state) => state.openThreadId);
  const showing = useUiStore((state) => state.view === "remote" && state.remoteHost === host);
  const threads = useMemo(() => folders.flatMap((folder) => folder.threads).sort((left, right) => right.updatedAt - left.updatedAt), [folders]);
  return { useStore, phase, problem, threads, openThreadId, showing };
}

/**
 * Dots 구역이다 (D-19). dot 은 사용자의 비서이다: 누르면 대화가 열린다. 사용자가 만든 클라우드 스레드와는 다른
 * 것이므로 구역을 따로 둔다. dot 이 만든 작업 대화는 접어 둔다.
 */
function DotsSection({ host, onOpenThread }: { host: string; onOpenThread(host: string, threadId: string): void }) {
  const { useStore, phase, threads, openThreadId, showing } = useRemoteHost(host);
  const [open, setOpen] = useState(true);
  const [workOpen, setWorkOpen] = useState(false);
  const dots = useStore((state) => state.dots);
  const dotLive = useStore((state) => state.dotLive);
  const dotUnreadRooms = useStore((state) => state.dotUnreadRooms);
  const shownDotId = useUiStore((state) => (state.view === "dot" && state.remoteHost === host ? state.remoteDotId : null));
  const agentThreads = useMemo(() => threads.filter((thread) => thread.agentThread === true), [threads]);
  if (dots.length === 0 && agentThreads.length === 0) return null;
  return (
    <section className="project" data-testid="dots-section">
      <header className="project-header" title="내 클라우드 에이전트">
        <button type="button" className="project-name" onClick={() => setOpen(!open)} aria-expanded={open}>
          <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />
          <Icon name="sparkle" size={14} />
          <span>Dots</span>
        </button>
        {phase !== "connected" && <span className="connection">{PHASE_LABEL[phase]}</span>}
      </header>
      {open && (
        <ul data-testid="remote-dots">
          {dots.map((dot) => {
            const mark = dotRowMark(dot, dotLive, dotUnreadRooms);
            return (
              <li className="thread-row" key={dot.id}>
                <button type="button" className={shownDotId === dot.id ? "thread selected dot-entry" : "thread dot-entry"} title={`${dot.displayName} 과의 대화를 엽니다.`} onClick={() => showDot(host, dot.id)} data-testid="remote-dot-entry">
                  <DotAvatar name={dot.displayName} size={22} />
                  <span className="thread-text">
                    <span className="thread-title">{dot.displayName}</span>
                  </span>
                  {mark === "unread" && <span className="badge badge-warn" data-testid="dot-unread">새 답</span>}
                  {mark !== "unread" && <span className="thread-age">{dotRowNote(dot, mark)}</span>}
                </button>
              </li>
            );
          })}
          {agentThreads.length > 0 && (
            <li className="thread-row">
              <button type="button" className="thread" onClick={() => setWorkOpen(!workOpen)} aria-expanded={workOpen} title="에이전트가 만든 작업 대화이다. 읽기 전용이다." data-testid="dot-work-toggle">
                <Icon name={workOpen ? "chevronDown" : "chevronRight"} size={12} />
                <span className="thread-text">
                  <span className="thread-title muted">작업 대화</span>
                </span>
                <span className="thread-age">{agentThreads.length}</span>
              </button>
            </li>
          )}
          {workOpen &&
            agentThreads.map((thread) => (
              <RemoteThreadRow key={thread.id} thread={thread} selected={showing && openThreadId === thread.id} onOpen={() => onOpenThread(host, thread.id)} testId="remote-agent-thread-entry" title="에이전트가 만든 작업 대화 (읽기 전용)" />
            ))}
        </ul>
      )}
    </section>
  );
}

/**
 * 원격 호스트(plan 25)의 클라우드 스레드 구역이다. 사용자가 만든 클라우드 스레드를 최근 것부터 보인다. 읽기 전용이다.
 * 원격의 작업 폴더는 이 컴퓨터에 없으므로 폴더로 묶지 않는다.
 */
function RemoteSection({ host, onOpenThread }: { host: string; onOpenThread(host: string, threadId: string): void }) {
  const { phase, problem, threads, openThreadId, showing } = useRemoteHost(host);
  const [open, setOpen] = useState(true);
  const cloudThreads = useMemo(() => threads.filter((thread) => thread.agentThread !== true), [threads]);
  return (
    <section className="project" data-testid="remote-host">
      <header className="project-header" title={`Gateway 호스트 ${host} (읽기 전용)`}>
        <button type="button" className="project-name" onClick={() => setOpen(!open)} aria-expanded={open}>
          <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />
          <Icon name="globe" size={14} />
          <span className="project-fixed">{host === "cloud" ? "클라우드" : host}</span>
          <span className="project-note">읽기 전용</span>
        </button>
        <span className="connection" title="이 호스트의 연결 상태">
          <span className={`connection-dot connection-${phase}`} />
          <span data-testid="remote-phase">{PHASE_LABEL[phase]}</span>
        </span>
      </header>
      {open && problem && <p className="empty-row error">{problem}</p>}
      {open && !problem && phase === "connected" && cloudThreads.length === 0 && <p className="empty-row">대화가 없습니다.</p>}
      {open && (
        <ul>
          {cloudThreads.map((thread) => (
            <RemoteThreadRow key={thread.id} thread={thread} selected={showing && openThreadId === thread.id} onOpen={() => onOpenThread(host, thread.id)} testId="remote-thread-entry" title="사용자가 만든 클라우드 대화 (읽기 전용)" />
          ))}
        </ul>
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
        {remoteAgents && remoteHosts && remoteHosts.split(",").map((host) => <DotsSection key={`dots:${host}`} host={host} onOpenThread={onOpenRemoteThread} />)}
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
