// 화면 구성 (P3 §4): 왼쪽 사이드바(작업 폴더 → 스레드), 오른쪽 대화 기록. Gateway 정보는 상단에 작게 둔다.
import { useEffect, useMemo, useRef, useState } from "react";

import { BrowserPanel } from "./browser/BrowserPanel";
import { ApprovalPrompt } from "./conversation/ApprovalPrompt";
import { Composer, type ComposerActions } from "./conversation/Composer";
import { ConversationView } from "./conversation/ConversationView";
import { WorkingTreePanel } from "./conversation/WorkingTreePanel";
import { KernelPanel } from "./kernel/KernelPanel";
import { DesktopSession } from "./session/desktopSession";
import { desktopSessionStore, openThreadCwd, useDesktopStore } from "./session/desktopStore";
import { Sidebar } from "./sidebar/Sidebar";
import { TerminalDock } from "./terminal/TerminalDock";
import { GitHubPanel } from "./work/GitHubPanel";
import { isTurnInProgress } from "./conversation/transcript";
import { DocumentsPanel } from "./work/DocumentsPanel";
import { JiraPanel } from "./work/JiraPanel";
import type { GatewayRequest } from "./work/notionHub";
import { NotionPanel } from "./work/NotionPanel";
import { fetchThreadDocuments, type ThreadDocumentsState } from "./work/threadDocuments";

/** 오른쪽 패널은 한 번에 하나만 연다. */
type SidePanel = "browser" | "documents" | "github" | "jira" | "notion" | "kernel" | null;

const CLIENT_VERSION = "0.0.0-development";

function GatewayBar({
  sidePanel,
  documentCount,
  onToggle,
}: {
  sidePanel: SidePanel;
  documentCount: number | null;
  onToggle(panel: Exclude<SidePanel, null>): void;
}) {
  const gateway = useDesktopStore((state) => state.gateway);
  const versionCheck = useDesktopStore((state) => state.versionCheck);
  const sessionError = useDesktopStore((state) => state.sessionError);
  return (
    <header className="gateway-bar" data-testid="gateway-info">
      {gateway && (
        <span className="muted">
          codex <span data-testid="codex-version">{gateway.codexVersion ?? "알 수 없음"}</span> · gateway{" "}
          {gateway.gatewayVersion} · upstream 재시작 {gateway.upstreamRestarts}
        </span>
      )}
      {versionCheck && !versionCheck.matches && (
        <span className="warning" data-testid="version-warning">
          생성 타입은 codex {versionCheck.generated ?? "?"} 기준인데 Gateway 는 {versionCheck.gateway ?? "?"} 를 실행
          중입니다. 타입을 다시 생성하세요.
        </span>
      )}
      {sessionError && <span className="error">{sessionError}</span>}
      <span className="side-toggles">
        <button type="button" onClick={() => onToggle("kernel")} aria-pressed={sidePanel === "kernel"} data-testid="kernel-toggle">
          {sidePanel === "kernel" ? "Akra 닫기" : "Akra"}
        </button>
        <button type="button" onClick={() => onToggle("documents")} aria-pressed={sidePanel === "documents"} data-testid="documents-toggle">
          {sidePanel === "documents" ? "문서 닫기" : documentCount ? `문서 ${documentCount}` : "문서"}
        </button>
        <button type="button" onClick={() => onToggle("browser")} aria-pressed={sidePanel === "browser"} data-testid="browser-toggle">
          {sidePanel === "browser" ? "브라우저 닫기" : "브라우저"}
        </button>
        <button type="button" onClick={() => onToggle("github")} aria-pressed={sidePanel === "github"} data-testid="github-toggle">
          {sidePanel === "github" ? "GitHub 닫기" : "GitHub"}
        </button>
        <button type="button" onClick={() => onToggle("jira")} aria-pressed={sidePanel === "jira"} data-testid="jira-toggle">
          {sidePanel === "jira" ? "Jira 닫기" : "Jira"}
        </button>
        <button type="button" onClick={() => onToggle("notion")} aria-pressed={sidePanel === "notion"} data-testid="notion-toggle">
          {sidePanel === "notion" ? "Notion 닫기" : "Notion"}
        </button>
      </span>
    </header>
  );
}

