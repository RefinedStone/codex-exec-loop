// 화면 구성 (P3 §4): 왼쪽 사이드바(작업 폴더 → 스레드), 오른쪽 대화 기록. Gateway 정보는 상단에 작게 둔다.
import { useEffect, useRef, useState } from "react";

import type { GitHubPullRequest } from "../shared/bridge";
import { BrowserPanel } from "./browser/BrowserPanel";
import { ApprovalPrompt } from "./conversation/ApprovalPrompt";
import { Composer, type ComposerActions } from "./conversation/Composer";
import { ConversationView } from "./conversation/ConversationView";
import { WorkingTreePanel } from "./conversation/WorkingTreePanel";
import { DesktopSession } from "./session/desktopSession";
import { desktopSessionStore, useDesktopStore } from "./session/desktopStore";
import { Sidebar } from "./sidebar/Sidebar";
import { TerminalDock } from "./terminal/TerminalDock";
import { GitHubPanel } from "./work/GitHubPanel";

/** 오른쪽 패널은 한 번에 하나만 연다. */
type SidePanel = "browser" | "github" | null;

const CLIENT_VERSION = "0.0.0-development";

function GatewayBar({ sidePanel, onToggle }: { sidePanel: SidePanel; onToggle(panel: Exclude<SidePanel, null>): void }) {
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
        <button type="button" onClick={() => onToggle("browser")} aria-pressed={sidePanel === "browser"} data-testid="browser-toggle">
          {sidePanel === "browser" ? "브라우저 닫기" : "브라우저"}
        </button>
        <button type="button" onClick={() => onToggle("github")} aria-pressed={sidePanel === "github"} data-testid="github-toggle">
          {sidePanel === "github" ? "GitHub 닫기" : "GitHub"}
        </button>
      </span>
    </header>
  );
}

export function App() {
  const session = useRef<DesktopSession | null>(null);
  const [sidePanel, setSidePanel] = useState<SidePanel>(null);

  useEffect(() => {
    const created = new DesktopSession(window.akra, desktopSessionStore(), CLIENT_VERSION);
    created.start();
    session.current = created;
    return () => created.stop();
  }, []);

  const startThread = (cwd: string) => {
    void session.current?.startThread(cwd).catch((error: unknown) =>
      useDesktopStore.getState().setSessionError(error instanceof Error ? error.message : String(error)),
    );
  };

  /** PR 의 해결되지 않은 리뷰 코멘트로 새 스레드를 만들고 입력창을 채운다. 폴더는 열린 스레드의 것, 없으면 고르게 한다. */
  const handOffReview = async (pull: GitHubPullRequest) => {
    const text = await window.akra.github.reviewPrompt(pull.url);
    const cwd = useDesktopStore.getState().transcript?.cwd ?? (await window.akra.chooseFolder());
    if (!cwd || !session.current) return;
    const threadId = await session.current.startThread(cwd);
    useDesktopStore.getState().setComposerDraft({ threadId, text });
  };

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
        <GatewayBar sidePanel={sidePanel} onToggle={(panel) => setSidePanel((open) => (open === panel ? null : panel))} />
        <WorkingTreePanel git={window.akra} />
        <ConversationView onLoadOlder={async () => session.current?.loadOlderTurns()} />
        <ApprovalPrompt onAnswer={(requestId, decision) => session.current?.answerApproval(requestId, decision)} />
        <Composer actions={composerActions} />
        <TerminalDock bridge={window.akra.terminal} />
      </main>
      {sidePanel === "browser" && <BrowserPanel bridge={window.akra.browser} />}
      {sidePanel === "github" && <GitHubPanel github={window.akra.github} onHandOff={handOffReview} />}
    </div>
  );
}
