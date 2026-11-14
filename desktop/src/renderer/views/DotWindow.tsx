// dot 하나와의 대화만 보이는 작은 창이다 (D-19). 본 창과 따로 띄워 두고 비서에게 말 걸듯 쓴다. 이 창은 자기
// Gateway 접속을 하나 연다: 그 호스트의 원격 접속이다. `local` 에는 붙지 않는다. 작업 대화와 다른 패널은 없다.
// 머리줄의 단추로 대화를 본 창의 옆 칸으로 되돌린다. 이 창이 열려 있는 동안 답의 알림은 이 창이 낸다.
import { useCallback, useEffect, useRef } from "react";

import { DesktopSession, remoteHostBridge } from "../session/desktopSession";
import { desktopSessionStore, remoteHostStore } from "../session/desktopStore";
import { chosenTheme, useUiStore } from "../session/uiStore";
import type { GatewayRequest } from "../work/notionHub";
import { DotChat } from "./DotChat";
import { notifyDotReply } from "./dotNotify";

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
  // 이 창은 본 창과 같은 밝기를 쓴다. 본 창이 밝기를 바꾸면 저장소 이벤트로 따라간다.
  const theme = useUiStore((state) => state.theme);
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);
  useEffect(() => {
    const follow = () => useUiStore.setState({ theme: chosenTheme() });
    window.addEventListener("storage", follow);
    return () => window.removeEventListener("storage", follow);
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
  // dot 이 답을 끝냈는데 이 창이 가려져 있으면 운영체제 알림을 보인다. 누르면 이 창이 앞으로 온다.
  const answered = useStore((state) => (room ? (state.dotReplies[room] ?? 0) : 0));
  const dotName = useRef("");
  dotName.current = dot?.displayName ?? "";
  useEffect(() => {
    if (answered === 0 || dotName.current === "") return;
    if (document.hidden || !document.hasFocus()) notifyDotReply(dotName.current, () => window.akra.dots.openWindow(host, dotId));
  }, [answered, host, dotId]);
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
      <DotChat dot={dot} live={live} replies={replies} request={request} onSeen={() => room && useStore.getState().markDotRoomRead(room)} onDock={() => window.akra.dots.dock(host, dotId)} />
    </div>
  );
}
