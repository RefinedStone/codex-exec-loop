// 대화 밖의 화면이다: 새 스레드(홈), 현황판, 명령 팔레트, 단축키 목록, 터미널, 알림.
import { useEffect, useMemo, useState } from "react";

import { Picker } from "../design/Picker";
import { Composer } from "./Composer";
import { Icon, type IconName } from "../design/icons";
import { ATTENTION_GROUPS, type Attention } from "./flow";
import { BASE_BRANCHES, PROJECTS, SUGGESTIONS, TERMINAL_LINES, type MockThread } from "./mockData";
import { StatusMark } from "./Sidebar";
import { answerApproval, goNextInTurn, handOffToNewThread, handOffToThread, notConnected, openThread, runDelivery, showView, togglePanel, toggleTheme, useAttentionList, useBlueprint, useDelivery } from "./store";

function projectName(projectId: string): string {
  return PROJECTS.find((project) => project.id === projectId)?.name ?? projectId;
}

const RUN_PLACES = ["Worktree 에서 실행", "작업 폴더에서 실행"] as const;
const GROUP_ORDER = ATTENTION_GROUPS.map((group) => group.id);
const RESUME_COUNT = 5;

export function Home() {
  const homeProjectId = useBlueprint((state) => state.homeProjectId);
  const homeWorktree = useBlueprint((state) => state.homeWorktree);
  const homeBase = useBlueprint((state) => state.homeBase);
  // 사람을 기다리는 스레드가 먼저이다: 내 차례, 읽을 것, 작업 중, 나머지.
  const resume = useAttentionList()
    .sort((left, right) => GROUP_ORDER.indexOf(left.attention.group) - GROUP_ORDER.indexOf(right.attention.group))
    .slice(0, RESUME_COUNT);
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
          {homeWorktree && <Picker value={`기준: ${homeBase}`} options={BASE_BRANCHES.map((base) => `기준: ${base}`)} onChange={(base) => useBlueprint.setState({ homeBase: base.replace("기준: ", "") })} />}
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
        <h3>이어서 하기</h3>
        <ul className="recent">
          {resume.map(({ thread, attention }) => (
            <li key={thread.id}>
              <button type="button" onClick={() => openThread(thread.id)}>
                <StatusMark status={thread.status} />
                <span className="recent-title">{thread.title}</span>
                {attention.reason && <span className={attention.group === "mine" ? "badge badge-warn" : "muted"}>{attention.reason}</span>}
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

/** "내 차례" 카드는 스레드를 열지 않고 답한다. 엿보기는 답하기 전에 근거를 보인다. */
function FleetCard({ thread, attention }: { thread: MockThread; attention: Attention }) {
  const [peek, setPeek] = useState(false);
  const approval = useBlueprint((state) => state.approvals[thread.id]);
  const lastAnswer = useBlueprint((state) => state.turns[thread.id]?.at(-1)?.items.findLast((item) => item.type === "agent"));
  const delivery = useDelivery(thread);
  const inTurn = attention.group === "mine";
  return (
    <div className="fleet-card">
      <button type="button" className="fleet-card-title" onClick={() => openThread(thread.id)}>
        <StatusMark status={thread.status} />
        {thread.title}
      </button>
      {attention.reason && <span className={inTurn ? "fleet-card-reason mine" : "fleet-card-reason"}>{attention.reason}</span>}
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
      {inTurn && (
        <span className="offer-buttons">
          {thread.status === "approval" ? (
            <>
              <button type="button" className="button primary" onClick={() => answerApproval(thread.id, true)}>
                허용
              </button>
              <button type="button" className="button" onClick={() => answerApproval(thread.id, false)}>
                거부
              </button>
            </>
          ) : (
            <button type="button" className="button primary" onClick={() => (delivery?.next?.id === "checks" ? runDelivery(thread.id, "checks") : handOffToThread(thread.id, "실패한 턴의 원인을 찾아줘."))}>
              실패 기록 넘기기
            </button>
          )}
          <button type="button" className="ghost" onClick={() => setPeek(!peek)} aria-expanded={peek}>
            엿보기
          </button>
        </span>
      )}
      {peek && <p className="fleet-card-peek">{approval ? `${approval.cwd}\n${approval.reason}` : lastAnswer?.type === "agent" ? lastAnswer.text : "아직 답이 없습니다"}</p>}
    </div>
  );
}

export function Fleet() {
  const rows = useAttentionList();
  return (
    <div className="fleet">
      <header className="fleet-header">
        <h1>현황판</h1>
        <span className="muted">모든 프로젝트의 스레드를 주의 묶음으로 봅니다. 내 차례는 여기에서 바로 답합니다.</span>
      </header>
      <div className="fleet-columns">
        {ATTENTION_GROUPS.map((column) => {
          const items = rows.filter(({ attention }) => attention.group === column.id);
          return (
            <section key={column.id} className="fleet-column">
              <h3 title={column.hint}>
                {column.title} <span className="muted">{items.length}</span>
              </h3>
              {items.map(({ thread, attention }) => (
                <FleetCard key={thread.id} thread={thread} attention={attention} />
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
    { id: "next", icon: "alert", label: "다음 내 차례로 이동", hint: "⌘⇧A", run: goNextInTurn },
    { id: "group", icon: "stack", label: "사이드바 묶기 바꾸기 (프로젝트 / 상태)", hint: "", run: () => useBlueprint.setState((state) => ({ sidebarGroup: state.sidebarGroup === "project" ? "attention" : "project" })) },
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
  ["⌘⇧A", "다음 내 차례로 이동"],
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
      if (!(event.metaKey || event.ctrlKey) || event.altKey) return;
      if (event.shiftKey) {
        // 글쇠 자리로 찾는다. 한글 입력 상태에서도 동작한다.
        if (event.code === "KeyA") {
          event.preventDefault();
          goNextInTurn();
        }
        return;
      }
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
