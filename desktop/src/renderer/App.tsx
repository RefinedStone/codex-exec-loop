// 화면 틀이다 (R10): 사이드바, 위 막대, 본문(새 스레드 · 현황판 · 대화와 입력창), 터미널, 오른쪽 패널.
// 모양은 UX 블루프린트(`blueprint/`)를 따른다. 대화 상태는 `DesktopSession` 과 `desktopStore` 가 가진다.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { BrowserPanel } from "./browser/BrowserPanel";
import { ApprovalPrompt } from "./conversation/ApprovalPrompt";
import { pullRequestBadgeClass } from "./conversation/BranchCommits";
import { Composer, type ComposerActions } from "./conversation/Composer";
import { ConversationView } from "./conversation/ConversationView";
import { isTurnInProgress } from "./conversation/transcript";
import { useWorkingTree, type WorkingTree, workingTreeCounts, WorkingTreePanel } from "./conversation/WorkingTreePanel";
import { Icon, type IconName } from "./design/icons";
import { useResizableWidth } from "./design/useResizableWidth";
import { KernelPanel } from "./kernel/KernelPanel";
import { DesktopSession, remoteHostBridge } from "./session/desktopSession";
import { ConversationStoreContext, desktopSessionStore, openThreadCwd, remoteHostKey, remoteHostStore, useDesktopStore } from "./session/desktopStore";
import { ATTENTION_TEXT, attentionEvents } from "./session/attention";
import { type NavEntry, stepNav } from "./session/navHistory";
import { closeDot, dotDraftKey, fitSideColumns, handOffToNewThread, MOD_KEY, type PanelId, setDraft, showDot, showNavEntry, showRemoteAgentsWhenDotsAreOn, showView, toast, togglePanel, toggleTerminal, useUiStore, visit } from "./session/uiStore";
import { Sidebar } from "./sidebar/Sidebar";
import { SkillsPanel } from "./skills/SkillsPanel";
import { folderName } from "./sidebar/threadStatus";
import { TerminalDock } from "./terminal/TerminalDock";
import { DotDock } from "./views/DotDock";
import { notifyDotReply, notifyOs } from "./views/notify";
import { SettingsDialog } from "./views/SettingsDialog";
import { useAppSettings } from "./views/useAppSettings";
import { useZoom } from "./views/useZoom";
import { CommandPalette, Fleet, Home, ShortcutsDialog, Toasts, useShortcuts, ZoomNotice } from "./views/Views";
import { DocumentsPanel } from "./work/DocumentsPanel";
import { GitHubPanel } from "./work/GitHubPanel";
import { GitHubProjectPanel } from "./work/GitHubProjectPanel";
import { type IssueDraftTarget, issueDraftTarget } from "./work/issueDraft";
import { IssueDraftPanel } from "./work/IssueDraftPanel";
import { findFolderForRepository, readIssueHandoffPrompt } from "./work/githubProjects";
import { JiraPanel } from "./work/JiraPanel";
import { LinearPanel } from "./work/LinearPanel";
import type { GatewayRequest } from "./work/notionHub";
import { NotionPanel } from "./work/NotionPanel";
import { GitGraphPanel } from "./git/GitGraphPanel";
import { changedCount, fetchThreadDocuments, markThreadDocumentRead, refreshThreadDocuments, type ThreadDocument, type ThreadDocumentsState } from "./work/threadDocuments";

export const CLIENT_VERSION = "0.0.0-development";

const PANELS: { id: PanelId; label: string; icon: IconName; testId: string }[] = [
  { id: "review", label: "변경", icon: "diff", testId: "review-toggle" },
  { id: "graph", label: "그래프", icon: "branch", testId: "git-graph-toggle" },
  { id: "kernel", label: "Akra", icon: "bolt", testId: "kernel-toggle" },
  { id: "browser", label: "브라우저", icon: "globe", testId: "browser-toggle" },
  { id: "work", label: "업무", icon: "briefcase", testId: "work-toggle" },
  { id: "docs", label: "문서", icon: "book", testId: "documents-toggle" },
  { id: "skills", label: "스킬", icon: "stack", testId: "skills-toggle" },
];

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** 이동 기록의 항목으로 지금 갈 수 있는지 본다. 지워진 스레드와 꺼진 원격 호스트는 건너뛴다 (plan 26 U-2). */
function navEntryExists(entry: NavEntry): boolean {
  const state = useDesktopStore.getState();
  if (entry.view === "thread") {
    return state.openThreadId === entry.threadId || state.folders.some((folder) => folder.threads.some((thread) => thread.id === entry.threadId));
  }
  if (entry.view === "remote") {
    return useUiStore.getState().remoteAgents && remoteHostKey(state).split(",").includes(entry.host);
  }
  return true;
}

