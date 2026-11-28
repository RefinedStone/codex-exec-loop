// 사이드바: 작업 폴더(스레드 cwd)별 스레드 목록이다 (D-16). 스레드마다 상태를 표시한다: 진행 중, 승인 대기, 실패, 읽지 않음.
import { useCallback, useMemo, useState } from "react";

import type { ConnectionState } from "../../shared/bridge";
import { Icon } from "../design/icons";
import { remoteHostKey, remoteHostStore, useDesktopStore, type WorkspaceFolder, type WorkspaceThread } from "../session/desktopStore";
import { closeChat, closeDot, isDotDockVisible, MOD_KEY, showChat, showDot, showHome, showView, toast, toggleTheme, useUiStore } from "../session/uiStore";
import { limitWindows, resetLabel, useAccountLimits, windowLabel } from "../session/accountLimits";
import { DotAvatar } from "../views/DotChat";
import { DiagnosticsDialog } from "../views/DiagnosticsDialog";
import { useZoomFactor, zoomStyle } from "../views/useZoom";
import { type ThreadActions, ThreadMenu, ThreadRenameInput } from "./ThreadMenu";
import { allThreads, archiveBlocked, folderName, needsAttention, relativeAge, threadMark, type ThreadMark, type ThreadMarkSource } from "./threadStatus";

export type { ThreadActions } from "./ThreadMenu";

