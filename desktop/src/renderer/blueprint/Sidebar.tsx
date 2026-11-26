// 사이드바: 고정한 스레드, 스레드 목록, 상태 표시이다. 목록은 프로젝트별이나 주의 묶음(내 차례, 읽을 것, 작업 중, 나머지)으로 묶는다.
import { useState } from "react";

import { Icon } from "../design/icons";
import { ATTENTION_GROUPS, attentionOf, isListed } from "./flow";
import { PROJECTS, type MockThread, type ThreadStatus } from "./mockData";
import { markDone, notConnected, openThread, restoreThread, showView, snooze, togglePinned, toggleTheme, useAttentionList, useBlueprint, useDelivery, type SidebarGroup } from "./store";

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

function projectName(projectId: string): string {
  return PROJECTS.find((project) => project.id === projectId)?.name ?? projectId;
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

/** `showProject` 는 주의 묶음으로 볼 때 켠다. 그때는 줄이 어느 프로젝트의 것인지 보여야 한다. */
function ThreadRow({ thread, showProject = false }: { thread: MockThread; showProject?: boolean }) {
  const selected = useBlueprint((state) => state.view === "thread" && state.openThreadId === thread.id);
  const approval = useBlueprint((state) => state.approvals[thread.id]);
  const attention = attentionOf(thread, approval, useDelivery(thread));
  // 사람을 기다리는 줄은 브랜치 대신 이유를 보인다 (plan 23 의 이유 1 줄).
  const reasoned = attention.group === "mine" || attention.group === "read";
  return (
    <li className="thread-row">
      <button type="button" className={selected ? "thread selected" : "thread"} title={thread.branch} onClick={() => openThread(thread.id)}>
        <StatusMark status={thread.status} />
        <span className="thread-text">
          <span className={thread.status === "unread" ? "thread-title unread" : "thread-title"}>{thread.title}</span>
          <span className="thread-sub">
            {showProject && <span className="thread-project">{projectName(thread.projectId)} ·</span>}
            {reasoned ? (
              <span className={attention.group === "mine" ? "thread-reason mine" : "thread-reason"}>{attention.reason}</span>
            ) : (
              <>
                {thread.worktree && <Icon name="branch" size={11} />}
                <span className="thread-branch">{thread.branch}</span>
                {thread.added + thread.removed > 0 && (
                  <span className="counts">
                    <span className="count-add">+{thread.added}</span> <span className="count-del">−{thread.removed}</span>
                  </span>
                )}
              </>
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

/** 주의 묶음으로 본 목록이다. 빈 묶음은 그리지 않는다. */
function AttentionSections() {
  const rows = useAttentionList().filter(({ thread }) => !thread.pinned);
  return (
    <>
      {ATTENTION_GROUPS.map((group) => {
        const mine = rows.filter(({ attention }) => attention.group === group.id);
        if (mine.length === 0) return null;
        return (
          <section key={group.id} className="project">
            <header className="project-header" title={group.hint}>
              <span className="section-label">
                {group.title} {mine.length}
              </span>
            </header>
            <ul>
              {mine.map(({ thread }) => (
                <ThreadRow key={thread.id} thread={thread} showProject />
              ))}
            </ul>
          </section>
        );
      })}
    </>
  );
}

const GROUP_LABEL: Record<SidebarGroup, string> = { project: "프로젝트", attention: "상태" };

export function Sidebar() {
  const view = useBlueprint((state) => state.view);
  const theme = useBlueprint((state) => state.theme);
  const group = useBlueprint((state) => state.sidebarGroup);
  const threads = useBlueprint((state) => state.threads);
  const pinned = threads.filter((thread) => thread.pinned && isListed(thread));
  const inTurn = useAttentionList().filter(({ attention }) => attention.group === "mine").length;
  return (
    <nav className="sidebar">
      <div className="sidebar-drag" />
      <div className="sidebar-nav">
        <button type="button" className={view === "home" ? "nav-item selected" : "nav-item"} onClick={() => showView("home")}>
          <Icon name="pencil" />
          <span>새 스레드</span>
          <kbd>⌘N</kbd>
        </button>
        <button type="button" className={view === "fleet" ? "nav-item selected" : "nav-item"} title="내 차례인 스레드의 수" onClick={() => showView("fleet")}>
          <Icon name="grid" />
          <span>현황판</span>
          {inTurn > 0 && <span className="badge badge-warn">{inTurn}</span>}
        </button>
        <button type="button" className="nav-item" onClick={() => useBlueprint.setState({ paletteOpen: true })}>
          <Icon name="search" />
          <span>검색과 명령</span>
          <kbd>⌘K</kbd>
        </button>
      </div>
      <div className="sidebar-scroll">
        <div className="segmented sidebar-group" role="group" aria-label="목록 묶기">
          {(Object.keys(GROUP_LABEL) as SidebarGroup[]).map((id) => (
            <button key={id} type="button" className={group === id ? "selected" : undefined} aria-pressed={group === id} onClick={() => useBlueprint.setState({ sidebarGroup: id })}>
              {GROUP_LABEL[id]}
            </button>
          ))}
        </div>
        {pinned.length > 0 && (
          <section className="project">
            <header className="project-header">
              <span className="section-label">고정</span>
            </header>
            <ul>
              {pinned.map((thread) => (
                <ThreadRow key={thread.id} thread={thread} showProject={group === "attention"} />
              ))}
            </ul>
          </section>
        )}
        {group === "project" ? PROJECTS.map((project) => <ProjectSection key={project.id} projectId={project.id} name={project.name} path={project.path} />) : <AttentionSections />}
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
