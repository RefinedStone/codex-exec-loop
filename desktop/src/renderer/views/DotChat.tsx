// dot 과의 대화 화면이다 (D-19). 메시지 방을 Gateway 의 `akra/dots/messages` 로 읽는다. 기본은 읽기 전용이다.
// 사용자가 이 dot 에 보내기를 한 번 허락하면 입력창이 나타나고 `akra/dots/send` 로 보낸다 ("세션 한 번 허용").
// 주기적으로 묻지 않는다. 화면을 열 때, 보낸 뒤, "새로 고침"을 누를 때만 읽는다 (plan 25 §8).
import { type KeyboardEvent, useEffect, useState } from "react";

import { Icon } from "../design/icons";

import { Markdown } from "../conversation/Markdown";
import type { DotMessage, DotSummary } from "../session/desktopStore";
import type { GatewayRequest } from "../work/notionHub";
import { allowDotSending, isDotSendingAllowed, messageTime } from "./dotSend";

const DOTS_MESSAGES_METHOD = "akra/dots/messages";
const DOTS_SEND_METHOD = "akra/dots/send";

type ChatState = { phase: "loading" } | { phase: "loaded"; messages: DotMessage[] } | { phase: "failed"; message: string };

export function DotChat({ dot, request, onOpenWorkThread }: { dot: DotSummary; request: GatewayRequest; onOpenWorkThread(threadId: string): void }) {
  const [state, setState] = useState<ChatState>({ phase: "loading" });
  const [refresh, setRefresh] = useState(0);
  const [canSend, setCanSend] = useState(() => isDotSendingAllowed(dot.id));
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const roomId = dot.messagingRoomId;
  useEffect(() => {
    setCanSend(isDotSendingAllowed(dot.id));
    setDraft("");
    setSendError(null);
  }, [dot.id]);

  const send = () => {
    const text = draft.trim();
    if (roomId === null || text.length === 0 || sending) return;
    setSending(true);
    setSendError(null);
    request<{ sent: boolean }>(DOTS_SEND_METHOD, { roomId, text }).then(
      () => {
        setDraft("");
        setSending(false);
        // 보낸 글을 보이려고 한 번 다시 읽는다. dot 의 답은 "새로 고침"으로 본다.
        setRefresh((count) => count + 1);
      },
      (error: unknown) => {
        // 실패하면 글을 입력창에 남긴다.
        setSending(false);
        setSendError(error instanceof Error ? error.message : String(error));
      },
    );
  };
  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      send();
    }
  };
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
          클라우드 에이전트 · {dot.displayName} · {canSend ? "보내기 허용됨 (이 세션). 실계정에 메시지를 보냅니다." : "대화는 읽기 전용입니다."}
        </span>
        {roomId !== null && !canSend && (
          <button type="button" className="banner-action" onClick={() => setCanSend(allowDotSending(dot.id, (message) => window.confirm(message)))} data-testid="dot-allow-send">
            이 세션에서 보내기 허용
          </button>
        )}
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
      {roomId !== null && canSend && (
        <div className="composer-area">
          <div className="column">
            {sendError && <p className="error">{sendError}</p>}
            <div className="composer">
              <textarea
                value={draft}
                rows={1}
                onChange={(event) => setDraft(event.target.value)}
                onKeyDown={onKeyDown}
                placeholder={`${dot.displayName} 에게 메시지`}
                disabled={sending}
                data-testid="dot-composer-input"
              />
              <div className="composer-bar">
                <span className="bar-spacer" />
                <button type="button" className="round-button send" title="보내기" aria-label="보내기" onClick={send} disabled={sending || draft.trim().length === 0} data-testid="dot-composer-send">
                  <Icon name="arrowUp" />
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
