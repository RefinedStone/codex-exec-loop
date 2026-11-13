// dot 과의 대화 화면이다 (D-19). 비서와 나누는 대화처럼 보인다: 머리줄에 이름과 지금 상태, 날짜로 나눈 말풍선,
// dot 이 답을 만드는 동안의 표시, 내 글의 "읽음".
//
// 읽기: 메시지 방을 Gateway 의 `akra/dots/messages` 로 읽는다. 화면을 열 때, "새로 고침"을 누를 때, dot 이 답을
// 끝냈을 때(`replies` 가 늘 때) 읽는다. 실시간 통로가 끊겨 있을 때만, 보낸 뒤 답이 올 때까지 짧은 간격으로 읽는다.
// 쓰기: 기본은 읽기 전용이다. 사용자가 이 dot 에 보내기를 한 번 허락하면 입력창이 나타나고 `akra/dots/send` 로
// 보낸다 ("세션 한 번 허용").
import { type KeyboardEvent, useEffect, useRef, useState } from "react";

import { Markdown } from "../conversation/Markdown";
import { Icon } from "../design/icons";
import type { DotLive, DotMessage, DotSummary } from "../session/desktopStore";
import type { GatewayRequest } from "../work/notionHub";
import { allowDotSending, dayLabel, dotStatusLine, hasDotRepliedSince, isDotSendingAllowed, lastReadMessageId, messageClock, REPLY_POLL_MS, REPLY_WAIT_LIMIT_MS } from "./dotSend";

const DOTS_MESSAGES_METHOD = "akra/dots/messages";
const DOTS_SEND_METHOD = "akra/dots/send";
/** 답이 끝났다는 알림 뒤에 한 번 더 읽기까지의 시간이다. 메시지가 알림보다 조금 늦게 방에 들어올 때를 덮는다. */
const SETTLE_REFRESH_MS = 1_500;

type ChatState = { phase: "loading" } | { phase: "loaded"; messages: DotMessage[]; dotReadAt: string | null } | { phase: "failed"; message: string };

export interface DotChatProps {
  dot: DotSummary;
  /** 방의 실시간 상태이다. 아직 받지 못했으면 없다. */
  live: DotLive | undefined;
  /** dot 이 답을 끝낸 횟수이다. 늘면 대화를 다시 읽는다. */
  replies: number;
  request: GatewayRequest;
  onOpenWorkThread(threadId: string): void;
  /** 사용자가 이 대화의 새 메시지를 봤다. */
  onSeen(): void;
}

export function DotAvatar({ name, size = 28 }: { name: string; size?: number }) {
  return (
    <span className="dot-avatar" style={{ width: size, height: size, fontSize: Math.round(size * 0.46) }} aria-hidden="true">
      {(name.trim()[0] ?? "?").toUpperCase()}
    </span>
  );
}

