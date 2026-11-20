// 본 창의 옆 칸에 둔 ChatGPT 개인 대화이다 (plan 25). 읽기 전용이다: 사람과 ChatGPT 가 나눈 글을 보여 준다. dot
// 옆 칸(`DotDock`)과 같은 자리를 쓰지만, 보내기·실시간·중간 메시지가 없다. 서비스가 기록 조회를 제한하면(공식 앱도
// 같은 제한을 쓴다) 재시도 안내를 보인다. 접속은 본 창의 원격 세션을 쓴다.
import { useEffect, useState } from "react";

import { Markdown } from "../conversation/Markdown";
import { Icon } from "../design/icons";
import { useResizableWidth } from "../design/useResizableWidth";
import { rateLimitedMessage, staleNotice } from "../session/chatsProblem";
import { type ChatConversation, remoteHostStore } from "../session/desktopStore";
import { closeChat } from "../session/uiStore";
import type { GatewayRequest } from "../work/notionHub";

const CHATS_READ_METHOD = "akra/chats/read";
const MIN_DOCK_WIDTH = 320;
const MAX_DOCK_WIDTH = 640;

export function ChatDock({ host, chatId, request }: { host: string; chatId: string; request: GatewayRequest }) {
  const useStore = remoteHostStore(host);
  const summary = useStore((state) => state.chats.find((candidate) => candidate.id === chatId) ?? null);
  const [width, startResize] = useResizableWidth(420, MIN_DOCK_WIDTH, MAX_DOCK_WIDTH);
  const [conversation, setConversation] = useState<ChatConversation | null>(null);
  const [loading, setLoading] = useState(true);
  const [problem, setProblem] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);

  useEffect(() => {
    let current = true;
    request<ChatConversation>(CHATS_READ_METHOD, { id: chatId }).then(
      (conversation) => {
        if (!current) return;
        setConversation(conversation);
        setProblem(staleNotice(conversation));
        setLoading(false);
      },
      (error: unknown) => {
        if (!current) return;
        setProblem(rateLimitedMessage(error) ?? (error instanceof Error ? error.message : String(error)));
        setLoading(false);
      },
    );
    return () => {
      current = false;
    };
  }, [chatId, request, refresh]);

  const title = conversation?.title ?? summary?.title ?? "대화";
  const messages = conversation?.messages ?? [];

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
          {/* 읽는 중에는 또 보내지 않는다: 겹친 요청이 조회 제한을 부른다. */}
          <button type="button" className="icon-button" title="새로 고침" aria-label="새로 고침" onClick={() => { setLoading(true); setRefresh((count) => count + 1); }} disabled={loading} data-testid="chat-refresh">
            <Icon name="refresh" />
          </button>
          <button type="button" className="icon-button" title="닫기" aria-label="닫기" onClick={closeChat} data-testid="chat-close">
            <Icon name="close" />
          </button>
        </header>
        {problem !== null && <p className="empty-row error" data-testid={conversation?.limited ? "chat-stale" : "chat-problem"}>{problem}</p>}
        <div className="conversation-wrap">
          <section className="conversation dot-messages" data-testid="chat-messages" aria-busy={loading}>
            <div className="column">
              {conversation === null && loading && <p className="empty-row">대화를 읽는 중입니다.</p>}
              {conversation !== null && messages.length === 0 && <p className="empty-row">보여 줄 메시지가 없습니다.</p>}
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