function TopBar({ tree, documentCount, changedDocumentCount, onNavigate }: { tree: WorkingTree; documentCount: number | null; changedDocumentCount: number; onNavigate(delta: -1 | 1): void }) {
  // 스레드 목록이 바뀌면 갈 수 있는 곳도 바뀐다.
  useDesktopStore((state) => state.folders);
  const canGoBack = useUiStore((state) => stepNav(state.nav, -1, navEntryExists) !== null);
  const canGoForward = useUiStore((state) => stepNav(state.nav, 1, navEntryExists) !== null);
  const sidebarOpen = useUiStore((state) => state.sidebarOpen);
  const panel = useUiStore((state) => state.panel);
  const terminalOpen = useUiStore((state) => state.terminalOpen);
  const inThread = useUiStore((state) => state.view === "thread");
  const title = useDesktopStore((state) => state.transcript?.title ?? null);
  const cwd = useDesktopStore(openThreadCwd);
  const counts = workingTreeCounts(tree);
  const pullRequest = tree.branch.phase === "loaded" && tree.branch.summary.ok ? tree.branch.summary.pullRequest : null;
  return (
    <header className={sidebarOpen ? "top-bar" : "top-bar no-sidebar"} data-testid="gateway-info">
      <button type="button" className="icon-button" title={`사이드바 (${MOD_KEY}B)`} onClick={() => useUiStore.setState({ sidebarOpen: !sidebarOpen })}>
        <Icon name="sidebar" />
      </button>
      <button type="button" className="icon-button" title={`뒤로 (${MOD_KEY}[)`} onClick={() => onNavigate(-1)} disabled={!canGoBack} data-testid="nav-back">
        <Icon name="chevronLeft" />
      </button>
      <button type="button" className="icon-button" title={`앞으로 (${MOD_KEY}])`} onClick={() => onNavigate(1)} disabled={!canGoForward} data-testid="nav-forward">
        <Icon name="chevronRight" />
      </button>
      {inThread && title && cwd && (
        <div className="top-title">
          <span className="muted" title={cwd}>
            {folderName(cwd)}
          </span>
          <Icon name="chevronRight" size={12} />
          <strong>{title}</strong>
          {counts?.branch && (
            <span className="chip" title="작업 폴더의 현재 브랜치">
              <Icon name="branch" size={12} />
              {counts.branch}
            </span>
          )}
          {pullRequest && (
            <span className={pullRequestBadgeClass(pullRequest)} title={pullRequest.title} data-testid="top-pr">
              PR #{pullRequest.number}
            </span>
          )}
        </div>
      )}
      <span className="bar-spacer" />
      {PANELS.map((item) => {
        const selected = panel === item.id;
        if (item.id === "review" && inThread && counts && counts.added + counts.removed > 0) {
          return (
            <button key={item.id} type="button" className={selected ? "chip-button selected" : "chip-button"} title={item.label} onClick={() => togglePanel(item.id)} aria-pressed={selected} data-testid={item.testId}>
              <Icon name="diff" size={13} />
              <span className="counts">
                <span className="count-add">+{counts.added}</span> <span className="count-del">−{counts.removed}</span>
              </span>
            </button>
          );
        }
        if (item.id === "docs" && documentCount) {
          return (
            <button key={item.id} type="button" className={selected ? "chip-button selected" : "chip-button"} title={item.label} onClick={() => togglePanel(item.id)} aria-pressed={selected} data-testid={item.testId}>
              문서 {documentCount}
              {changedDocumentCount ? ` · 바뀜 ${changedDocumentCount}` : ""}
            </button>
          );
        }
        return (
          <button key={item.id} type="button" className={selected ? "icon-button selected" : "icon-button"} title={item.label} onClick={() => togglePanel(item.id)} aria-pressed={selected} data-testid={item.testId}>
            <Icon name={item.icon} />
          </button>
        );
      })}
      <button type="button" className={terminalOpen ? "icon-button selected" : "icon-button"} title={`터미널 (${MOD_KEY}J)`} onClick={toggleTerminal} aria-pressed={terminalOpen} data-testid="terminal-toggle">
        <Icon name="terminal" />
      </button>
    </header>
  );
}

