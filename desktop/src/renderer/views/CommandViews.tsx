import { useEffect, useMemo, useState } from "react";
import { command, type CommandId, COMMANDS, commandForKey, PALETTE_ORDER } from "../../shared/commands";
import { Icon, type IconName } from "../design/icons";
import { openCommitActions } from "../git/openCommitActions";
import { nextAttentionThread } from "../kernel/attentionNavigation";
import { useDesktopStore } from "../session/desktopStore";
import { MOD_KEY, openPanel, showHome, showView, togglePanel, toggleDotsCloud, toggleTerminal, toggleTheme, useUiStore, toast } from "../session/uiStore";
import { allThreads, folderName } from "../sidebar/threadStatus";

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
export function commandHandlers(onNavigate?: (delta: -1 | 1) => void, onOpenThread?: (id: string) => void): Partial<Record<CommandId, () => void>> {
  return {
    "next-attention": () => {
      const { kernel, openThreadId } = useDesktopStore.getState();
      if (kernel.phase !== "attached" || kernel.attentionItems === null) { toast("현재 저장소의 응답 필요 항목을 읽지 못했습니다."); return; }
      const target = nextAttentionThread(kernel.attentionItems, openThreadId);
      if (!target) { toast("응답이 필요한 스레드가 없습니다."); return; }
      if (onOpenThread) onOpenThread(target);
      else toast("현황판에서 스레드를 열어 주세요.");
    },
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
    "graph-actions": openCommitActions,
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

function paletteCommands(onOpenThread: (id: string) => void): PaletteEntry[] {
  const handlers = commandHandlers(undefined, onOpenThread);
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
    return [...paletteCommands(onOpenThread), ...dots, ...threads].filter((entry) => entry.label.toLowerCase().includes(needle)).slice(0, PALETTE_LIMIT);
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

/** 전역 단축키이다. 입력창에 글을 쓰는 중에도 동작한다. `onNavigate` 는 이동 기록에서 뒤(-1)나 앞(1)으로 간다. */
export function useShortcuts(onNavigate: (delta: -1 | 1) => void, onOpenThread?: (id: string) => void): void {
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
      const actions = commandHandlers(onNavigate, onOpenThread);
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
  }, [onNavigate, onOpenThread]);
}
