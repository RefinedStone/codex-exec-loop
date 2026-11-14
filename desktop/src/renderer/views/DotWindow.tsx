// dot 하나와의 대화만 보이는 작은 창이다 (D-19). 본 창과 따로 띄워 두고 비서에게 말 걸듯 쓴다. 이 창은 자기
// Gateway 접속을 하나 연다: 그 호스트의 원격 접속이다. `local` 에는 붙지 않는다. 작업 대화와 다른 패널은 없다.
import { useCallback, useEffect, useRef } from "react";

import { DesktopSession, remoteHostBridge } from "../session/desktopSession";
import { desktopSessionStore, remoteHostStore } from "../session/desktopStore";
import type { GatewayRequest } from "../work/notionHub";
import { DotChat } from "./DotChat";

export function DotWindow({ host, dotId, clientVersion }: { host: string; dotId: string; clientVersion: string }) {
  const useStore = remoteHostStore(host);
  const session = useRef<DesktopSession | null>(null);
  useEffect(() => {
    const created = new DesktopSession(remoteHostBridge(window.akra.remote, host), desktopSessionStore(useStore), clientVersion, undefined, true);
    session.current = created;
    created.start();
    return () => {
      created.stop();
      session.current = null;
      window.akra.remote.close(host);
    };
  }, [host, clientVersion, useStore]);
  // 이 창은 운영체제의 밝기 설정을 따른다. 본 창의 밝기 단추는 이 창에 없다.
  useEffect(() => {
    const dark = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      document.documentElement.dataset.theme = dark.matches ? "dark" : "light";
    };
    apply();
    dark.addEventListener("change", apply);
    return () => dark.removeEventListener("change", apply);
  }, []);

  const dot = useStore((state) => state.dots.find((candidate) => candidate.id === dotId) ?? null);
  const phase = useStore((state) => state.connection?.phase ?? "connecting");
  const problem = useStore((state) => (state.connection?.phase === "connected" ? state.sessionError : (state.connection?.detail ?? null)));
  const room = dot?.messagingRoomId ?? null;
  const live = useStore((state) => (room ? state.dotLive[room] : undefined));
  const replies = useStore((state) => (room ? (state.dotRefreshes[room] ?? 0) : 0));
  useEffect(() => {
    document.title = dot ? `${dot.displayName} · Akra` : "Akra";
  }, [dot]);
  const request = useCallback<GatewayRequest>((method, params) => {
    const current = session.current;
    return current ? current.requestGatewayService(method, params) : Promise.reject(new Error("Gateway 에 연결되어 있지 않습니다."));
  }, []);

  if (!dot) {
    return (
      <div className="dot-window" data-testid="dot-window">
        <p className={problem ? "empty error" : "empty muted"} data-testid="dot-window-state">
          {problem ?? (phase === "connected" ? "에이전트를 찾는 중입니다." : "Gateway 에 연결하는 중입니다.")}
        </p>
      </div>
    );
  }
  return (
    <div className="dot-window" data-testid="dot-window">
      <DotChat dot={dot} live={live} replies={replies} request={request} onSeen={() => room && useStore.getState().markDotRoomRead(room)} />
    </div>
  );
}
