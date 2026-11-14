// 본 창의 옆 칸에 둔 dot 과의 대화이다 (D-19). 보기(스레드, 현황판, 새 스레드)와 따로 있다: 스레드로 일하는 동안
// 옆에서 비서에게 말을 건다. 머리줄의 단추로 따로 뜬 창으로 옮기거나 닫는다. 접속은 본 창의 원격 세션을 쓴다.
import { useEffect } from "react";

import { useResizableWidth } from "../design/useResizableWidth";
import { remoteHostStore } from "../session/desktopStore";
import { closeDot, dotDraftKey, useUiStore } from "../session/uiStore";
import type { GatewayRequest } from "../work/notionHub";
import { DotChat } from "./DotChat";

const MIN_DOCK_WIDTH = 320;
const MAX_DOCK_WIDTH = 640;

export function DotDock({ host, dotId, request }: { host: string; dotId: string; request: GatewayRequest }) {
  const useStore = remoteHostStore(host);
  const dot = useStore((state) => state.dots.find((candidate) => candidate.id === dotId) ?? null);
  const room = dot?.messagingRoomId ?? null;
  const live = useStore((state) => (room ? state.dotLive[room] : undefined));
  const replies = useStore((state) => (room ? (state.dotRefreshes[room] ?? 0) : 0));
  const [width, startResize] = useResizableWidth(400, MIN_DOCK_WIDTH, MAX_DOCK_WIDTH);
  // dot 이 목록에서 빠지면 옆 칸을 닫는다.
  useEffect(() => {
    if (dot === null) closeDot();
  }, [dot]);
  if (dot === null) return null;
  return (
    <aside className="dot-dock" style={{ width }} aria-label={`${dot.displayName} 과의 대화`} data-testid="dot-dock">
      <div className="resize-handle" onPointerDown={startResize} />
      <DotChat
        dot={dot}
        live={live}
        replies={replies}
        request={request}
        onSeen={() => room && useStore.getState().markDotRoomRead(room)}
        onPopOut={() => {
          // 쓰던 글도 창으로 옮긴다. 창은 다른 renderer 이므로 이 창의 초안을 보지 못한다.
          window.akra.dots.openWindow(host, dot.id, useUiStore.getState().drafts[dotDraftKey(dot.id)] ?? "");
          closeDot();
        }}
        onClose={closeDot}
      />
    </aside>
  );
}