export function DotChat({ dot, live, replies, request, onOpenWorkThread, onSeen }: DotChatProps) {
  const [state, setState] = useState<ChatState>({ phase: "loading" });
  const [refresh, setRefresh] = useState(0);
  const [canSend, setCanSend] = useState(() => isDotSendingAllowed(dot.id));
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  /** 보낸 뒤 답을 기다리기 시작한 시각이다. null 이면 기다리지 않는다. 실시간 통로가 없을 때만 쓴다. */
  const [waitingSince, setWaitingSince] = useState<number | null>(null);
  /** 보내기 직전의 가장 새 메시지 id 이다. 그 뒤에 온 dot 의 메시지가 답이다. */
  const newestIdBeforeSend = useRef<string | null>(null);
  const scroller = useRef<HTMLElement | null>(null);
  const roomId = dot.messagingRoomId;
  const liveConnected = live?.connected === true;
  const working = liveConnected && live.working;

  useEffect(() => {
    setCanSend(isDotSendingAllowed(dot.id));
    setDraft("");
    setSendError(null);
    setWaitingSince(null);
    setState({ phase: "loading" });
  }, [dot.id]);

  // dot 이 답을 끝냈다. 바로 읽고, 조금 뒤에 한 번 더 읽는다.
  useEffect(() => {
    if (replies === 0) return;
    setRefresh((count) => count + 1);
    const timer = window.setTimeout(() => setRefresh((count) => count + 1), SETTLE_REFRESH_MS);
    return () => window.clearTimeout(timer);
  }, [replies]);

  // 실시간 통로가 없을 때의 대비이다. 보낸 뒤 답이 올 때까지만 다시 읽는다.
  useEffect(() => {
    if (waitingSince === null || liveConnected) return;
    const timer = window.setInterval(() => {
      if (Date.now() - waitingSince > REPLY_WAIT_LIMIT_MS) {
        setWaitingSince(null);
      } else {
        setRefresh((count) => count + 1);
      }
    }, REPLY_POLL_MS);
    return () => window.clearInterval(timer);
  }, [waitingSince, liveConnected]);

  useEffect(() => {
    if (roomId === null) return;
    let current = true;
    // 이미 보이는 대화는 다시 읽는 동안 그대로 둔다. 화면이 깜빡이지 않는다.
    setState((previous) => (previous.phase === "loaded" ? previous : { phase: "loading" }));
    request<{ messages: DotMessage[]; dotReadAt?: string | null }>(DOTS_MESSAGES_METHOD, { roomId }).then(
      (response) => {
        if (!current) return;
        setState({ phase: "loaded", messages: response.messages, dotReadAt: response.dotReadAt ?? null });
        if (hasDotRepliedSince(response.messages, newestIdBeforeSend.current)) setWaitingSince(null);
        onSeen();
      },
      (error: unknown) => current && setState({ phase: "failed", message: error instanceof Error ? error.message : String(error) }),
    );
    return () => {
      current = false;
    };
    // onSeen 은 부모가 매번 새로 만든다. 의존 목록에 넣지 않는다: 넣으면 그릴 때마다 다시 읽는다.
  }, [roomId, refresh, request]);

  // 새 메시지가 오거나 dot 이 답을 만들기 시작하면 맨 아래를 보인다.
  const messageCount = state.phase === "loaded" ? state.messages.length : 0;
  useEffect(() => {
    scroller.current?.scrollTo({ top: scroller.current.scrollHeight });
  }, [messageCount, working]);

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

  const rootThreadId = dot.rootThreadId;
  const messages = state.phase === "loaded" ? state.messages : [];
  const readId = state.phase === "loaded" ? lastReadMessageId(messages, state.dotReadAt) : null;
  const status = dotStatusLine(dot, live);
  return (
    <div className="dot-chat" data-testid="dot-view">
      <header className="dot-header" data-testid="dot-banner">
        <DotAvatar name={dot.displayName} size={36} />
        <div className="dot-identity">
          <span className="dot-name">{dot.displayName}</span>
          <span className={`dot-status dot-status-${status.tone}`} data-testid="dot-status">
            <span className="dot-status-mark" />
            {status.text}
          </span>
        </div>
        <span className="bar-spacer" />
        {roomId !== null && (
          <button type="button" className="icon-button" title="새로 고침" aria-label="새로 고침" onClick={() => setRefresh((count) => count + 1)} data-testid="dot-refresh">
            <Icon name="undo" />
          </button>
        )}
        {rootThreadId !== null && (
          <button type="button" className="chip-button" title="이 에이전트의 작업 대화를 엽니다 (읽기 전용)" onClick={() => onOpenWorkThread(rootThreadId)} data-testid="dot-open-work-thread">
            <Icon name="stack" size={14} />
            작업 대화
          </button>
        )}
      </header>
      <div className="conversation-wrap">
        <section className="conversation dot-messages" data-testid="dot-chat" ref={scroller}>
          <div className="column">
            {roomId === null && <p className="empty-row">이 에이전트에는 메시지 방이 없습니다.</p>}
            {roomId !== null && state.phase === "loading" && <p className="empty-row">대화를 읽는 중입니다.</p>}
            {roomId !== null && state.phase === "failed" && <p className="empty-row error">{state.message}</p>}
            {roomId !== null && state.phase === "loaded" && messages.length === 0 && <p className="empty-row">아직 나눈 대화가 없습니다.</p>}
            {messages.map((message, index) => {
              const previous = messages[index - 1];
              const newDay = previous === undefined || dayLabel(previous.createdAt) !== dayLabel(message.createdAt);
              // 같은 사람이 이어서 보낸 글은 한 묶음으로 붙인다.
              const grouped = !newDay && previous?.fromDot === message.fromDot;
              return (
                <div key={message.id}>
                  {newDay && (
                    <div className="dot-day" data-testid="dot-day">
                      <span>{dayLabel(message.createdAt)}</span>
                    </div>
                  )}
                  <article className={`dot-row ${message.fromDot ? "dot-row-agent" : "dot-row-me"}${grouped ? " dot-row-grouped" : ""}`} data-testid="dot-message" data-from-dot={message.fromDot}>
                    {message.fromDot && (grouped ? <span className="dot-avatar-gap" /> : <DotAvatar name={message.author || dot.displayName} />)}
                    <div className="dot-bubble-column">
                      <div className={message.fromDot ? "dot-bubble dot-bubble-agent" : "dot-bubble dot-bubble-me"}>{message.fromDot ? <Markdown text={message.text} /> : <p>{message.text}</p>}</div>
                      <span className="dot-meta">
                        {messageClock(message.createdAt)}
                        {message.id === readId && <span data-testid="dot-read"> · 읽음</span>}
                      </span>
                    </div>
                  </article>
                </div>
              );
            })}
            {working && (
              <article className="dot-row dot-row-agent" data-testid="dot-typing">
                <DotAvatar name={dot.displayName} />
                <div className="dot-bubble dot-bubble-agent dot-typing" role="status" aria-label={`${dot.displayName} 이(가) 답을 만드는 중입니다`}>
                  <span className="dot-typing-marks" aria-hidden="true">
                    <i />
                    <i />
                    <i />
                  </span>
                  {live.activityLabel && <span className="dot-typing-label">{live.activityLabel}</span>}
                </div>
              </article>
            )}
          </div>
        </section>
      </div>
      {roomId !== null && !canSend && (
        <div className="dot-consent" data-testid="dot-consent">
          <span>지금은 읽기 전용입니다. 보내면 실계정의 대화에 남고, {dot.displayName} 이(가) 답하면서 모델을 씁니다.</span>
          <button type="button" className="chip-button selected" onClick={() => setCanSend(allowDotSending(dot.id, (message) => window.confirm(message)))} data-testid="dot-allow-send">
            이 세션에서 보내기 허용
          </button>
        </div>
      )}
      {roomId !== null && canSend && (
        <div className="composer-area">
          <div className="column">
            {sendError && <p className="error">{sendError}</p>}
            {waitingSince !== null && !liveConnected && (
              <p className="muted" data-testid="dot-waiting">
                답을 기다리는 중입니다…
              </p>
            )}
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
    </div>
  );
}