/** 버전이 맞지 않거나 세션 요청이 실패했을 때만 위 막대 아래에 보인다. */
function SessionBanner() {
  const versionCheck = useDesktopStore((state) => state.versionCheck);
  const sessionError = useDesktopStore((state) => state.sessionError);
  const mismatch = versionCheck && !versionCheck.matches;
  if (!mismatch && !sessionError) return null;
  return (
    <div className="banner">
      {mismatch && (
        <span className="warning" data-testid="version-warning">
          생성 타입은 codex {versionCheck.generated ?? "?"} 기준인데 Gateway 는 {versionCheck.gateway ?? "?"} 를 실행 중입니다. 타입을 다시 생성하세요.
        </span>
      )}
      {sessionError && <span className="error">{sessionError}</span>}
    </div>
  );
}

/**
 * 다른 Codex 앱이 열린 스레드의 쓰기 잠금을 가지고 있어서 읽기만 했을 때 보인다. "다시 연결"은 다시 구독을
 * 시도하고, 잠금이 아직 있으면 최신 기록을 다시 읽는다.
 */
function WriterElsewhereBanner({ onReconnect }: { onReconnect(): Promise<void> }) {
  const writerElsewhere = useDesktopStore((state) => state.writerElsewhere);
  const [reconnecting, setReconnecting] = useState(false);
  if (!writerElsewhere) return null;
  const reconnect = () => {
    setReconnecting(true);
    void onReconnect().finally(() => setReconnecting(false));
  };
  return (
    <div className="banner" data-testid="writer-elsewhere-banner">
      <span className="warning">다른 Codex 앱이 이 스레드를 쓰고 있습니다. 읽기 전용이고, 새 내용은 다시 연결해야 보입니다.</span>
      <button type="button" className="chip-button" onClick={reconnect} disabled={reconnecting} data-testid="writer-elsewhere-reconnect">
        {reconnecting ? "연결하는 중…" : "다시 연결"}
      </button>
    </div>
  );
}

const MIN_PANEL_WIDTH = 360;
const MAX_PANEL_WIDTH = 900;

function RightPanel({ panel, children }: { panel: PanelId; children: React.ReactNode }) {
  const [width, startResize] = useResizableWidth(520, MIN_PANEL_WIDTH, MAX_PANEL_WIDTH);
  return (
    <aside className="right-panel" style={{ width }}>
      <div className="resize-handle" onPointerDown={startResize} />
      <header className="panel-tabs">
        {PANELS.map((item) => (
          <button key={item.id} type="button" className={item.id === panel ? "selected" : undefined} onClick={() => useUiStore.setState({ panel: item.id })}>
            <Icon name={item.icon} size={14} />
            {item.label}
          </button>
        ))}
        <span className="bar-spacer" />
        <button type="button" className="icon-button" title="패널 닫기" onClick={() => useUiStore.setState({ panel: null })}>
          <Icon name="close" />
        </button>
      </header>
      <div className="panel-body">{children}</div>
    </aside>
  );
}

const WORK_SOURCES = ["GitHub", "Projects", "Jira", "Notion", "Linear"] as const;
type WorkSource = (typeof WORK_SOURCES)[number];

/** 폴더의 GitHub 저장소(`owner/name`)이다. 폴더가 없거나 GitHub 가 아니면 null 이다. */
function useGitHubRepository(cwd: string | null): string | null {
  const [repository, setRepository] = useState<string | null>(null);
  useEffect(() => {
    setRepository(null);
    if (!cwd) return;
    let current = true;
    window.akra.github.repository(cwd).then(
      (found) => current && setRepository(found),
      () => undefined,
    );
    return () => {
      current = false;
    };
  }, [cwd]);
  return repository;
}

/**
 * 이슈 내용을 새 스레드의 입력창에 채운다 (R11). 폴더는 열린 스레드의 폴더부터 사이드바의 작업 폴더까지 `origin` 이 같은
 * 것을 고른다. 없으면 고르지 않고, 보낼 때 폴더 선택 창이 열린다.
 */
async function handOffIssue(request: GatewayRequest, url: string, repository: string, number: number): Promise<void> {
  const state = useDesktopStore.getState();
  const openCwd = openThreadCwd(state);
  const folders = [...(openCwd ? [openCwd] : []), ...state.folders.map((folder) => folder.cwd)];
  const cwd = await findFolderForRepository(repository, folders, (folder) => window.akra.github.repository(folder));
  const text = await readIssueHandoffPrompt(request, url, repository, number, cwd);
  useUiStore.setState({ homeCwd: cwd });
  handOffToNewThread(text);
}

