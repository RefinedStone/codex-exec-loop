// dot 과의 대화 화면이다 (D-19). 메시지 방을 Gateway 의 `akra/dots/messages` 로 읽는다. 읽기 전용이다: 메시지를
// 보내지 않는다. 주기적으로 묻지 않는다. 화면을 열 때와 "새로 고침"을 누를 때만 읽는다 (plan 25 §8).
import { useEffect, useState } from "react";

import { Markdown } from "../conversation/Markdown";
import type { DotMessage, DotSummary } from "../session/desktopStore";
import type { GatewayRequest } from "../work/notionHub";

const DOTS_MESSAGES_METHOD = "akra/dots/messages";

type ChatState = { phase: "loading" } | { phase: "loaded"; messages: DotMessage[] } | { phase: "failed"; message: string };

/** 서비스의 시각 글자를 이 컴퓨터의 날짜와 시각으로 보인다. 읽을 수 없으면 그대로 보인다. */
export function messageTime(createdAt: string): string {
  const time = new Date(createdAt);
  return Number.isNaN(time.getTime()) ? createdAt : time.toLocaleString();
}

export function DotChat({ dot, request, onOpenWorkThread }: { dot: DotSummary; request: GatewayRequest; onOpenWorkThread(threadId: string): void }) {
  const [state, setState] = useState<ChatState>({ phase: "loading" });
  const [refresh, setRefresh] = useState(0);
  const roomId = dot.messagingRoomId;
  useEffect(() => {
    if (roomId === null) return;
    let current = true;
    setState({ phase: "loading" });
    request<{ messages: DotMessage[] }>(DOTS_MESSAGES_METHOD, { roomId }).then(
      (response) => current && setState({ phase: "loaded", messages: response.messages }),
      (error: unknown) => current && setState({ phase: "failed", message: error instanceof Error ? error.message : String(error) }),
    );
    return () => {
      current = false;
    };
  }, [roomId, refresh, request]);

  const rootThreadId = dot.rootThreadId;
  return (
    <>
      <div className="banner" data-testid="dot-banner">
        <span>
          클라우드 에이전트 · {dot.displayName} · 대화는 읽기 전용입니다.
        </span>
        {roomId !== null && (
          <button type="button" className="banner-action" onClick={() => setRefresh((count) => count + 1)} data-testid="dot-refresh">
            새로 고침
          </button>
        )}
        {rootThreadId !== null && (
          <button type="button" className="banner-action" onClick={() => onOpenWorkThread(rootThreadId)} data-testid="dot-open-work-thread">
            작업 대화 열기
          </button>
        )}
      </div>
      <div className="conversation-wrap">
        <section className="conversation" data-testid="dot-chat">
          {roomId === null && <p className="empty-row">이 에이전트에는 메시지 방이 없습니다.</p>}
          {roomId !== null && state.phase === "loading" && <p className="empty-row">대화를 읽는 중입니다.</p>}
          {roomId !== null && state.phase === "failed" && <p className="empty-row error">{state.message}</p>}
          {roomId !== null && state.phase === "loaded" && state.messages.length === 0 && <p className="empty-row">메시지가 없습니다.</p>}
          {roomId !== null &&
            state.phase === "loaded" &&
            state.messages.map((message) => (
              <article className="turn" key={message.id} data-testid="dot-message" data-from-dot={message.fromDot}>
                <div className={message.fromDot ? "agent-message" : "user-message"} title={`${message.author || "알 수 없음"} · ${messageTime(message.createdAt)}`}>
                  {message.fromDot ? <Markdown text={message.text} /> : <p>{message.text}</p>}
                </div>
              </article>
            ))}
        </section>
      </div>
    </>
  );
}
