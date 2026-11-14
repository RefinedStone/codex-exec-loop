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
import { setDotSending, setDraft, useUiStore } from "../session/uiStore";
import type { GatewayRequest } from "../work/notionHub";
import { allowDotSending, dayLabel, dotStatusLine, hasDotRepliedSince, isDotSendingAllowed, isSentDraft, lastReadMessageId, messageClock, REPLY_POLL_MS, REPLY_WAIT_LIMIT_MS } from "./dotSend";

const DOTS_MESSAGES_METHOD = "akra/dots/messages";
const DOTS_SEND_METHOD = "akra/dots/send";
/** 답이 끝났다는 알림 뒤에 한 번 더 읽기까지의 시간이다. 메시지가 알림보다 조금 늦게 방에 들어올 때를 덮는다. */
const SETTLE_REFRESH_MS = 1_500;

type ChatState = { phase: "loading" } | { phase: "loaded"; messages: DotMessage[]; dotReadAt: string | null } | { phase: "failed"; message: string };

export interface DotChatProps {
  dot: DotSummary;
  /** 방의 실시간 상태이다. 아직 받지 못했으면 없다. */
  live: DotLive | undefined;
  /** 대화를 다시 읽어야 하는 횟수이다 (`dotRefreshes`): 답이 끝났거나 실시간 통로가 다시 붙었다. 늘면 읽는다. */
  replies: number;
  request: GatewayRequest;
  /** 작업 대화를 여는 곳이 있을 때만 준다. dot 창에는 없다. */
  onOpenWorkThread?(threadId: string): void;
  /** 사용자가 이 대화의 새 메시지를 봤다. */
  onSeen(): void;
  /** 이 대화를 작은 창으로 따로 연다. 이미 그 창이면 없다. */
  onPopOut?(): void;
}

export function DotAvatar({ name, size = 28 }: { name: string; size?: number }) {
  return (
    <span className="dot-avatar" style={{ width: size, height: size, fontSize: Math.round(size * 0.46) }} aria-hidden="true">
      {(name.trim()[0] ?? "?").toUpperCase()}
    </span>
  );
}

