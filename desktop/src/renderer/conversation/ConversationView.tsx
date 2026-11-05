// 열린 스레드의 대화 기록이다. 긴 대화는 보이는 턴만 그린다 (P3 §7, 가상 스크롤).
// 사용자가 맨 아래를 보고 있으면 새 내용이 올 때 맨 아래를 따라간다. 위로 올려 읽는 중이면 따라가지 않는다.
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";

import { useDesktopStore } from "../session/desktopStore";
import { ItemView } from "./ItemView";
import type { TurnView } from "./transcript";

const TURN_STATUS_LABEL: Record<string, string> = {
  inProgress: "진행 중",
  completed: "완료",
  interrupted: "중단됨",
  failed: "실패",
};
const ESTIMATED_TURN_HEIGHT = 320;
const BOTTOM_THRESHOLD_PX = 48;

const Turn = memo(function Turn({ turn }: { turn: TurnView }) {
  return (
    <article className="turn" data-testid="turn" data-status={turn.status}>
      {turn.items.map((item) => (
        <ItemView key={item.id} item={item} />
      ))}
      {turn.status !== "completed" && (
        <div className={`turn-status status-${turn.status}`}>{TURN_STATUS_LABEL[turn.status] ?? turn.status}</div>
      )}
      {turn.errorMessage && <div className="error">{turn.errorMessage}</div>}
    </article>
  );
});

export function ConversationView({ onLoadOlder }: { onLoadOlder(): Promise<void> }) {
  const openThreadId = useDesktopStore((state) => state.openThreadId);
  const transcript = useDesktopStore((state) => state.transcript);
  const transcriptError = useDesktopStore((state) => state.transcriptError);
  const scrollRef = useRef<HTMLElement>(null);
  const followBottom = useRef(true);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [olderError, setOlderError] = useState<string | null>(null);
  /** 이전 턴을 붙이기 전의 첫 턴 id 이다. 붙인 뒤 그 턴이 보던 자리에 오도록 스크롤한다. */
  const anchorTurnId = useRef<string | null>(null);
  const turns = transcript?.turns ?? [];

  const virtualizer = useVirtualizer({
    count: turns.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ESTIMATED_TURN_HEIGHT,
    getItemKey: (index) => turns[index]?.id ?? index,
    overscan: 3,
  });

  // 스레드를 새로 열면 맨 아래(최근 턴)부터 보여 준다.
  useEffect(() => {
    followBottom.current = true;
  }, [openThreadId]);

  // 내용이 바뀔 때마다, 맨 아래를 보고 있었으면 맨 아래로 간다. 이전 턴을 붙였으면 보던 턴으로 돌아간다.
  useLayoutEffect(() => {
    const anchor = anchorTurnId.current;
    if (anchor) {
      const index = turns.findIndex((turn) => turn.id === anchor);
      if (index > 0) {
        anchorTurnId.current = null;
        virtualizer.scrollToIndex(index, { align: "start" });
      }
      return;
    }
    if (followBottom.current && turns.length > 0) {
      virtualizer.scrollToIndex(turns.length - 1, { align: "end" });
    }
  }, [transcript, turns, virtualizer]);

  const loadOlder = async () => {
    setOlderError(null);
    setLoadingOlder(true);
    followBottom.current = false;
    anchorTurnId.current = turns[0]?.id ?? null;
    try {
      await onLoadOlder();
    } catch (error) {
      anchorTurnId.current = null;
      setOlderError(error instanceof Error ? error.message : String(error));
    } finally {
      setLoadingOlder(false);
    }
  };

  if (!openThreadId) {
    return <p className="muted empty">왼쪽에서 스레드를 고르세요.</p>;
  }
  if (transcriptError) {
    return <p className="error">{transcriptError}</p>;
  }
  if (!transcript) {
    return <p className="muted empty">불러오는 중…</p>;
  }

  const onScroll = () => {
    const element = scrollRef.current;
    if (element) {
      followBottom.current = element.scrollHeight - element.scrollTop - element.clientHeight < BOTTOM_THRESHOLD_PX;
    }
  };

  return (
    <section className="conversation" data-testid="conversation" ref={scrollRef} onScroll={onScroll}>
      <header className="conversation-header">
        <h2>{transcript.title}</h2>
        <span className="muted">{transcript.cwd}</span>
      </header>
      {transcript.olderTurnsCursor && (
        <div className="load-older">
          <button type="button" onClick={() => void loadOlder()} disabled={loadingOlder} data-testid="load-older">
            {loadingOlder ? "불러오는 중…" : "이전 턴 더 보기"}
          </button>
          {olderError && <span className="error">{olderError}</span>}
        </div>
      )}
      <div className="turn-list" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map((row) => {
          const turn = turns[row.index];
          return turn ? (
            <div
              key={row.key}
              data-index={row.index}
              ref={virtualizer.measureElement}
              className="turn-row"
              style={{ transform: `translateY(${row.start}px)` }}
            >
              <Turn turn={turn} />
            </div>
          ) : null;
        })}
      </div>
    </section>
  );
}
