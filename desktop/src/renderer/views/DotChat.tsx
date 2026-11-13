// dot 과의 대화 화면이다 (D-19). 메시지 방을 Gateway 의 `akra/dots/messages` 로 읽는다. 기본은 읽기 전용이다.
// 사용자가 이 dot 에 보내기를 한 번 허락하면 입력창이 나타나고 `akra/dots/send` 로 보낸다 ("세션 한 번 허용").
// 늘 묻지는 않는다 (plan 25 §8). 화면을 열 때와 "새로 고침"을 누를 때 읽고, 보낸 뒤에는 dot 의 답이 올 때까지만
// 짧은 간격으로 다시 읽는다. 답이 오거나 기다리는 시간이 지나면 멈춘다.
import { type KeyboardEvent, useEffect, useRef, useState } from "react";

import { Icon } from "../design/icons";

import { Markdown } from "../conversation/Markdown";
import type { DotMessage, DotSummary } from "../session/desktopStore";
import type { GatewayRequest } from "../work/notionHub";
import { allowDotSending, hasDotRepliedSince, isDotSendingAllowed, messageTime, REPLY_POLL_MS, REPLY_WAIT_LIMIT_MS } from "./dotSend";

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
  /** 보낸 뒤 답을 기다리기 시작한 시각이다. null 이면 기다리지 않는다. */
  const [waitingSince, setWaitingSince] = useState<number | null>(null);
  /** 보내기 직전의 가장 새 메시지 id 이다. 그 뒤에 온 dot 의 메시지가 답이다. */
  const newestIdBeforeSend = useRef<string | null>(null);
  const scroller = useRef<HTMLElement | null>(null);
  const roomId = dot.messagingRoomId;
  useEffect(() => {
    setCanSend(isDotSendingAllowed(dot.id));
    setDraft("");
    setSendError(null);
    setWaitingSince(null);
    setState({ phase: "loading" });
  }, [dot.id]);
  // 답을 기다리는 동안만 다시 읽는다.
  useEffect(() => {
    if (waitingSince === null) return;
    const timer = window.setInterval(() => {
      if (Date.now() - waitingSince > REPLY_WAIT_LIMIT_MS) {
        setWaitingSince(null);
      } else {
        setRefresh((count) => count + 1);
      }
    }, REPLY_POLL_MS);
    return () => window.clearInterval(timer);
  }, [waitingSince]);
  // 새 메시지가 오면 맨 아래를 보인다.
  const messageCount = state.phase === "loaded" ? state.messages.length : 0;
  useEffect(() => {
    scroller.current?.scrollTo({ top: scroller.current.scrollHeight });
  }, [messageCount]);

  const send = () => {
    const text = draft.trim();
    if (roomId === null || text.length === 0 || sending) return;
    setSending(true);
    setSendError(null);
    newestIdBeforeSend.current = state.phase === "loaded" ? (state.messages.at(-1)?.id ?? null) : null;
    request<{ sent: boolean }>(DOTS_SEND_METHOD, { roomId, text }).then(
      () => {
        setDraft("");
        setSending(false);
        // 보낸 글을 보이려고 바로 다시 읽고, dot 의 답이 올 때까지 짧은 간격으로 읽는다.
        setRefresh((count) => count + 1);
        setWaitingSince(Date.now());
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
    // 이미 보이는 대화는 다시 읽는 동안 그대로 둔다. 답을 기다리며 읽을 때 화면이 깜빡이지 않는다.
    setState((previous) => (previous.phase === "loaded" ? previous : { phase: "loading" }));
    request<{ messages: DotMessage[] }>(DOTS_MESSAGES_METHOD, { roomId }).then(
      (response) => {
        if (!current) return;
        setState({ phase: "loaded", messages: response.messages });
        if (hasDotRepliedSince(response.messages, newestIdBeforeSend.current)) setWaitingSince(null);
      },
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
        {waitingSince !== null && <span data-testid="dot-waiting">답을 기다리는 중입니다…</span>}
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
        <section className="conversation" data-testid="dot-chat" ref={scroller}>
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
