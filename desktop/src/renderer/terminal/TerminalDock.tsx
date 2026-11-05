// 화면 아래의 터미널 영역이다 (R2). 탭으로 여러 터미널을 연다. 새 탭은 열린 스레드의 cwd 에서, 없으면 홈 폴더에서 연다.
// 스레드를 바꿔도 터미널은 그대로 둔다. Ctrl+` 로 보이거나 숨긴다.
import { useEffect, useRef, useState } from "react";

import type { TerminalBridge } from "../../shared/bridge";
import { useDesktopStore } from "../session/desktopStore";
import { TerminalView } from "./TerminalView";

interface TerminalTab {
  key: number;
  cwd: string | null;
  title: string;
  exited: boolean;
}

function folderName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

export function TerminalDock({ bridge }: { bridge: TerminalBridge }) {
  const threadCwd = useDesktopStore((state) => state.transcript?.cwd ?? null);
  const [open, setOpen] = useState(false);
  const [tabs, setTabs] = useState<TerminalTab[]>([]);
  const [activeKey, setActiveKey] = useState<number | null>(null);
  const nextKey = useRef(1);

  const addTab = () => {
    const key = nextKey.current++;
    setTabs((current) => [...current, { key, cwd: threadCwd, title: threadCwd ? folderName(threadCwd) : "홈", exited: false }]);
    setActiveKey(key);
    setOpen(true);
  };

  const closeTab = (key: number) => {
    const remaining = tabs.filter((tab) => tab.key !== key);
    setTabs(remaining);
    if (activeKey === key) setActiveKey(remaining.at(-1)?.key ?? null);
    if (remaining.length === 0) setOpen(false);
  };

  const updateTab = (key: number, change: Partial<TerminalTab>) =>
    setTabs((current) => current.map((tab) => (tab.key === key ? { ...tab, ...change } : tab)));

  const toggle = () => {
    if (open) {
      setOpen(false);
    } else if (tabs.length === 0) {
      addTab();
    } else {
      setOpen(true);
    }
  };
  const toggleRef = useRef(toggle);
  toggleRef.current = toggle;

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.ctrlKey && event.key === "`") {
        event.preventDefault();
        toggleRef.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <section className={open ? "terminal-dock open" : "terminal-dock"} data-testid="terminal-dock">
      <div className="terminal-tabs">
        <button type="button" className="terminal-toggle" onClick={toggle} data-testid="terminal-toggle" aria-expanded={open}>
          {open ? "▾" : "▸"} 터미널
        </button>
        {open &&
          tabs.map((tab, index) => (
            <span key={tab.key} className={tab.key === activeKey ? "terminal-tab active" : "terminal-tab"}>
              <button type="button" onClick={() => setActiveKey(tab.key)} title={tab.cwd ?? "홈 폴더"} data-testid="terminal-tab">
                {index + 1}: {tab.title}
                {tab.exited ? " (끝남)" : ""}
              </button>
              <button type="button" className="terminal-close" onClick={() => closeTab(tab.key)} aria-label="터미널 닫기" data-testid="terminal-close">
                ×
              </button>
            </span>
          ))}
        {open && (
          <button type="button" className="terminal-add" onClick={addTab} title="새 터미널" data-testid="terminal-add">
            +
          </button>
        )}
      </div>
      <div className="terminal-body" hidden={!open}>
        {tabs.map((tab) => (
          <TerminalView
            key={tab.key}
            bridge={bridge}
            cwd={tab.cwd}
            visible={open && tab.key === activeKey}
            onOpened={({ cwd }) => updateTab(tab.key, { title: folderName(cwd) })}
            onExited={() => updateTab(tab.key, { exited: true })}
          />
        ))}
      </div>
    </section>
  );
}