function WorkTab({ request, onHandOff, onLinearHandOff }: {
  request: GatewayRequest;
  onHandOff(text: string): Promise<void>;
  onLinearHandOff(text: string): Promise<void>;
}) {
  const [source, setSource] = useState<WorkSource>("GitHub");
  const currentRepository = useGitHubRepository(useDesktopStore(openThreadCwd));
  // 고치는 중인 이슈이다 (#2426). 그 글은 이 패널에만 있고 대화 입력창으로 가지 않는다.
  const [editing, setEditing] = useState<IssueDraftTarget | null>(null);
  if (editing) return <IssueDraftPanel request={request} target={editing} onClose={() => setEditing(null)} />;
  return (
    <div className="work-tab">
      <div className="segmented">
        {WORK_SOURCES.map((name) => (
          <button key={name} type="button" className={source === name ? "selected" : undefined} onClick={() => setSource(name)} data-testid={`${name.toLowerCase()}-toggle`}>
            {name}
          </button>
        ))}
      </div>
      {source === "GitHub" && <GitHubPanel github={window.akra.github} onHandOff={async (pull) => onHandOff(await window.akra.github.reviewPrompt(pull.url))} />}
      {source === "Projects" && (
        <GitHubProjectPanel
          github={window.akra.github}
          currentRepository={currentRepository}
          onHandOff={async (item) => {
            if (item.url && item.repository && item.number !== null) await handOffIssue(request, item.url, item.repository, item.number);
          }}
          onEdit={async (item) => {
            if (!item.repository || item.number === null) return;
            const state = useDesktopStore.getState();
            const openCwd = openThreadCwd(state);
            const folders = [...(openCwd ? [openCwd] : []), ...state.folders.map((folder) => folder.cwd)];
            const cwd = await findFolderForRepository(item.repository, folders, (folder) => window.akra.github.repository(folder));
            const target = cwd ? issueDraftTarget(cwd, item.repository, item.number) : null;
            if (!target) throw new Error("이 저장소의 작업 폴더가 없습니다. 그 저장소의 폴더를 사이드바에 추가하세요.");
            setEditing(target);
          }}
        />
      )}
      {source === "Jira" && <JiraPanel jira={window.akra.jira} onHandOff={async (issue) => onHandOff(await window.akra.jira.issuePrompt(issue.key))} />}
      {source === "Notion" && <NotionPanel request={request} notion={window.akra.notion} onHandOff={onHandOff} />}
      {source === "Linear" && <LinearPanel linear={window.akra.linear} onHandOff={async (issue) => onLinearHandOff(await window.akra.linear.issuePrompt(issue.id))} />}
    </div>
  );
}

