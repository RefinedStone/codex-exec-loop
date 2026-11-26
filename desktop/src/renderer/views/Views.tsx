// 대화 밖의 화면이다 (R10): 새 스레드(홈), 현황판, 검색과 명령, 단축키 목록, 알림.
import { useEffect, useMemo, useState } from "react";

import { command, type CommandId, COMMANDS, commandForKey, PALETTE_ORDER } from "../../shared/commands";

import { Composer, type ComposerActions } from "../conversation/Composer";
import { Icon, type IconName } from "../design/icons";
import { Picker } from "../design/Picker";
import { useDesktopStore } from "../session/desktopStore";
import { MOD_KEY, openPanel, showHome, showView, togglePanel, toggleDotsCloud, toggleTerminal, toggleTheme, useUiStore } from "../session/uiStore";
import { StatusMark, useThreadMarks } from "../sidebar/Sidebar";
import { allThreads, folderName, type ListedThread, relativeAge, threadMark, type ThreadMark } from "../sidebar/threadStatus";

const CHOOSE_FOLDER = "폴더 고르기…";
const RECENT_COUNT = 5;

export function Home({ actions, onChooseFolder, onOpenThread }: { actions: ComposerActions; onChooseFolder(): Promise<string | null>; onOpenThread(threadId: string): void }) {
  const homeCwd = useUiStore((state) => state.homeCwd);
  const folders = useDesktopStore((state) => state.folders);
  const marks = useThreadMarks();
  const recent = useMemo(() => allThreads(folders).slice(0, RECENT_COUNT), [folders]);
  // 같은 이름의 폴더가 둘이면 구분할 수 있게 전체 경로를 보여 준다.
  const options = [...new Set([...(homeCwd ? [homeCwd] : []), ...folders.map((folder) => folder.cwd)]), CHOOSE_FOLDER];
  const choose = (value: string) => {
    if (value !== CHOOSE_FOLDER) {
      useUiStore.setState({ homeCwd: value });
      return;
    }
    void onChooseFolder().then((cwd) => cwd && useUiStore.setState({ homeCwd: cwd }));
  };
  return (
    <div className="home" data-testid="home">
      <div className="home-inner">
        <h1>무엇을 할까요?</h1>
        <div className="home-pickers">
          <Picker icon={<Icon name="folder" size={12} />} value={homeCwd ?? CHOOSE_FOLDER} options={options} onChange={choose} />
        </div>
        {/* 폴더를 바꾸면 입력창을 새로 만든다. 이전 폴더의 `@` 후보와 진행 중인 찾기를 버린다 (#2837). 초안은 저장소에 남는다. */}
        <Composer key={homeCwd ?? ""} actions={actions} home />
        {recent.length > 0 && <h3>최근 스레드</h3>}
        <ul className="recent">
          {recent.map((thread) => (
            <li key={thread.id}>
              <button type="button" onClick={() => onOpenThread(thread.id)}>
                <StatusMark mark={threadMark(thread, marks)} />
                <span className="recent-title">{thread.title}</span>
                <span className="muted">{folderName(thread.cwd)}</span>
                <span className="muted">{relativeAge(thread.updatedAt, Date.now())}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

const FLEET_COLUMNS: { title: string; marks: ThreadMark[]; hint: string }[] = [
  { title: "확인 필요", marks: ["approval", "failed"], hint: "승인 또는 실패 확인을 기다립니다" },
  { title: "진행 중", marks: ["running"], hint: "에이전트가 작업합니다" },
  { title: "읽지 않음", marks: ["unread"], hint: "끝났고 아직 보지 않았습니다" },
  { title: "최근", marks: ["idle"], hint: "본 스레드입니다" },
];
/** shortcut: 한 열에 최근 30개까지만 그린다. 스레드가 수천 개일 때 화면이 느려지지 않게 한다. 더 필요하면 가상 스크롤로 바꾼다. */
const FLEET_COLUMN_LIMIT = 30;

function FleetCard({ thread, mark, onOpenThread }: { thread: ListedThread; mark: ThreadMark; onOpenThread(threadId: string): void }) {
  return (
    <button type="button" className="fleet-card" onClick={() => onOpenThread(thread.id)} data-testid="fleet-card">
      <span className="fleet-card-title">
        <StatusMark mark={mark} />
        {thread.title}
      </span>
      <span className="fleet-card-meta">
        <Icon name="folder" size={12} />
        {folderName(thread.cwd)}
        {thread.branch && (
          <>
            <Icon name="branch" size={12} />
            {thread.branch}
          </>
        )}
        <span className="bar-spacer" />
        {relativeAge(thread.updatedAt, Date.now())}
      </span>
    </button>
  );
}

export function Fleet({ onOpenThread }: { onOpenThread(threadId: string): void }) {
  const folders = useDesktopStore((state) => state.folders);
  const marks = useThreadMarks();
  const threads = useMemo(() => allThreads(folders).map((thread) => ({ thread, mark: threadMark(thread, marks) })), [folders, marks]);
  return (
    <div className="fleet" data-testid="fleet">
      <header className="fleet-header">
        <h1>현황판</h1>
        <span className="muted">모든 작업 폴더의 스레드를 상태별로 봅니다.</span>
      </header>
      <div className="fleet-columns">
        {FLEET_COLUMNS.map((column) => {
          const items = threads.filter((item) => column.marks.includes(item.mark));
          return (
            <section key={column.title} className="fleet-column">
              <h3 title={column.hint}>
                {column.title} <span className="muted">{items.length}</span>
              </h3>
              {items.slice(0, FLEET_COLUMN_LIMIT).map((item) => (
                <FleetCard key={item.thread.id} thread={item.thread} mark={item.mark} onOpenThread={onOpenThread} />
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

const PALETTE_LIMIT = 50;

/**
 * 명령 표의 id 마다 renderer 가 하는 일이다 (#2840). 표에 없는 동작은 여기에 없다. 메뉴가 맡는 명령(확대, 새 창)은 main 이
 * 처리한다. `onNavigate` 는 단축키에서만 준다.
 */
export function commandHandlers(onNavigate?: (delta: -1 | 1) => void): Partial<Record<CommandId, () => void>> {
  return {
    palette: () => useUiStore.setState((state) => ({ overlay: state.overlay === "palette" ? null : "palette" })),
    "new-thread": () => showHome(),
    sidebar: () => useUiStore.setState((state) => ({ sidebarOpen: !state.sidebarOpen })),
    "right-panel": () => (useUiStore.getState().panel ? useUiStore.setState({ panel: null }) : openPanel("review")),
    terminal: toggleTerminal,
    // 대화가 화면에 있을 때만 찾기 막대를 연다. 이미 열려 있으면 입력란으로 포커스를 옮긴다.
    find: () => {
      const view = useUiStore.getState().view;
      if (view !== "thread" && view !== "remote") return;
      useUiStore.setState({ findOpen: true, overlay: null });
      document.querySelector<HTMLInputElement>("[data-find-input]")?.select();
    },
    back: () => onNavigate?.(-1),
    forward: () => onNavigate?.(1),
    fleet: () => showView("fleet"),
    review: () => togglePanel("review"),
    graph: () => togglePanel("graph"),
    kernel: () => togglePanel("kernel"),
    browser: () => togglePanel("browser"),
    work: () => togglePanel("work"),
    docs: () => togglePanel("docs"),
    skills: () => togglePanel("skills"),
    theme: toggleTheme,
    settings: () => useUiStore.setState({ overlay: "settings" }),
  };
}

function paletteCommands(): PaletteEntry[] {
  const handlers = commandHandlers();
  return PALETTE_ORDER.map((id) => {
    const spec = command(id);
    return {
      id,
      icon: (spec.paletteIcon ?? "bolt") as IconName,
      label: spec.paletteTitle ?? spec.title,
      hint: spec.keyLabel ? `${MOD_KEY}${spec.keyLabel}` : "",
      run: () => handlers[id]?.(),
    };
  });
}

export function CommandPalette({ onOpenThread }: { onOpenThread(threadId: string): void }) {
  const folders = useDesktopStore((state) => state.folders);
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const entries = useMemo(() => {
    const dots: PaletteEntry[] = [{ id: "dots-cloud", icon: "sparkle", label: "Dots 켜기 / 끄기", hint: "", run: () => void toggleDotsCloud() }];
    const threads = allThreads(folders).map<PaletteEntry>((thread) => ({ id: thread.id, icon: "message", label: thread.title, hint: folderName(thread.cwd), run: () => onOpenThread(thread.id) }));
    const needle = query.trim().toLowerCase();
    return [...paletteCommands(), ...dots, ...threads].filter((entry) => entry.label.toLowerCase().includes(needle)).slice(0, PALETTE_LIMIT);
  }, [folders, query, onOpenThread]);
  const close = () => useUiStore.setState({ overlay: null });
  const run = (entry: PaletteEntry | undefined) => {
    if (!entry) return;
    close();
    entry.run();
  };
  return (
    <div className="overlay" onClick={close}>
      <div className="palette" role="dialog" aria-label="검색과 명령" onClick={(event) => event.stopPropagation()} data-testid="palette">
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

/** 명령 표에 없는 입력 규칙이다. 단축키 창에만 보인다. */
const INPUT_KEYS = [
  ["↑ · ↓", "빈 입력창에서 이전 프롬프트 불러오기"],
  ["⏎", "보내기 (진행 중이면 추가 지시)"],
  ["⇧⏎", "줄바꿈"],
  [`${MOD_KEY}⏎`, "승인 요청 허용"],
  [`${MOD_KEY}⌫`, "승인 요청 거부"],
] as const;

const SHORTCUTS: ReadonlyArray<readonly [string, string]> = [
  ...COMMANDS.filter((spec) => (spec.keys || spec.accelerator) && spec.keyLabel).map((spec) => [`${MOD_KEY}${spec.keyLabel}`, spec.title] as const),
  ...INPUT_KEYS,
];

export function ShortcutsDialog() {
  const close = () => useUiStore.setState({ overlay: null });
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

/** 확대 값을 바꾼 직후에 배율을 잠깐 보인다 (plan 26 Z2). */
export function ZoomNotice() {
  const text = useUiStore((state) => state.zoomNotice);
  if (text === null) return null;
  return (
    <div className="zoom-notice" role="status" data-testid="zoom-notice">
      {text}
    </div>
  );
}

export function Toasts() {
  const toasts = useUiStore((state) => state.toasts);
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

/** 전역 단축키이다. 입력창에 글을 쓰는 중에도 동작한다. `onNavigate` 는 이동 기록에서 뒤(-1)나 앞(1)으로 간다. */
export function useShortcuts(onNavigate: (delta: -1 | 1) => void): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        useUiStore.setState({ overlay: null, findOpen: false });
        return;
      }
      if (!(event.metaKey || event.ctrlKey) || event.shiftKey || event.altKey) return;
      // 터미널 안의 Ctrl 조합은 셸의 것이다 (Ctrl+K, Ctrl+N 등). Ctrl+` 만 받는다.
      const inTerminal = event.target instanceof Element && event.target.closest(".terminal-body") !== null;
      if (inTerminal && !event.metaKey && event.code !== "Backquote") return;
      // 글자가 아니라 글쇠 자리로 찾는다. 한글 입력 상태에서는 `event.key` 가 한글 자모이다.
      const id = commandForKey(event.code);
      const actions = commandHandlers(onNavigate);
      const action = id ? actions[id] : undefined;
      if (action) {
        event.preventDefault();
        action();
      }
    };
    // 마우스의 뒤로(3)와 앞으로(4) 단추이다.
    const onMouse = (event: MouseEvent) => {
      if (event.button !== 3 && event.button !== 4) return;
      event.preventDefault();
      onNavigate(event.button === 3 ? -1 : 1);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mouseup", onMouse);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mouseup", onMouse);
    };
  }, [onNavigate]);
}
