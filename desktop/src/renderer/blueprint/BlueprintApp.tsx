// Akra Desktop UX 블루프린트의 전체 틀이다: 사이드바, 위 막대, 대화, 입력창, 터미널, 오른쪽 패널.
// 모든 값은 mock 이다. 목표 화면을 먼저 정하고, 실제 앱의 구성요소를 이 모양으로 하나씩 옮긴다.
import { useEffect } from "react";

import { Composer } from "./Composer";
import { Conversation } from "./Conversation";
import { Icon } from "./icons";
import { PROJECTS } from "./mockData";
import { PANELS, RightPanel } from "./RightPanel";
import { Sidebar } from "./Sidebar";
import { togglePanel, useBlueprint } from "./store";
import { CommandPalette, Fleet, Home, ShortcutsDialog, TerminalDock, Toasts, useShortcuts } from "./Views";

function TopBar() {
  const sidebarOpen = useBlueprint((state) => state.sidebarOpen);
  const panel = useBlueprint((state) => state.panel);
  const terminalOpen = useBlueprint((state) => state.terminalOpen);
  const thread = useBlueprint((state) => (state.view === "thread" ? state.threads.find((item) => item.id === state.openThreadId) : undefined));
  const project = PROJECTS.find((item) => item.id === thread?.projectId);
  return (
    <header className={sidebarOpen ? "top-bar" : "top-bar no-sidebar"}>
      <button type="button" className="icon-button" title="사이드바 (⌘B)" onClick={() => useBlueprint.setState({ sidebarOpen: !sidebarOpen })}>
        <Icon name="sidebar" />
      </button>
      {thread && project && (
        <div className="top-title">
          <span className="muted">{project.name}</span>
          <Icon name="chevronRight" size={12} />
          <strong>{thread.title}</strong>
          <span className="chip" title={thread.worktree ? "worktree 에서 실행합니다" : "작업 폴더에서 실행합니다"}>
            <Icon name="branch" size={12} />
            {thread.branch}
          </span>
          {thread.pullRequest && (
            <span className={`badge badge-${thread.pullRequest.checks === "fail" ? "bad" : "ok"}`}>PR #{thread.pullRequest.number}</span>
          )}
        </div>
      )}
      <span className="bar-spacer" />
      <span className="blueprint-tag" title="이 화면의 값은 모두 mock 입니다">
        BLUEPRINT
      </span>
      {thread && thread.added + thread.removed > 0 && (
        <button type="button" className={panel === "review" ? "chip-button selected" : "chip-button"} onClick={() => togglePanel("review")}>
          <Icon name="diff" size={13} />
          <span className="counts">
            <span className="count-add">+{thread.added}</span> <span className="count-del">−{thread.removed}</span>
          </span>
        </button>
      )}
      {PANELS.filter((item) => item.id !== "review").map((item) => (
        <button key={item.id} type="button" className={panel === item.id ? "icon-button selected" : "icon-button"} title={item.label} onClick={() => togglePanel(item.id)}>
          <Icon name={item.icon} />
        </button>
      ))}
      <button type="button" className={terminalOpen ? "icon-button selected" : "icon-button"} title="터미널 (⌘J)" onClick={() => useBlueprint.setState({ terminalOpen: !terminalOpen })}>
        <Icon name="terminal" />
      </button>
    </header>
  );
}

export function BlueprintApp() {
  const theme = useBlueprint((state) => state.theme);
  const view = useBlueprint((state) => state.view);
  const sidebarOpen = useBlueprint((state) => state.sidebarOpen);
  const panel = useBlueprint((state) => state.panel);
  const terminalOpen = useBlueprint((state) => state.terminalOpen);
  const paletteOpen = useBlueprint((state) => state.paletteOpen);
  const shortcutsOpen = useBlueprint((state) => state.shortcutsOpen);
  const thread = useBlueprint((state) => state.threads.find((item) => item.id === state.openThreadId));
  useShortcuts();

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  return (
    <div className="shell">
      {sidebarOpen && <Sidebar />}
      <div className="workspace">
        <TopBar />
        <div className="workspace-body">
          <main className="main">
            {view === "home" && <Home />}
            {view === "fleet" && <Fleet />}
            {view === "thread" && thread && (
              <>
                <Conversation threadId={thread.id} />
                <Composer threadId={thread.id} />
              </>
            )}
            {terminalOpen && <TerminalDock branch={thread?.branch ?? "main"} />}
          </main>
          {panel && <RightPanel panel={panel} />}
        </div>
      </div>
      {paletteOpen && <CommandPalette />}
      {shortcutsOpen && <ShortcutsDialog />}
      <Toasts />
    </div>
  );
}