export function App() {
  const session = useRef<DesktopSession | null>(null);
  const theme = useUiStore((state) => state.theme);
  const view = useUiStore((state) => state.view);
  const sidebarOpen = useUiStore((state) => state.sidebarOpen);
  const panel = useUiStore((state) => state.panel);
  const overlay = useUiStore((state) => state.overlay);
  const openCwd = useDesktopStore(openThreadCwd);
  const tree = useWorkingTree(window.akra);
  useZoom();
  useAppSettings();
  // 앱 메뉴의 "설정…" (⌘,) 은 main 이 받아서 이 창에 알린다.
  useEffect(() => window.akra.settings.onOpen(() => useUiStore.setState({ overlay: "settings" })), []);
  // 실행 중인 턴이 있는지 main 에 알린다. main 이 "일할 때만 잠자기 방지"에 쓴다 (plan 26 W5).
  // 접속이 끊기면 턴의 상태를 알 수 없다. 그때는 일하는 중으로 치지 않는다: 잠자기 방지가 켜진 채로 남지 않는다.
  const working = useDesktopStore((state) => state.connection?.phase === "connected" && state.activeThreadIds.length > 0);
  useEffect(() => window.akra.settings.reportWorking(working), [working]);
  // 프로젝트(열린 스레드의 폴더)가 바뀌면 이 창이 연 로컬 복원본을 다시 잠근다 (#2427).
  useEffect(() => () => window.akra.vaultRestore.projectChanged(), [openCwd]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  useEffect(() => {
    const created = new DesktopSession(window.akra, desktopSessionStore(), CLIENT_VERSION);
    const unsubscribe = created.onVersionProviderReady(() => setDocumentsRefresh((value) => value + 1));
    created.start();
    session.current = created;
    return () => {
      unsubscribe();
      created.stop();
    };
  }, []);

  useEffect(() => {
    void showRemoteAgentsWhenDotsAreOn().catch(() => undefined);
  }, []);

  // 원격 호스트(plan 25)마다 읽기 전용 세션을 하나 둔다. 사용자가 켰고 Gateway 가 그 호스트를 알릴 때만 접속한다.
  const remoteSessions = useRef(new Map<string, DesktopSession>());
  const remoteAgents = useUiStore((state) => state.remoteAgents);
  const remoteHost = useUiStore((state) => state.remoteHost);
  const remoteHosts = useDesktopStore(remoteHostKey);
  useEffect(() => {
    if (!remoteAgents || !remoteHosts) return;
    const sessions = remoteSessions.current;
    const hosts = remoteHosts.split(",");
    for (const host of hosts) {
      const created = new DesktopSession(remoteHostBridge(window.akra.remote, host), desktopSessionStore(remoteHostStore(host)), CLIENT_VERSION, undefined, true);
      created.start();
      sessions.set(host, created);
    }
    return () => {
      for (const host of hosts) {
        sessions.get(host)?.stop();
        sessions.delete(host);
        window.akra.remote.close(host);
      }
    };
  }, [remoteAgents, remoteHosts]);
  // 원격 구역을 끄거나 그 호스트가 목록에서 빠지면 원격 대화 화면을 닫는다.
  const remoteVisible = remoteAgents && remoteHost !== null && remoteHosts.split(",").includes(remoteHost);
  useEffect(() => {
    if (view === "remote" && !remoteVisible) showView("home");
  }, [view, remoteVisible]);
  // dot 과의 대화는 보기와 따로 옆 칸에 있다. 원격 구역을 끄거나 그 호스트가 목록에서 빠지면 옆 칸을 닫는다.
  const dotDock = useUiStore((state) => state.dotDock);
  const dotDockVisible = dotDock !== null && remoteAgents && remoteHosts.split(",").includes(dotDock.host);
  useEffect(() => {
    if (dotDock !== null && !dotDockVisible) closeDot();
  }, [dotDock, dotDockVisible]);
  // 창을 줄이면 두 옆 칸이 함께 있을 자리가 없어진다.
  useEffect(() => {
    window.addEventListener("resize", fitSideColumns);
    return () => window.removeEventListener("resize", fitSideColumns);
  }, []);
  // dot 창이 대화를 본 창으로 되돌리면 옆 칸에 연다.
  useEffect(
    () =>
      window.akra.dots.onSeen((host, dotId) => {
        const store = remoteHostStore(host).getState();
        const room = store.dots.find((dot) => dot.id === dotId)?.messagingRoomId;
        if (room) store.markDotRoomRead(room);
      }),
    [],
  );
  // 그 창에서 쓰던 글이 가장 새 글이다: 옆 칸의 초안으로 둔다.
  useEffect(
    () =>
      window.akra.dots.onDock((host, dotId, draft) => {
        setDraft(dotDraftKey(dotId), draft);
        showDot(host, dotId);
      }),
    [],
  );
  // dot 이 답을 끝냈는데 사용자가 그 대화를 보고 있지 않으면 운영체제 알림을 보인다. 누르면 그 대화가 옆 칸에 열린다.
  // 그 dot 의 창이 열려 있으면 그 창이 알린다: 본 창은 알리지 않는다. 안 읽음 표시는 남긴다. 사용자가 그 창에서
  // 답을 보면 창이 알려 주고(`onSeen`), 그때 지운다.
  useEffect(() => {
    if (!remoteAgents || !remoteHosts) return;
    const stops = remoteHosts.split(",").map((host) => {
      const store = remoteHostStore(host);
      let seen = store.getState().dotReplies;
      return store.subscribe((state) => {
        if (state.dotReplies === seen) return;
        const previous = seen;
        seen = state.dotReplies;
        for (const dot of state.dots) {
          const room = dot.messagingRoomId;
          if (room === null || (state.dotReplies[room] ?? 0) === (previous[room] ?? 0)) continue;
          const docked = useUiStore.getState().dotDock;
          if (!document.hidden && docked?.host === host && docked.dotId === dot.id) continue;
          window.akra.dots.hasWindow(host, dot.id).then(
            (inWindow) => inWindow || notifyDotReply(dot.displayName, () => showDot(host, dot.id)),
            () => undefined,
          );
        }
      });
    });
    return () => stops.forEach((stop) => stop());
  }, [remoteAgents, remoteHosts]);
  const dotDockHost = dotDock?.host ?? null;
  const dotGatewayRequest = useMemo<GatewayRequest>(
    () => (method, params) => {
      const current = dotDockHost ? remoteSessions.current.get(dotDockHost) : undefined;
      return current ? current.requestGatewayService(method, params) : Promise.reject(new Error("원격 호스트에 연결되어 있지 않습니다."));
    },
    [dotDockHost],
  );

  const loadRemoteThread = useCallback((host: string, threadId: string) => {
    const state = remoteHostStore(host).getState();
    if (state.openThreadId === threadId && state.transcript?.threadId === threadId) return;
    void remoteSessions.current.get(host)?.openThread(threadId);
  }, []);
  const openRemoteThread = useCallback(
    (host: string, threadId: string) => {
      visit({ view: "remote", host, threadId });
      loadRemoteThread(host, threadId);
    },
    [loadRemoteThread],
  );

  // 커널 패널이 열려 있을 때만 열린 스레드의 저장소 커널에 붙는다. 붙어 있는 동안 그 저장소의 TUI 는 열리지 않는다.
  const kernelWorkspace = panel === "kernel" ? openCwd : null;
  useEffect(() => {
    void session.current?.showKernel(kernelWorkspace);
  }, [kernelWorkspace]);

  const loadThread = useCallback((threadId: string) => {
    // 이미 열려서 기록이 있는 스레드는 다시 읽지 않는다. 다시 읽으면 실시간으로만 오는 값(턴 diff, 계획, 사용량)을 잃는다.
    const state = useDesktopStore.getState();
    if (state.openThreadId === threadId && state.transcript?.threadId === threadId) return;
    void session.current?.openThread(threadId);
  }, []);
  const openThread = useCallback(
    (threadId: string) => {
      visit({ view: "thread", threadId });
      loadThread(threadId);
    },
    [loadThread],
  );

  // 이동 기록에서 뒤나 앞으로 간다 (plan 26 N1). 기록의 자리만 옮기고, 그 항목의 스레드를 연다.
  const navigate = useCallback(
    (delta: -1 | 1) => {
      const target = stepNav(useUiStore.getState().nav, delta, navEntryExists);
      const entry = target && showNavEntry(target);
      if (entry?.view === "thread") loadThread(entry.threadId);
      if (entry?.view === "remote") loadRemoteThread(entry.host, entry.threadId);
    },
    [loadThread, loadRemoteThread],
  );
  useShortcuts(navigate);

  // 턴이 끝나거나 승인 요청이 오면 운영체제 알림을 보인다 (plan 26 A1). 누르면 그 스레드를 연다. 창에 포커스가 있을
  // 때도 알릴지는 알림 설정이 정한다 (`notifyOs`). 알림에는 스레드 이름만 넣는다. 대화의 글은 넣지 않는다.
  useEffect(
    () =>
      useDesktopStore.subscribe((state, previous) => {
        for (const event of attentionEvents(previous, state)) {
          const title = state.folders.flatMap((folder) => folder.threads).find((thread) => thread.id === event.threadId)?.title ?? "Akra";
          notifyOs(title, ATTENTION_TEXT[event.kind], () => openThread(event.threadId), `akra:${event.kind}:${event.threadId}`);
        }
      }),
    [openThread],
  );

  // 대화가 화면에 있는지 저장소에 알린다. 다른 보기에 있는 동안 끝난 턴은 읽지 않음이 된다. 대화로 돌아오면 지운다.
  useEffect(() => {
    const threadVisible = view === "thread";
    useDesktopStore.setState((state) => ({
      threadVisible,
      unreadThreadIds: threadVisible ? state.unreadThreadIds.filter((id) => id !== state.openThreadId) : state.unreadThreadIds,
    }));
  }, [view]);

  /** 업무 패널이 만든 글을 새 스레드 화면의 입력창에 채운다. 폴더는 열린 스레드의 것을 먼저 고른다. 보내기는 사용자가 한다. */
  const handOff = async (text: string) => {
    const cwd = openThreadCwd(useDesktopStore.getState());
    if (cwd) useUiStore.setState({ homeCwd: cwd });
    handOffToNewThread(text);
  };
  const handOffLinear = async (text: string) => {
    const currentView = useUiStore.getState().view;
    const cwd = currentView === "thread" ? openThreadCwd(useDesktopStore.getState()) : null;
    const folder = cwd ?? await window.akra.chooseFolder();
    if (!folder) return;
    useUiStore.setState({ homeCwd: folder });
    handOffToNewThread(text);
  };

  // 스킬 패널이 쓰는 두 함수이다 (#2383). 패널이 다시 읽지 않도록 한 번만 만든다.
  const listCodexSkills = useCallback(
    (cwd: string | null, forceReload: boolean) => session.current?.listSkills(cwd, forceReload) ?? Promise.reject(new Error("세션이 아직 시작되지 않았습니다.")),
    [],
  );
  const onSkillsChanged = useCallback((listener: () => void) => session.current?.onSkillsChanged(listener) ?? (() => undefined), []);

  // 패널이 접속마다 새 객체를 받지 않도록 한 번만 만든다. 요청은 그때의 세션으로 보낸다.
  const gatewayRequest = useMemo<GatewayRequest>(
    () =>
      function request<T>(method: string, params: unknown): Promise<T> {
        const current = session.current;
        return current ? current.requestGatewayService<T>(method, params) : Promise.reject(new Error("세션이 아직 시작되지 않았습니다."));
      },
    [],
  );

  // 스레드를 열 때와 턴이 끝날 때 그 스레드의 관련 문서를 읽는다 (plan 22). Gateway 가 이때 새 기록에서 문서를 찾아
  // 장부에 적는다. 패널이 닫혀 있어도 한다. 모델을 부르지 않는 조회이다.
  // 목록을 먼저 보이고, 그 뒤에 문서가 바뀌었는지 확인한다. 확인은 외부 서비스에 물어서 느릴 수 있다.
  // 확인이 실패하면 (이전 Gateway, 서비스에 닿지 않음) 목록을 그대로 둔다.
  const connected = useDesktopStore((state) => state.connection?.phase === "connected");
  const openThreadId = useDesktopStore((state) => state.openThreadId);
  const browserThreadId = view === "thread" ? openThreadId : null;
  useEffect(() => {
    void window.akra.browser.agent.setActiveThread(browserThreadId).catch((error: unknown) => toast(errorText(error)));
  }, [browserThreadId]);
  const turnInProgress = useDesktopStore((state) => isTurnInProgress(state.transcript));
  const [documents, setDocuments] = useState<ThreadDocumentsState>({ phase: "idle" });
  const [documentsRefresh, setDocumentsRefresh] = useState(0);
  useEffect(() => {
    if (!openThreadId || !connected) {
      setDocuments({ phase: "idle" });
      return;
    }
    if (turnInProgress) return;
    let current = true;
    setDocuments((previous) => (previous.phase === "loaded" && previous.threadId === openThreadId ? previous : { phase: "loading", threadId: openThreadId }));
    const show = (found: ThreadDocument[]) => current && setDocuments({ phase: "loaded", threadId: openThreadId, documents: found });
    fetchThreadDocuments(gatewayRequest, openThreadId).then(
      (found) => {
        show(found);
        if (current && found.length > 0) refreshThreadDocuments(gatewayRequest, openThreadId).then(show, () => undefined);
      },
      (error: unknown) => current && setDocuments({ phase: "failed", threadId: openThreadId, message: errorText(error) }),
    );
    return () => {
      current = false;
    };
  }, [openThreadId, connected, turnInProgress, documentsRefresh, gatewayRequest]);
  // 사용자가 바뀐 문서를 열었다. 실패하면 표시가 남고, 다음에 열 때 다시 한다.
  const markDocumentRead = (document: ThreadDocument) => {
    if (!openThreadId) return;
    const threadId = openThreadId;
    markThreadDocumentRead(gatewayRequest, threadId, document).then(
      (found) => setDocuments((previous) => (previous.phase === "loaded" && previous.threadId === threadId ? { ...previous, documents: found } : previous)),
      () => undefined,
    );
  };

  const composerActions: ComposerActions = {
    sendMessage: async (text) => session.current?.sendMessage(text),
    interrupt: async () => session.current?.interrupt(),
    startThread: async (text) => {
      const current = session.current;
      const cwd = useUiStore.getState().homeCwd ?? (await window.akra.chooseFolder());
      if (!cwd || !current) return false;
      const threadId = await current.startThread(cwd);
      visit({ view: "thread", threadId });
      // 스레드는 이미 만들었다. 첫 메시지가 실패하면 글을 그 스레드의 입력창에 되돌려서 다시 보낼 수 있게 한다.
      await current.sendMessage(text).catch((error: unknown) => {
        setDraft(threadId, text);
        toast(`첫 메시지를 보내지 못했습니다: ${errorText(error)}`);
      });
      return true;
    },
  };

  return (
    <div className="shell">
      {sidebarOpen && <Sidebar onOpenThread={openThread} onOpenRemoteThread={openRemoteThread} />}
      <div className="workspace">
        <TopBar onNavigate={navigate} tree={tree} documentCount={documents.phase === "loaded" ? documents.documents.length : null} changedDocumentCount={documents.phase === "loaded" ? changedCount(documents.documents) : 0} />
        <SessionBanner />
        <div className="workspace-body">
          <main className="main">
            {view === "home" && <Home actions={composerActions} onChooseFolder={() => window.akra.chooseFolder()} onOpenThread={openThread} />}
            {view === "fleet" && <Fleet onOpenThread={openThread} />}
            {view === "thread" && (
              <>
                <WriterElsewhereBanner onReconnect={async () => session.current?.reopenThread()} />
                <ConversationView onLoadOlder={async () => session.current?.loadOlderTurns()} />
                {/* 스레드마다 새로 만든다. 전송 중 표시와 오류가 다른 스레드의 입력창으로 넘어가지 않는다. 초안은 `uiStore` 에 남는다. */}
                <Composer key={openThreadId ?? ""} actions={composerActions} above={<ApprovalPrompt onAnswer={(requestId, decision) => session.current?.answerApproval(requestId, decision)} />} />
              </>
            )}
            {view === "remote" && remoteHost && remoteVisible && (
              <ConversationStoreContext.Provider value={remoteHostStore(remoteHost)}>
                {/* 클라우드 스레드는 읽기만 한다. 말을 거는 곳은 dot 의 대화(옆 칸)이다. */}
                <div className="banner" data-testid="remote-banner">
                  <span>클라우드 · 읽기 전용입니다. 메시지를 보내지 않습니다.</span>
                </div>
                <ConversationView onLoadOlder={async () => remoteSessions.current.get(remoteHost)?.loadOlderTurns()} />
              </ConversationStoreContext.Provider>
            )}
            <TerminalDock bridge={window.akra.terminal} />
          </main>
          {panel && (
            <RightPanel panel={panel}>
              {panel === "review" && <WorkingTreePanel tree={tree} git={window.akra} onOpenUrl={(url) => window.akra.github.openExternal(url)} />}
              {panel === "graph" && <GitGraphPanel request={gatewayRequest} cwd={openCwd ?? null} onOpenThread={openThread} />}
              {panel === "kernel" && <KernelPanel onRefresh={async () => session.current?.refreshKernel()} onOpenThread={openThread} />}
              {panel === "browser" && <BrowserPanel bridge={window.akra.browser} threadId={browserThreadId} onStartConversation={async () => {
                const current = session.current;
                const cwd = openCwd ?? useUiStore.getState().homeCwd ?? (await window.akra.chooseFolder());
                if (!cwd || !current) return;
                const threadId = await current.startBrowserThread(cwd, window.akra.browser.agent);
                visit({ view: "thread", threadId });
              }} />}
              {panel === "work" && <WorkTab request={gatewayRequest} onHandOff={handOff} onLinearHandOff={handOffLinear} />}
              {panel === "docs" && <DocumentsPanel state={documents} onRefresh={() => setDocumentsRefresh((count) => count + 1)} onOpenChanged={markDocumentRead} />}
              {panel === "skills" && <SkillsPanel cwd={openCwd ?? null} connected={connected} skills={window.akra.skills} listCodexSkills={listCodexSkills} onSkillsChanged={onSkillsChanged} />}
            </RightPanel>
          )}
          {dotDock !== null && dotDockVisible && <DotDock host={dotDock.host} dotId={dotDock.dotId} request={dotGatewayRequest} />}
        </div>
      </div>
      {overlay === "palette" && <CommandPalette onOpenThread={openThread} />}
      {overlay === "shortcuts" && <ShortcutsDialog />}
      {overlay === "settings" && <SettingsDialog />}
      <ZoomNotice />
      <Toasts />
    </div>
  );
}
