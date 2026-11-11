// 대화 밖의 화면이다: 새 스레드(홈), 현황판, 명령 팔레트, 단축키 목록, 터미널, 알림.
import { useEffect, useMemo, useState } from "react";

import { Composer, Picker } from "./Composer";
import { Icon, type IconName } from "./icons";
import { PROJECTS, SUGGESTIONS, TERMINAL_LINES, type MockThread, type ThreadStatus } from "./mockData";
import { StatusMark } from "./Sidebar";
import { handOffToNewThread, notConnected, openThread, showView, togglePanel, toggleTheme, useBlueprint } from "./store";

function projectName(projectId: string): string {
  return PROJECTS.find((project) => project.id === projectId)?.name ?? projectId;
}

const RUN_PLACES = ["Worktree 에서 실행", "작업 폴더에서 실행"] as const;

export function Home() {
  const homeProjectId = useBlueprint((state) => state.homeProjectId);
  const homeWorktree = useBlueprint((state) => state.homeWorktree);
  const threads = useBlueprint((state) => state.threads);
  return (
    <div className="home">
      <div className="home-inner">
        <h1>무엇을 할까요?</h1>
        <div className="home-pickers">
          <Picker
            icon={<Icon name="folder" size={12} />}
            value={projectName(homeProjectId)}
            options={PROJECTS.map((project) => project.name)}
            onChange={(name) => useBlueprint.setState({ homeProjectId: PROJECTS.find((project) => project.name === name)?.id ?? homeProjectId })}
          />
          <Picker
            icon={<Icon name="branch" size={12} />}
            value={homeWorktree ? RUN_PLACES[0] : RUN_PLACES[1]}
            options={RUN_PLACES}
            onChange={(place) => useBlueprint.setState({ homeWorktree: place === RUN_PLACES[0] })}
          />
        </div>
        <Composer threadId={null} />
        <h3>제안</h3>
        <div className="suggestions">
          {SUGGESTIONS.map((suggestion) => (
            <button key={suggestion.title} type="button" className="suggestion" onClick={() => handOffToNewThread(`${suggestion.title} 작업을 해줘.`)}>
              <Icon name={suggestion.icon} />
              <strong>{suggestion.title}</strong>
              <span className="muted">{suggestion.source}</span>
            </button>
          ))}
        </div>
        <h3>최근 스레드</h3>
        <ul className="recent">
          {threads.slice(0, 4).map((thread) => (
            <li key={thread.id}>
              <button type="button" onClick={() => openThread(thread.id)}>
                <StatusMark status={thread.status} />
                <span className="recent-title">{thread.title}</span>
                <span className="muted">{projectName(thread.projectId)}</span>
                <span className="muted">{thread.updated}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

const FLEET_COLUMNS: { title: string; statuses: ThreadStatus[]; hint: string }[] = [
  { title: "확인 필요", statuses: ["approval", "failed"], hint: "승인 또는 실패 확인을 기다립니다" },
  { title: "진행 중", statuses: ["running"], hint: "에이전트가 작업합니다" },
  { title: "읽지 않음", statuses: ["unread"], hint: "끝났고 아직 보지 않았습니다" },
  { title: "최근", statuses: ["idle"], hint: "본 스레드입니다" },
];

function FleetCard({ thread }: { thread: MockThread }) {
  return (
    <button type="button" className="fleet-card" onClick={() => openThread(thread.id)}>
      <span className="fleet-card-title">
        <StatusMark status={thread.status} />
        {thread.title}
      </span>
      <span className="fleet-card-meta">
        <Icon name="folder" size={12} />
        {projectName(thread.projectId)}
        <Icon name="branch" size={12} />
        {thread.branch}
      </span>
      <span className="fleet-card-meta">
        {thread.added + thread.removed > 0 && (
          <span className="counts">
            <span className="count-add">+{thread.added}</span> <span className="count-del">−{thread.removed}</span>
          </span>
        )}
        {thread.pullRequest && (
          <span className={`badge badge-${thread.pullRequest.checks === "fail" ? "bad" : "ok"}`}>
            PR #{thread.pullRequest.number} · CI {thread.pullRequest.checks === "fail" ? "실패" : "통과"}
          </span>
        )}
        <span className="bar-spacer" />
        {thread.updated}
      </span>
    </button>
  );
}

export function Fleet() {
  const threads = useBlueprint((state) => state.threads);
  return (
    <div className="fleet">
      <header className="fleet-header">
        <h1>현황판</h1>
        <span className="muted">모든 프로젝트의 스레드를 상태별로 봅니다.</span>
      </header>
      <div className="fleet-columns">
        {FLEET_COLUMNS.map((column) => {
          const items = threads.filter((thread) => column.statuses.includes(thread.status));
          return (
            <section key={column.title} className="fleet-column">
              <h3 title={column.hint}>
                {column.title} <span className="muted">{items.length}</span>
              </h3>
              {items.map((thread) => (
                <FleetCard key={thread.id} thread={thread} />
              ))}
              {items.length === 0 && <p className="muted fleet-empty">없음</p>}
            </section>
          );
        })}
      </div>
    </div>
  );
}

interface PaletteEntry {
  id: string;
  icon: IconName;
  label: string;
  hint: string;
  run(): void;
}

function paletteEntries(threads: readonly MockThread[]): PaletteEntry[] {
  const commands: PaletteEntry[] = [
    { id: "new", icon: "pencil", label: "새 스레드", hint: "⌘N", run: () => showView("home") },
    { id: "fleet", icon: "grid", label: "현황판 열기", hint: "", run: () => showView("fleet") },
    { id: "review", icon: "diff", label: "변경 검토 패널", hint: "⌘\\", run: () => togglePanel("review") },
    { id: "kernel", icon: "bolt", label: "Akra 커널 패널", hint: "", run: () => togglePanel("kernel") },
    { id: "browser", icon: "globe", label: "브라우저 패널", hint: "", run: () => togglePanel("browser") },
    { id: "work", icon: "briefcase", label: "업무 패널 (GitHub, Jira, Notion)", hint: "", run: () => togglePanel("work") },
    { id: "terminal", icon: "terminal", label: "터미널 열기 / 닫기", hint: "⌘J", run: () => useBlueprint.setState((state) => ({ terminalOpen: !state.terminalOpen, paletteOpen: false })) },
    { id: "theme", icon: "sun", label: "밝은 화면 / 어두운 화면", hint: "", run: toggleTheme },
  ];
  const threadEntries = threads.map((thread) => ({
    id: thread.id,
    icon: "message" as const,
    label: thread.title,
    hint: projectName(thread.projectId),
    run: () => openThread(thread.id),
  }));
  return [...commands, ...threadEntries];
}

export function CommandPalette() {
  const threads = useBlueprint((state) => state.threads);
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const entries = useMemo(
    () => paletteEntries(threads).filter((entry) => entry.label.toLowerCase().includes(query.trim().toLowerCase())),
    [threads, query],
  );
  const close = () => useBlueprint.setState({ paletteOpen: false });
  const run = (entry: PaletteEntry | undefined) => {
    if (!entry) return;
    close();
    entry.run();
  };
  return (
    <div className="overlay" onClick={close}>
      <div className="palette" role="dialog" aria-label="검색과 명령" onClick={(event) => event.stopPropagation()}>
        <div className="palette-input">
          <Icon name="search" />
          <input
            autoFocus
            value={query}
            placeholder="스레드 이름이나 명령을 입력하십시오"
            onChange={(event) => {
              setQuery(event.target.value);
              setCursor(0);
            }}
            onKeyDown={(event) => {
              if (event.key === "ArrowDown") {
                event.preventDefault();
                setCursor(Math.min(cursor + 1, entries.length - 1));
              } else if (event.key === "ArrowUp") {
                event.preventDefault();
                setCursor(Math.max(cursor - 1, 0));
              } else if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                run(entries[cursor]);
              }
            }}
          />
          <kbd>esc</kbd>
        </div>
        <ul>
          {entries.map((entry, index) => (
            <li key={entry.id}>
              <button type="button" className={index === cursor ? "selected" : undefined} onMouseEnter={() => setCursor(index)} onClick={() => run(entry)}>
                <Icon name={entry.icon} />
                <span>{entry.label}</span>
                <span className="palette-hint">{entry.hint}</span>
              </button>
            </li>
          ))}
          {entries.length === 0 && <li className="empty-row">결과가 없습니다</li>}
        </ul>
      </div>
    </div>
  );
}

const SHORTCUTS = [
  ["⌘K", "검색과 명령"],
  ["⌘N", "새 스레드"],
  ["⌘B", "사이드바 열기 / 닫기"],
  ["⌘\\", "오른쪽 패널 열기 / 닫기"],
  ["⌘J", "터미널 열기 / 닫기"],
  ["⏎", "보내기 (진행 중이면 대기 목록)"],
  ["⇧⏎", "줄바꿈"],
  ["⌘⏎", "승인 요청 허용"],
  ["⌘⌫", "승인 요청 거부"],
] as const;

export function ShortcutsDialog() {
  const close = () => useBlueprint.setState({ shortcutsOpen: false });
  return (
    <div className="overlay" onClick={close}>
      <div className="dialog" role="dialog" aria-label="단축키" onClick={(event) => event.stopPropagation()}>
        <header>
          <strong>단축키</strong>
          <button type="button" className="icon-button" title="닫기" onClick={close}>
            <Icon name="close" />
          </button>
        </header>
        <dl>
          {SHORTCUTS.map(([keys, label]) => (
            <div key={keys}>
              <dt>
                <kbd>{keys}</kbd>
              </dt>
              <dd>{label}</dd>
            </div>
          ))}
        </dl>
      </div>
    </div>
  );
}

export function TerminalDock({ branch }: { branch: string }) {
  return (
    <section className="terminal-dock">
      <header>
        <button type="button" className="terminal-tab selected">
          <Icon name="terminal" size={13} />
          zsh · {branch}
        </button>
        <button type="button" className="icon-button" title="새 터미널" onClick={() => notConnected("새 터미널")}>
          <Icon name="plus" size={14} />
        </button>
        <span className="bar-spacer" />
        <button type="button" className="icon-button" title="터미널 닫기" onClick={() => useBlueprint.setState({ terminalOpen: false })}>
          <Icon name="close" size={14} />
        </button>
      </header>
      <pre>
        {TERMINAL_LINES.join("\n")}
        <span className="cursor" />
      </pre>
    </section>
  );
}

export function Toasts() {
  const toasts = useBlueprint((state) => state.toasts);
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((item) => (
        <div key={item.id} className="toast">
          {item.text}
        </div>
      ))}
    </div>
  );
}

/** 전역 단축키이다. 입력창에 글을 쓰는 중에도 동작한다. */
export function useShortcuts(): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        useBlueprint.setState({ paletteOpen: false, shortcutsOpen: false });
        return;
      }
      if (!(event.metaKey || event.ctrlKey) || event.shiftKey || event.altKey) return;
      const actions: Record<string, () => void> = {
        k: () => useBlueprint.setState((state) => ({ paletteOpen: !state.paletteOpen })),
        n: () => showView("home"),
        b: () => useBlueprint.setState((state) => ({ sidebarOpen: !state.sidebarOpen })),
        j: () => useBlueprint.setState((state) => ({ terminalOpen: !state.terminalOpen })),
        "\\": () => useBlueprint.setState((state) => ({ panel: state.panel ? null : "review" })),
      };
      const action = actions[event.key.toLowerCase()];
      if (action) {
        event.preventDefault();
        action();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
