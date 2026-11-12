// 사이드바: 고정한 스레드, 프로젝트별 스레드, 상태 표시이다. 상태는 진행 중, 승인 대기, 읽지 않음, 실패이다.
import { useState } from "react";

import { Icon } from "../design/icons";
import { PROJECTS, type MockThread, type ThreadStatus } from "./mockData";
import { markDone, notConnected, openThread, restoreThread, showView, snooze, togglePinned, toggleTheme, useBlueprint } from "./store";

const STATUS_LABEL: Record<ThreadStatus, string> = {
  running: "진행 중",
  approval: "승인 대기",
  unread: "읽지 않음",
  failed: "실패",
  idle: "",
};

export function StatusMark({ status }: { status: ThreadStatus }) {
  return <span className={`status-mark status-mark-${status}`} role="img" aria-label={STATUS_LABEL[status]} title={STATUS_LABEL[status]} />;
}

export function needsAttention(status: ThreadStatus): boolean {
  return status === "approval" || status === "failed";
}

/** 완료로 정리했거나 미룬 스레드는 목록에서 뺀다. */
function isListed(thread: MockThread): boolean {
  return !thread.done && thread.snoozedUntil === null;
}

/** 목록에서 뺀 스레드이다. 펼치면 되돌릴 수 있다. */
function SetAside() {
  const [open, setOpen] = useState(false);
  const threads = useBlueprint((state) => state.threads);
  const aside = threads.filter((thread) => !isListed(thread));
  if (aside.length === 0) return null;
  const done = aside.filter((thread) => thread.done).length;
  return (
    <section className="project">
      <header className="project-header">
        <button type="button" className="project-name" onClick={() => setOpen(!open)} aria-expanded={open}>
          <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />
          <span className="section-label">
            완료 {done} · 미룸 {aside.length - done}
          </span>
        </button>
      </header>
      {open && (
        <ul>
          {aside.map((thread) => (
            <li key={thread.id} className="aside-row">
              <button type="button" className="thread" onClick={() => openThread(thread.id)}>
                <Icon name={thread.done ? "check" : "clock"} size={12} />
                <span className="thread-text">
                  <span className="thread-title">{thread.title}</span>
                  <span className="thread-sub">{thread.done ? "완료" : `${thread.snoozedUntil ?? ""}에 돌아옴`}</span>
                </span>
              </button>
              <button type="button" className="ghost" onClick={() => restoreThread(thread.id)}>
                되돌리기
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function ThreadRow({ thread }: { thread: MockThread }) {
  const selected = useBlueprint((state) => state.view === "thread" && state.openThreadId === thread.id);
  return (
    <li className="thread-row">
      <button type="button" className={selected ? "thread selected" : "thread"} onClick={() => openThread(thread.id)}>
        <StatusMark status={thread.status} />
        <span className="thread-text">
          <span className={thread.status === "unread" ? "thread-title unread" : "thread-title"}>{thread.title}</span>
          <span className="thread-sub">
            {thread.worktree && <Icon name="branch" size={11} />}
            <span className="thread-branch">{thread.branch}</span>
            {thread.added + thread.removed > 0 && (
              <span className="counts">
                <span className="count-add">+{thread.added}</span> <span className="count-del">−{thread.removed}</span>
              </span>
            )}
          </span>
        </span>
        <span className="thread-age">{thread.status === "approval" ? <span className="badge badge-warn">승인</span> : thread.updated}</span>
      </button>
      <span className="row-actions">
        <button type="button" className={thread.pinned ? "pinned" : undefined} title={thread.pinned ? "고정 해제" : "고정"} onClick={() => togglePinned(thread.id)}>
          <Icon name="stack" size={13} />
        </button>
        <button type="button" title="내일까지 미루기" onClick={() => snooze(thread.id)}>
          <Icon name="clock" size={13} />
        </button>
        <button type="button" title="완료로 정리" onClick={() => markDone(thread.id)}>
          <Icon name="check" size={13} />
        </button>
      </span>
    </li>
  );
}

function ProjectSection({ projectId, name, path }: { projectId: string; name: string; path: string }) {
  const [open, setOpen] = useState(true);
  const threads = useBlueprint((state) => state.threads);
  const mine = threads.filter((thread) => thread.projectId === projectId && !thread.pinned && isListed(thread));
  return (
    <section className="project">
      <header className="project-header" title={path}>
        <button type="button" className="project-name" onClick={() => setOpen(!open)} aria-expanded={open}>
          <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />
          <Icon name="folder" size={14} />
          <span>{name}</span>
        </button>
        <button
          type="button"
          className="icon-button"
          title="이 프로젝트에서 새 스레드"
          onClick={() => {
            useBlueprint.setState({ homeProjectId: projectId });
            showView("home");
          }}
        >
          <Icon name="plus" size={14} />
        </button>
      </header>
      {open && (
        <ul>
          {mine.map((thread) => (
            <ThreadRow key={thread.id} thread={thread} />
          ))}
          {mine.length === 0 && <li className="empty-row">스레드가 없습니다</li>}
        </ul>
      )}
    </section>
  );
}

export function Sidebar() {
  const view = useBlueprint((state) => state.view);
  const theme = useBlueprint((state) => state.theme);
  const threads = useBlueprint((state) => state.threads);
  const pinned = threads.filter((thread) => thread.pinned && isListed(thread));
  const attention = threads.filter((thread) => needsAttention(thread.status)).length;
  return (
    <nav className="sidebar">
      <div className="sidebar-drag" />
      <div className="sidebar-nav">
        <button type="button" className={view === "home" ? "nav-item selected" : "nav-item"} onClick={() => showView("home")}>
          <Icon name="pencil" />
          <span>새 스레드</span>
          <kbd>⌘N</kbd>
        </button>
        <button type="button" className={view === "fleet" ? "nav-item selected" : "nav-item"} onClick={() => showView("fleet")}>
          <Icon name="grid" />
          <span>현황판</span>
          {attention > 0 && <span className="badge badge-warn">{attention}</span>}
        </button>
        <button type="button" className="nav-item" onClick={() => useBlueprint.setState({ paletteOpen: true })}>
          <Icon name="search" />
          <span>검색과 명령</span>
          <kbd>⌘K</kbd>
        </button>
      </div>
      <div className="sidebar-scroll">
        {pinned.length > 0 && (
          <section className="project">
            <header className="project-header">
              <span className="section-label">고정</span>
            </header>
            <ul>
              {pinned.map((thread) => (
                <ThreadRow key={thread.id} thread={thread} />
              ))}
            </ul>
          </section>
        )}
        {PROJECTS.map((project) => (
          <ProjectSection key={project.id} projectId={project.id} name={project.name} path={project.path} />
        ))}
        <SetAside />
        <button type="button" className="nav-item add-project" onClick={() => notConnected("프로젝트 추가")}>
          <Icon name="plus" />
          <span>프로젝트 추가</span>
        </button>
      </div>
      <footer className="sidebar-footer">
        <span className="connection" title="Gateway 연결 상태 (mock)">
          <span className="connection-dot" />
          연결됨 · codex 0.159
        </span>
        <button type="button" className="icon-button" title="단축키" onClick={() => useBlueprint.setState({ shortcutsOpen: true })}>
          <Icon name="keyboard" />
        </button>
        <button type="button" className="icon-button" title="밝은 화면 / 어두운 화면" onClick={toggleTheme}>
          <Icon name={theme === "dark" ? "sun" : "moon"} />
        </button>
      </footer>
    </nav>
  );
}