export function App() {
  const session = useRef<DesktopSession | null>(null);
  const [sidePanel, setSidePanel] = useState<SidePanel>(null);
  const openCwd = useDesktopStore(openThreadCwd);

  useEffect(() => {
    const created = new DesktopSession(window.akra, desktopSessionStore(), CLIENT_VERSION);
    created.start();
    session.current = created;
    return () => created.stop();
  }, []);

  // 커널 패널이 열려 있을 때만 열린 스레드의 저장소 커널에 붙는다. 붙어 있는 동안 그 저장소의 TUI 는 열리지 않는다.
  const kernelWorkspace = sidePanel === "kernel" ? openCwd : null;
  useEffect(() => {
    void session.current?.showKernel(kernelWorkspace);
  }, [kernelWorkspace]);

  const startThread = (cwd: string) => {
    void session.current?.startThread(cwd).catch((error: unknown) =>
      useDesktopStore.getState().setSessionError(error instanceof Error ? error.message : String(error)),
    );
  };

  /** 업무 패널이 만든 글로 새 스레드를 만들고 입력창을 채운다. 폴더는 열린 스레드의 것, 없으면 고르게 한다. */
  const handOff = async (text: string) => {
    const cwd = useDesktopStore.getState().transcript?.cwd ?? (await window.akra.chooseFolder());
    if (!cwd || !session.current) return;
    const threadId = await session.current.startThread(cwd);
    useDesktopStore.getState().setComposerDraft({ threadId, text });
  };

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
  const connected = useDesktopStore((state) => state.connection?.phase === "connected");
  const openThreadId = useDesktopStore((state) => state.openThreadId);
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
    fetchThreadDocuments(gatewayRequest, openThreadId).then(
      (found) => current && setDocuments({ phase: "loaded", threadId: openThreadId, documents: found }),
      (error: unknown) => current && setDocuments({ phase: "failed", threadId: openThreadId, message: error instanceof Error ? error.message : String(error) }),
    );
    return () => {
      current = false;
    };
  }, [openThreadId, connected, turnInProgress, documentsRefresh, gatewayRequest]);

  const composerActions: ComposerActions = {
    sendMessage: async (text) => session.current?.sendMessage(text),
    interrupt: async () => session.current?.interrupt(),
  };

  return (
    <div className={sidePanel ? "layout with-side" : "layout"}>
      <Sidebar
        onOpenThread={(threadId) => void session.current?.openThread(threadId)}
        onStartThread={startThread}
        onStartThreadInNewFolder={() => {
          void window.akra.chooseFolder().then((cwd) => {
            if (cwd) {
              startThread(cwd);
            }
          });
        }}
      />
      <main className="main">
        <GatewayBar
          sidePanel={sidePanel}
          documentCount={documents.phase === "loaded" ? documents.documents.length : null}
          onToggle={(panel) => setSidePanel((open) => (open === panel ? null : panel))} />
        <WorkingTreePanel git={window.akra} />
        <ConversationView onLoadOlder={async () => session.current?.loadOlderTurns()} />
        <ApprovalPrompt onAnswer={(requestId, decision) => session.current?.answerApproval(requestId, decision)} />
        <Composer actions={composerActions} />
        <TerminalDock bridge={window.akra.terminal} />
      </main>
      {sidePanel === "browser" && <BrowserPanel bridge={window.akra.browser} />}
      {sidePanel === "documents" && <DocumentsPanel state={documents} onRefresh={() => setDocumentsRefresh((count) => count + 1)} />}
      {sidePanel === "github" && (
        <GitHubPanel github={window.akra.github} onHandOff={async (pull) => handOff(await window.akra.github.reviewPrompt(pull.url))} />
      )}
      {sidePanel === "kernel" && <KernelPanel onRefresh={async () => session.current?.refreshKernel()} />}
      {sidePanel === "jira" && <JiraPanel jira={window.akra.jira} onHandOff={async (issue) => handOff(await window.akra.jira.issuePrompt(issue.key))} />}
      {sidePanel === "notion" && <NotionPanel request={gatewayRequest} notion={window.akra.notion} onHandOff={handOff} />}
    </div>
  );
}
