// 본 창의 옆 칸에 둔 dot 과의 대화이다 (D-19). 보기(스레드, 현황판, 새 스레드)와 따로 있다: 스레드로 일하는 동안
// 옆에서 비서에게 말을 건다. 머리줄의 단추로 따로 뜬 창으로 옮기거나 닫는다. 접속은 본 창의 원격 세션을 쓴다.
import { useEffect } from "react";

import { useResizableWidth } from "../design/useResizableWidth";
import { remoteHostStore } from "../session/desktopStore";
import { closeDot, dotDraftKey, setDraft, useUiStore } from "../session/uiStore";
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
          // 쓰던 글도 창으로 옮긴다. 창은 다른 renderer 이므로 이 창의 초안을 보지 못한다. 새 창이 글을 받았을 때만
          // 여기의 사본을 지운다: 창에서 보낸 글이 나중에 옆 칸에 초안으로 다시 나타나지 않는다. 다른 본 창이 이미
          // 그 dot 의 창을 열어 뒀으면 그 창은 글을 받지 않는다. 그때는 글을 여기에 남긴다.
          const key = dotDraftKey(dot.id);
          const moved = useUiStore.getState().drafts[key] ?? "";
          void window.akra.dots.openWindow(host, dot.id, moved).then(
            (opened) => {
              if (opened && (useUiStore.getState().drafts[key] ?? "") === moved) setDraft(key, "");
            },
            () => undefined,
          );
          closeDot();
        }}
        onClose={closeDot}
      />
    </aside>
  );
}
