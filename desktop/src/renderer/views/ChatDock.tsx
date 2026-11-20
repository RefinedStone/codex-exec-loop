// 본 창의 옆 칸에 둔 ChatGPT 개인 대화이다 (plan 25). 읽기 전용이다: 사람과 ChatGPT 가 나눈 글을 보여 준다. dot
// 옆 칸(`DotDock`)과 같은 자리를 쓰지만, 보내기·실시간·중간 메시지가 없다. 서비스가 기록 조회를 제한하면(공식 앱도
// 같은 제한을 쓴다) 재시도 안내를 보인다. 접속은 본 창의 원격 세션을 쓴다.
import { useEffect, useRef, useState } from "react";

import { Markdown } from "../conversation/Markdown";
import { Icon } from "../design/icons";
import { useResizableWidth } from "../design/useResizableWidth";
import { rateLimitedMessage } from "../session/chatsProblem";
import { type ChatConversation, remoteHostStore } from "../session/desktopStore";
import { closeChat } from "../session/uiStore";
import type { GatewayRequest } from "../work/notionHub";

const CHATS_READ_METHOD = "akra/chats/read";
const MIN_DOCK_WIDTH = 320;
const MAX_DOCK_WIDTH = 640;

type ChatState =
  | { phase: "loading" }
  | { phase: "loaded"; conversation: ChatConversation }
  | { phase: "failed"; message: string };

export function ChatDock({ host, chatId, request }: { host: string; chatId: string; request: GatewayRequest }) {
  const useStore = remoteHostStore(host);
  const summary = useStore((state) => state.chats.find((candidate) => candidate.id === chatId) ?? null);
  const [width, startResize] = useResizableWidth(420, MIN_DOCK_WIDTH, MAX_DOCK_WIDTH);
  const [state, setState] = useState<ChatState>({ phase: "loading" });
  const [refresh, setRefresh] = useState(0);
  const scroller = useRef<HTMLElement | null>(null);

  useEffect(() => {
    let current = true;
    setState({ phase: "loading" });
    request<ChatConversation>(CHATS_READ_METHOD, { id: chatId }).then(
      (conversation) => {
        if (current) setState({ phase: "loaded", conversation });
      },
      (error: unknown) => {
        if (!current) return;
        setState({ phase: "failed", message: rateLimitedMessage(error) ?? (error instanceof Error ? error.message : String(error)) });
      },
    );
    return () => {
      current = false;
    };
  }, [chatId, request, refresh]);

  const title = state.phase === "loaded" ? state.conversation.title : (summary?.title ?? "대화");
  const messages = state.phase === "loaded" ? state.conversation.messages : [];
  useEffect(() => {
    scroller.current?.scrollTo({ top: 0 });
  }, [chatId]);

  return (
    <aside className="dot-dock" style={{ width }} aria-label={`${title} (ChatGPT 대화)`} data-testid="chat-dock">
      <div className="resize-handle" onPointerDown={startResize} />
      <div className="dot-chat" data-testid="chat-view">
        <header className="dot-header" data-testid="chat-banner">
          <div className="dot-identity">
            <span className="dot-name">{title}</span>
            <span className="dot-status dot-status-idle" data-testid="chat-source">
              <span className="dot-status-mark" />
              ChatGPT · 읽기 전용
            </span>
          </div>
          <span className="bar-spacer" />
          <button type="button" className="icon-button" title="새로 고침" aria-label="새로 고침" onClick={() => setRefresh((count) => count + 1)} data-testid="chat-refresh">
            <Icon name="refresh" />
          </button>
          <button type="button" className="icon-button" title="닫기" aria-label="닫기" onClick={closeChat} data-testid="chat-close">
            <Icon name="close" />
          </button>
        </header>
        <div className="conversation-wrap">
          <section className="conversation dot-messages" data-testid="chat-messages" ref={scroller}>
            <div className="column">
              {state.phase === "loading" && <p className="empty-row">대화를 읽는 중입니다.</p>}
              {state.phase === "failed" && <p className="empty-row error">{state.message}</p>}
              {state.phase === "loaded" && messages.length === 0 && <p className="empty-row">보여 줄 메시지가 없습니다.</p>}
              {messages.map((message) => {
                const fromAssistant = message.role === "assistant";
                return (
                  <article key={message.id} className={`dot-row ${fromAssistant ? "dot-row-agent" : "dot-row-me"}`} data-testid="chat-message" data-role={message.role}>
                    <div className="dot-bubble-column">
                      <div className={fromAssistant ? "dot-bubble dot-bubble-agent" : "dot-bubble dot-bubble-me"}>
                        {fromAssistant ? <Markdown text={message.text} /> : <p>{message.text}</p>}
                      </div>
                    </div>
                  </article>
                );
              })}
            </div>
          </section>
        </div>
      </div>
    </aside>
  );
}
