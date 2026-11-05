// P2 시험 화면: 연결 상태, Gateway·codex 버전, 작업 폴더 목록을 보여 준다. 대화 화면은 P3 에서 만든다.
import { useEffect } from "react";

import type { ConnectionState } from "../shared/bridge";
import { useDesktopStore } from "./session/desktopStore";
import { startDesktopSession } from "./session/desktopSession";

const CLIENT_VERSION = "0.0.0-development";

const PHASE_LABEL: Record<ConnectionState["phase"], string> = {
  connecting: "연결 중",
  connected: "연결됨",
  disconnected: "끊김",
};

export function App() {
  const { connection, gateway, versionCheck, folders, sessionError } = useDesktopStore();

  useEffect(() => startDesktopSession(window.akra, useDesktopStore.getState(), CLIENT_VERSION), []);

  return (
    <main className="shell">
      <header className="status-bar">
        <span className={`status status-${connection?.phase ?? "connecting"}`} data-testid="connection-phase">
          {PHASE_LABEL[connection?.phase ?? "connecting"]}
        </span>
        <span className="endpoint">{connection?.endpoint ?? ""}</span>
        {connection?.phase === "disconnected" && connection.retryInMs !== undefined && (
          <span className="retry">
            {Math.round(connection.retryInMs / 1000)}초 뒤 다시 연결 {connection.detail ? `(${connection.detail})` : ""}
          </span>
        )}
      </header>

      {sessionError && <p className="error">{sessionError}</p>}

      {gateway && (
        <section className="panel" data-testid="gateway-info">
          <h2>Gateway</h2>
          <dl>
            <dt>codex</dt>
            <dd data-testid="codex-version">{gateway.codexVersion ?? "알 수 없음"}</dd>
            <dt>gateway</dt>
            <dd>{gateway.gatewayVersion}</dd>
            <dt>upstream 재시작</dt>
            <dd>{gateway.upstreamRestarts}</dd>
          </dl>
          {versionCheck && !versionCheck.matches && (
            <p className="warning" data-testid="version-warning">
              생성 타입은 codex {versionCheck.generated ?? "?"} 기준인데 Gateway 는 {versionCheck.gateway ?? "?"} 를
              실행 중입니다. 타입을 다시 생성하세요.
            </p>
          )}
        </section>
      )}

      <section className="panel" data-testid="folders">
        <h2>작업 폴더</h2>
        {folders.length === 0 && <p className="muted">스레드가 없습니다.</p>}
        <ul>
          {folders.map((folder) => (
            <li key={folder.cwd}>
              <strong>{folder.cwd}</strong>
              <span className="muted"> · 스레드 {folder.threads.length}개</span>
            </li>
          ))}
        </ul>
      </section>
    </main>
  );
}