export function DotChat({ dot, live, replies, request, onOpenWorkThread, onSeen, onPopOut }: DotChatProps) {
  const [state, setState] = useState<ChatState>({ phase: "loading" });
  const [refresh, setRefresh] = useState(0);
  const [canSend, setCanSend] = useState(() => isDotSendingAllowed(dot.id));
  // 쓰던 글은 dot 마다 화면 밖에 둔다. 다른 dot 이나 작업 대화를 보고 돌아와도 남는다. 보내면 지운다.
  const draftKey = `dot:${dot.id}`;
  const draft = useUiStore((ui) => ui.drafts[draftKey] ?? "");
  // 전송 중인지도 dot 마다 화면 밖에 둔다. 답을 기다리는 사이 다른 화면을 보고 와도 다시 보낼 수 없다.
  const sending = useUiStore((ui) => ui.sendingDotIds.includes(dot.id));
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

  // 읽기는 한 번에 하나만 한다. 읽는 동안 온 요청(폴링, 답 알림)은 진행 중인 읽기를 버리지 않고, 그 읽기가 끝난
  // 뒤에 한 번 더 읽게 한다. 읽기가 간격보다 느려도 답을 잃지 않는다. 방이 바뀌면 진행 중인 읽기의 답은 버린다.
  const reading = useRef<{ roomId: string; again: boolean } | null>(null);
  const seen = useRef(onSeen);
  seen.current = onSeen;
  /** 창이 숨겨진 동안 대화를 새로 불러왔다. 창이 다시 보이면 읽은 것으로 한다. */
  const loadedWhileHidden = useRef(false);
  useEffect(() => {
    if (roomId === null) return;
    if (reading.current?.roomId === roomId) {
      reading.current.again = true;
      return;
    }
    const read = () => {
      const ticket = { roomId, again: false };
      reading.current = ticket;
      // 이미 보이는 대화는 다시 읽는 동안 그대로 둔다. 화면이 깜빡이지 않는다.
      setState((previous) => (previous.phase === "loaded" ? previous : { phase: "loading" }));
      // 이 읽기가 아직 유효하면 끝낸다. 밀린 요청이 있으면 다음 읽기를 시작한다. 돌려주는 값은 이 읽기가 가장 새
      // 읽기인지이다: 다음 읽기를 시작했으면 "latest" 가 아니다.
      const done = (): "stale" | "superseded" | "latest" => {
        if (reading.current !== ticket) return "stale";
        reading.current = null;
        if (!ticket.again) return "latest";
        read();
        return "superseded";
      };
      request<{ messages: DotMessage[]; dotReadAt?: string | null }>(DOTS_MESSAGES_METHOD, { roomId }).then(
        (response) => {
          const outcome = done();
          if (outcome === "stale") return;
          setState({ phase: "loaded", messages: response.messages, dotReadAt: response.dotReadAt ?? null });
          if (hasDotRepliedSince(response.messages, newestIdBeforeSend.current)) setWaitingSince(null);
          // 다음 읽기가 밀려 있으면 이 응답은 새 답을 담지 않았을 수 있다. 읽은 것으로 하지 않는다. 다음 읽기가
          // 성공하면 그때 한다.
          if (outcome === "superseded") return;
          // 창이 보일 때만 읽은 것으로 한다. 숨겨진 동안 불러온 답은 창이 다시 보일 때 처리한다 (아래).
          if (document.hidden) {
            loadedWhileHidden.current = true;
          } else {
            seen.current();
          }
        },
        (error: unknown) => {
          // 다음 읽기가 시작됐으면 그 결과를 기다린다. 이 실패로 화면을 바꾸지 않는다.
          if (done() === "latest") setState({ phase: "failed", message: error instanceof Error ? error.message : String(error) });
        },
      );
    };
    read();
  }, [roomId, refresh, request]);
  // 방이 바뀌거나 화면을 떠나면 진행 중인 읽기의 답을 쓰지 않는다.
  useEffect(
    () => () => {
      reading.current = null;
      loadedWhileHidden.current = false;
    },
    [roomId],
  );
  // 숨겨졌던 창이 다시 보이면, 숨겨진 동안 화면에 불러온 답만 읽은 것으로 한다. 아직 읽는 중이거나 읽기에
  // 실패했으면 화면에 답이 없으므로 표시를 지우지 않는다. 읽는 중이던 것은 끝날 때 위에서 처리한다.
  useEffect(() => {
    const onVisible = () => {
      if (document.hidden || !loadedWhileHidden.current) return;
      loadedWhileHidden.current = false;
      seen.current();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => document.removeEventListener("visibilitychange", onVisible);
  }, []);

  // 새 메시지가 오거나 dot 이 답을 만들기 시작하면 맨 아래를 보인다. 방은 최근 메시지만 주므로 개수는 그대로일 수
  // 있다. 가장 새 메시지의 id 로 새 메시지를 안다.
  const newestMessageId = state.phase === "loaded" ? (state.messages.at(-1)?.id ?? null) : null;
  useEffect(() => {
    scroller.current?.scrollTo({ top: scroller.current.scrollHeight });
  }, [newestMessageId, working]);

  const send = () => {
    const text = draft.trim();
    // 저장소의 지금 값도 본다. 그려진 값은 한 박자 늦을 수 있다.
    if (roomId === null || text.length === 0 || sending || useUiStore.getState().sendingDotIds.includes(dot.id)) return;
    const dotId = dot.id;
    setDotSending(dotId, true);
    setSendError(null);
    newestIdBeforeSend.current = state.phase === "loaded" ? (state.messages.at(-1)?.id ?? null) : null;
    request<{ sent: boolean }>(DOTS_SEND_METHOD, { roomId, text }).then(
      () => {
        // 보낸 글이 아직 초안으로 남아 있을 때만 지운다. 답을 기다리는 사이 다른 화면을 보고 와서 쓴 새 글은 남긴다.
        if (isSentDraft(useUiStore.getState().drafts[draftKey], text)) setDraft(draftKey, "");
        setDotSending(dotId, false);
        setRefresh((count) => count + 1);
        setWaitingSince(Date.now());
      },
      (error: unknown) => {
        // 실패하면 글을 입력창에 남긴다.
        setDotSending(dotId, false);
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
        {onPopOut && (
          <button type="button" className="icon-button" title="작은 창으로 따로 열기" aria-label="작은 창으로 따로 열기" onClick={onPopOut} data-testid="dot-pop-out">
            <Icon name="panel" />
          </button>
        )}
        {rootThreadId !== null && onOpenWorkThread && (
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
                onChange={(event) => setDraft(draftKey, event.target.value)}
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
