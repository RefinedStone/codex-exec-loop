// 화면 아래의 터미널 영역이다 (R2). 탭으로 여러 터미널을 연다. 새 탭은 열린 스레드의 cwd 에서, 없으면 홈 폴더에서 연다.
// 스레드를 바꿔도 터미널은 그대로 둔다. 위 막대의 버튼, Ctrl+`, Ctrl/⌘+J 로 보이거나 숨긴다 (`useShortcuts`).
import { useEffect, useRef, useState } from "react";

import type { TerminalBridge } from "../../shared/bridge";
import { Icon } from "../design/icons";
import { useDesktopStore } from "../session/desktopStore";
import { useUiStore } from "../session/uiStore";
import { folderName } from "../sidebar/threadStatus";
import { TerminalView } from "./TerminalView";

interface TerminalTab {
  key: number;
  cwd: string | null;
  title: string;
  exited: boolean;
}

export function TerminalDock({ bridge }: { bridge: TerminalBridge }) {
  const threadCwd = useDesktopStore((state) => state.transcript?.cwd ?? null);
  const open = useUiStore((state) => state.terminalOpen);
  const [tabs, setTabs] = useState<TerminalTab[]>([]);
  const [activeKey, setActiveKey] = useState<number | null>(null);
  const nextKey = useRef(1);

  const addTab = () => {
    const key = nextKey.current++;
    setTabs((current) => [...current, { key, cwd: threadCwd, title: threadCwd ? folderName(threadCwd) : "홈", exited: false }]);
    setActiveKey(key);
  };

  const closeTab = (key: number) => {
    const remaining = tabs.filter((tab) => tab.key !== key);
    setTabs(remaining);
    if (activeKey === key) setActiveKey(remaining.at(-1)?.key ?? null);
    if (remaining.length === 0) useUiStore.setState({ terminalOpen: false });
  };

  const updateTab = (key: number, change: Partial<TerminalTab>) =>
    setTabs((current) => current.map((tab) => (tab.key === key ? { ...tab, ...change } : tab)));

  // 터미널을 열었는데 탭이 없으면 하나를 만든다.
  useEffect(() => {
    if (open && tabs.length === 0) addTab();
  }, [open]); // 열림 상태가 바뀔 때만 본다.

  return (
    <section className="terminal-dock" hidden={!open} data-testid="terminal-dock">
      <header>
        {tabs.map((tab, index) => (
          <span key={tab.key} className={tab.key === activeKey ? "terminal-tab selected" : "terminal-tab"}>
            <button type="button" onClick={() => setActiveKey(tab.key)} title={tab.cwd ?? "홈 폴더"} data-testid="terminal-tab">
              {index + 1}: {tab.title}
              {tab.exited ? " (끝남)" : ""}
            </button>
            <button type="button" className="terminal-close" onClick={() => closeTab(tab.key)} aria-label="터미널 닫기" data-testid="terminal-close">
              <Icon name="close" size={11} />
            </button>
          </span>
        ))}
        <button type="button" className="icon-button" onClick={addTab} title="새 터미널" data-testid="terminal-add">
          <Icon name="plus" size={14} />
        </button>
        <span className="bar-spacer" />
        <button type="button" className="icon-button" title="터미널 숨기기" onClick={() => useUiStore.setState({ terminalOpen: false })}>
          <Icon name="close" size={14} />
        </button>
      </header>
      <div className="terminal-body">
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
