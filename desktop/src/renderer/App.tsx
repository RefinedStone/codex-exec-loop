// 화면 구성 (P3 §4): 왼쪽 사이드바(작업 폴더 → 스레드), 오른쪽 대화 기록. Gateway 정보는 상단에 작게 둔다.
import { useEffect, useRef } from "react";

import { ConversationView } from "./conversation/ConversationView";
import { DesktopSession } from "./session/desktopSession";
import { desktopSessionStore, useDesktopStore } from "./session/desktopStore";
import { Sidebar } from "./sidebar/Sidebar";

const CLIENT_VERSION = "0.0.0-development";

function GatewayBar() {
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
    </header>
  );
}

export function App() {
  const session = useRef<DesktopSession | null>(null);

  useEffect(() => {
    const created = new DesktopSession(window.akra, desktopSessionStore(), CLIENT_VERSION);
    created.start();
    session.current = created;
    return () => created.stop();
  }, []);

  return (
    <div className="layout">
      <Sidebar onOpenThread={(threadId) => void session.current?.openThread(threadId)} />
      <main className="main">
        <GatewayBar />
        <ConversationView />
      </main>
    </div>
  );
}