function failure(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

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

function ThreadRow({ thread, folder, actions, onArchived, onOpenThread }: {
  thread: WorkspaceThread;
  folder: string;
  actions?: ThreadActions;
  onArchived(thread: WorkspaceThread): void;
  onOpenThread(threadId: string): void;
}) {
  const open = useDesktopStore((state) => state.openThreadId === thread.id);
  const selected = useUiStore((state) => state.view === "thread") && open;
  const mark = useDesktopStore((state) => threadMark(thread, state));
  const turnActive = useDesktopStore((state) => archiveBlocked(thread.id, state));
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState(false);
  const closeMenu = useCallback(() => setMenu(null), []);
  if (renaming && actions) {
    return (
      <li className="thread-row">
        <ThreadRenameInput title={thread.title} onCancel={() => setRenaming(false)} onSubmit={(name) => {
          setRenaming(false);
          actions.rename(thread.id, name).catch((cause: unknown) => toast(`이름을 바꾸지 못했습니다: ${failure(cause)}`));
        }} />
      </li>
    );
  }
  return (
    <li className="thread-row">
      {menu && actions && (
        <ThreadMenu thread={thread} folder={folder} running={turnActive} position={menu} onClose={closeMenu}
          onRename={() => setRenaming(true)}
          onArchive={() => actions.archive(thread.id).then(() => onArchived(thread), (cause: unknown) => toast(`보관하지 못했습니다: ${failure(cause)}`))}
          onOpenFolder={() => actions.openFolder(folder).then((opened) => { if (!opened) toast("폴더를 열지 못했습니다."); })} />
      )}
      <button type="button" className={selected ? "thread selected" : "thread"} onClick={() => onOpenThread(thread.id)} data-testid="thread-entry"
        onContextMenu={actions ? (event) => { event.preventDefault(); setMenu({ x: event.clientX, y: event.clientY }); } : undefined}>
        <StatusMark mark={mark} />
        <span className="thread-text">
          <span className={mark === "unread" ? "thread-title unread" : "thread-title"}>
            {thread.forkedFromId && <span className="badge badge-fork" title="다른 대화에서 분기한 대화입니다" data-testid="fork-badge">분기</span>}
            {thread.title}
          </span>
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

/** 계정 사용 한도 한 줄이다 (#2821). 서버가 준 창만 보인다. 받지 못했으면 이유만 짧게 쓴다. */
function AccountLimitsLine() {
  const { status, snapshot } = useAccountLimits();
  const windows = limitWindows(snapshot);
  if (status === "unknown" || status === "unsupported") return null;
  if (status === "unavailable" || windows.length === 0) {
    return <p className="account-limits muted" data-testid="account-limits">사용 한도를 확인하지 못했습니다.</p>;
  }
  return (
    <p className="account-limits" data-testid="account-limits">
      {windows.map((window) => {
        const reset = resetLabel(window);
        return (
          <span key={windowLabel(window)} className={window.usedPercent >= 90 ? "limit-window limit-high" : "limit-window"}>
            {windowLabel(window)} {Math.round(window.usedPercent)}%{reset ? ` · ${reset} 초기화` : ""}
          </span>
        );
      })}
    </p>
  );
}

function FolderSection({ folder, connected, actions, onArchived, onOpenThread }: {
  folder: WorkspaceFolder;
  connected: boolean;
  actions?: ThreadActions;
  onArchived(thread: WorkspaceThread): void;
  onOpenThread(threadId: string): void;
}) {
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
            <ThreadRow key={thread.id} thread={thread} folder={folder.cwd} actions={connected ? actions : undefined} onArchived={onArchived} onOpenThread={onOpenThread} />
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
 * Dots 구역이다 (D-19). dot 은 사용자의 비서이다: 누르면 대화가 옆 칸에 열리고, 보던 화면은 그대로 남는다. 다시
 * 누르면 닫힌다. 그 dot 의 창이 따로 떠 있으면 그 창을 앞으로 가져온다. 사용자가 만든 클라우드 스레드와는 다른
 * 것이므로 구역을 따로 둔다. dot 이 만든 작업 대화는 접어 둔다.
 */
function DotsSection({ host, onOpenThread }: { host: string; onOpenThread(host: string, threadId: string): void }) {
  const { useStore, phase, threads, openThreadId, showing } = useRemoteHost(host);
  const [open, setOpen] = useState(true);
  const [workOpen, setWorkOpen] = useState(false);
  const dots = useStore((state) => state.dots);
  const dotLive = useStore((state) => state.dotLive);
  const dotUnreadRooms = useStore((state) => state.dotUnreadRooms);
  const shownDotId = useUiStore((state) => {
    const dock = state.dotDock;
    return dock !== null && isDotDockVisible(state, host, dock.dotId) ? dock.dotId : null;
  });
  const toggleDot = (dotId: string) => {
    if (shownDotId === dotId) {
      closeDot();
      return;
    }
    window.akra.dots.hasWindow(host, dotId).then(
      (inWindow) => (inWindow ? void window.akra.dots.openWindow(host, dotId) : showDot(host, dotId)),
      () => showDot(host, dotId),
    );
  };
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
                <button type="button" className={shownDotId === dot.id ? "thread selected dot-entry" : "thread dot-entry"} title={`${dot.displayName} 과의 대화를 옆에 열거나 닫습니다.`} onClick={() => toggleDot(dot.id)} aria-pressed={shownDotId === dot.id} data-testid="remote-dot-entry">
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
 * ChatGPT 구역이다 (plan 25). 사용자의 ChatGPT 개인 대화를 최신 것부터 보인다. 누르면 읽기 전용 대화가 옆 칸에
 * 열리고, 다시 누르면 닫힌다. dot 과 같은 옆 칸을 쓴다. chats 가 없는 Gateway 이면 구역을 숨긴다. 읽지 못했거나
 * 낡은 목록이면(조회 제한 등) 이유를 보이고, 머리줄의 단추로 다시 읽는다. 자동으로 다시 읽지 않는다: 요청이 제한을 늘린다.
 */
function ChatsSection({ host, onReload }: { host: string; onReload(host: string): void }) {
  const { useStore, phase } = useRemoteHost(host);
  const [open, setOpen] = useState(true);
  const chats = useStore((state) => state.chats);
  const available = useStore((state) => state.chatsAvailable);
  const problem = useStore((state) => state.chatsProblem);
  const shownChatId = useUiStore((state) => (state.chatDock?.host === host ? state.chatDock.chatId : null));
  const toggleChat = (chatId: string) => (shownChatId === chatId ? closeChat() : showChat(host, chatId));
  // chats 가 없는 Gateway 에서만 숨긴다. 빈 목록에는 머리줄과 다시 읽기 단추를 남긴다: 새 대화를 읽을 수 있다.
  if (!available && problem === null) return null;
  return (
    <section className="project" data-testid="chats-section">
      <header className="project-header" title="내 ChatGPT 대화 (읽기 전용)">
        <button type="button" className="project-name" onClick={() => setOpen(!open)} aria-expanded={open}>
          <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />
          <Icon name="message" size={14} />
          <span>ChatGPT</span>
        </button>
        {phase !== "connected" && <span className="connection">{PHASE_LABEL[phase]}</span>}
        <button type="button" className="icon-button" title="ChatGPT 대화 목록 다시 읽기" aria-label="ChatGPT 대화 목록 다시 읽기" onClick={() => onReload(host)} disabled={phase !== "connected"} data-testid="chats-reload">
          <Icon name="refresh" size={14} />
        </button>
      </header>
      {open && problem && (
        <p className="empty-row error" data-testid="chats-problem">
          {problem}
        </p>
      )}
      {open && !problem && chats.length === 0 && <p className="empty-row">대화가 없습니다.</p>}
      {open && (
        <ul data-testid="remote-chats">
          {chats.map((chat) => (
            <li className="thread-row" key={chat.id}>
              <button type="button" className={shownChatId === chat.id ? "thread selected" : "thread"} title={`${chat.title} 을(를) 옆에 열거나 닫습니다. (읽기 전용)`} onClick={() => toggleChat(chat.id)} aria-pressed={shownChatId === chat.id} data-testid="remote-chat-entry">
                <StatusMark mark="idle" />
                <span className="thread-text">
                  <span className="thread-title">{chat.title}</span>
                </span>
              </button>
            </li>
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
        {/* 연결됐을 때는 점만 보인다. 글자는 화면 읽기 프로그램에만 준다. 좁은 머리줄에서 "읽기 전용"과 겹치지 않는다. */}
        <span className="connection connection-compact" title={`이 호스트의 연결 상태: ${PHASE_LABEL[phase]}`}>
          <span className={`connection-dot connection-${phase}`} />
          <span className={phase === "connected" ? "visually-hidden" : undefined} data-testid="remote-phase">
            {PHASE_LABEL[phase]}
          </span>
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

export function Sidebar({ onOpenThread, onOpenRemoteThread, onReloadChats, threadActions }: {
  onOpenThread(threadId: string): void;
  onOpenRemoteThread(host: string, threadId: string): void;
  onReloadChats(host: string): void;
  /** 로컬 스레드의 이름 바꾸기·보관이다 (#2841). 없으면 메뉴를 보이지 않는다. */
  threadActions?: ThreadActions;
}) {
  const [archived, setArchived] = useState<WorkspaceThread | null>(null);
  const [undoing, setUndoing] = useState(false);
  const diagnosticOpen = useUiStore(state => state.overlay === "diagnostics");
  const folders = useDesktopStore((state) => state.folders);
  // 원격 구역은 사용자가 켰고 Gateway 에 원격 호스트가 있을 때만 보인다.
  const remoteAgents = useUiStore((state) => state.remoteAgents);
  const remoteHosts = useDesktopStore(remoteHostKey);
  const connection = useDesktopStore((state) => state.connection);
  const codexVersion = useDesktopStore((state) => state.gateway?.codexVersion ?? null);
  const marks = useThreadMarks();
  const kernel = useDesktopStore((state) => state.kernel);
  const attention = useMemo(() => {
    const items = kernel.phase === "attached" ? kernel.attentionItems ?? [] : [];
    const kernelThreads = new Set(items.filter(item => item.subject.kind === "thread").map(item => item.subject.id));
    // User-turn failures and unread states still come from the thread store.
    return items.length + allThreads(folders).filter(thread => !kernelThreads.has(thread.id) && needsAttention(threadMark(thread, marks))).length;
  }, [folders, marks, kernel.phase, kernel.attentionItems]);
  const view = useUiStore((state) => state.view);
  const theme = useUiStore((state) => state.theme);
  const phase = connection?.phase ?? "connecting";
  const zoom = useZoomFactor("sidebar");
  // macOS 창 단추 밑의 끌기 칸은 위 막대의 높이를 따른다. 사이드바 배율로 나누어 두 배율이 달라도 단추가 항목과 겹치지 않는다.
  const dragScale = useZoomFactor("topbar") / zoom;

  return (
    <nav className="sidebar" style={zoomStyle(zoom)} data-zoom-scope="sidebar" data-testid="folders">
      <div className="sidebar-drag" style={dragScale === 1 ? undefined : ({ "--drag-scale": dragScale } as React.CSSProperties)} />
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
        <button type="button" className="nav-item" onClick={() => useUiStore.setState({ overlay: "palette" })}>
          <Icon name="search" />
          <span>검색과 명령</span>
          <kbd>{MOD_KEY}K</kbd>
        </button>
      </div>
      <div className="sidebar-scroll">
        {/* 원격 구역은 맨 위에 둔다. 로컬 스레드가 많으면 아래에 둔 구역은 스크롤해야 보여서 없는 것처럼 보인다. */}
        {remoteAgents && remoteHosts && remoteHosts.split(",").map((host) => <DotsSection key={`dots:${host}`} host={host} onOpenThread={onOpenRemoteThread} />)}
        {remoteAgents && remoteHosts && remoteHosts.split(",").map((host) => <ChatsSection key={`chats:${host}`} host={host} onReload={onReloadChats} />)}
        {remoteAgents && remoteHosts && remoteHosts.split(",").map((host) => <RemoteSection key={host} host={host} onOpenThread={onOpenRemoteThread} />)}
        {folders.length === 0 && <p className="empty-row">아직 스레드가 없습니다. 새 스레드에서 작업 폴더를 고르세요.</p>}
        {archived && threadActions && (
          <div className="archived-notice" role="status" data-testid="archived-notice">
            <span>‘{archived.title}’ 을(를) 보관했습니다.</span>
            <button type="button" className="button" data-testid="archived-undo" disabled={undoing} onClick={() => {
              const thread = archived;
              setUndoing(true);
              // 실패하면 알림을 남겨 다시 누를 수 있게 한다. 보관한 스레드를 되돌릴 다른 화면이 없다.
              threadActions.unarchive(thread.id).then(
                () => setArchived((current) => (current === thread ? null : current)),
                (cause: unknown) => toast(`보관을 되돌리지 못했습니다: ${failure(cause)}`),
              ).finally(() => setUndoing(false));
            }}>되돌리기</button>
            <button type="button" className="icon-button" title="닫기" onClick={() => setArchived(null)}><Icon name="close" size={12} /></button>
          </div>
        )}
        {folders.map((folder) => (
          <FolderSection key={folder.cwd} folder={folder} connected={phase === "connected"} actions={threadActions} onArchived={setArchived} onOpenThread={onOpenThread} />
        ))}
      </div>
      <AccountLimitsLine />
      <footer className="sidebar-footer">
        <button type="button" className="connection diagnostic-connection" title="Gateway 연결 상태 · 로컬 연결 진단 저장" aria-label="연결 진단 저장" data-testid="diagnostics-open" onClick={() => useUiStore.setState({ overlay: "diagnostics" })}>
          <span className={`connection-dot connection-${phase}`} />
          <span data-testid="connection-phase">{PHASE_LABEL[phase]}</span>
          {phase === "connected" && codexVersion && <span data-testid="codex-version"> · {codexVersion}</span>}
          {phase === "disconnected" && connection?.retryInMs !== undefined && <span> · {Math.round(connection.retryInMs / 1000)}초 뒤 다시 연결</span>}
        </button>
        <button type="button" className="icon-button" title="단축키" onClick={() => useUiStore.setState({ overlay: "shortcuts" })}>
          <Icon name="keyboard" />
        </button>
        <button type="button" className="icon-button" title={`설정 (${MOD_KEY},)`} onClick={() => useUiStore.setState({ overlay: "settings" })} data-testid="settings-open">
          <Icon name="sliders" />
        </button>
        <button type="button" className="icon-button" title="밝은 화면 / 어두운 화면" onClick={toggleTheme}>
          <Icon name={theme === "dark" ? "sun" : "moon"} />
        </button>
      </footer>
      {diagnosticOpen && <DiagnosticsDialog />}
    </nav>
  );
}
