// 입력창 (P3 §6.2). 위에는 승인 요청과 계획 진행을 붙인다 (R10). Enter 는 보내기, Shift+Enter 는 줄바꿈이다.
// 턴이 진행 중이면 보내기는 steer 가 되고 중단 버튼이 보인다. 연결이 끊기면 입력을 막는다 (§6.5).
// `home` 이면 새 스레드 화면의 입력창이다. 보내면 새 스레드를 만들고 첫 메시지를 보낸다.
import { type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from "react";

import type { TurnPlanStep } from "../../shared/protocol/generated/v2/TurnPlanStep";
import { Icon } from "../design/icons";
import { useDesktopStore } from "../session/desktopStore";
import { HOME_DRAFT, setDraft as setDraftFor, useUiStore } from "../session/uiStore";
import { PlanList } from "./ConversationView";
import { contextPercent, isTurnInProgress } from "./transcript";

export interface ComposerActions {
  sendMessage(text: string): Promise<void>;
  interrupt(): Promise<void>;
  /** 새 스레드를 만들고 첫 메시지를 보낸다. 사용자가 폴더 고르기를 취소하면 false 이다. */
  startThread(text: string): Promise<boolean>;
}

const MAX_INPUT_HEIGHT_PX = 220;

function PlanStrip({ steps }: { steps: readonly TurnPlanStep[] }) {
  const [open, setOpen] = useState(false);
  const done = steps.filter((step) => step.status === "completed").length;
  const active = steps.find((step) => step.status === "inProgress");
  return (
    <div className="plan-strip">
      <button type="button" className="plan-strip-line" onClick={() => setOpen(!open)} aria-expanded={open}>
        <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />
        <span className="plan-progress">
          계획 {done}/{steps.length}
        </span>
        <span className="plan-bar">
          <span style={{ width: `${(done / steps.length) * 100}%` }} />
        </span>
        {active && <span className="plan-active">{active.step}</span>}
      </button>
      {open && <PlanList steps={steps} />}
    </div>
  );
}

function ContextRing({ percent }: { percent: number }) {
  const circumference = 2 * Math.PI * 7;
  return (
    <span className="context-ring" title={`컨텍스트 ${percent}% 사용`} data-testid="context-ring">
      <svg width="18" height="18" viewBox="0 0 18 18">
        <circle cx="9" cy="9" r="7" fill="none" stroke="var(--line-strong)" strokeWidth="2" />
        <circle cx="9" cy="9" r="7" fill="none" stroke="var(--accent)" strokeWidth="2" strokeDasharray={circumference} strokeDashoffset={circumference * (1 - percent / 100)} transform="rotate(-90 9 9)" strokeLinecap="round" />
      </svg>
      {percent}%
    </span>
  );
}

export function Composer({ actions, home = false, above }: { actions: ComposerActions; home?: boolean; above?: ReactNode }) {
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const input = useRef<HTMLTextAreaElement>(null);
  const connected = useDesktopStore((state) => state.connection?.phase === "connected");
  const transcript = useDesktopStore((state) => (home ? null : state.transcript));
  const inProgress = isTurnInProgress(transcript);
  const livePlan = inProgress ? (transcript?.turns.at(-1)?.plan ?? null) : null;
  const percent = contextPercent(transcript);
  // 초안은 화면 저장소에 둔다. 보기를 바꾸거나 업무 패널이 글을 넘겨도 같은 값을 본다.
  const draftKey = home ? HOME_DRAFT : (transcript?.threadId ?? "");
  const draft = useUiStore((state) => state.drafts[draftKey] ?? "");
  const setDraft = (text: string) => setDraftFor(draftKey, text);
  const disabled = !connected || (!home && !transcript) || sending;

  useEffect(() => {
    if (!disabled) input.current?.focus();
  }, [transcript?.threadId, home, disabled]);

  // 내용에 맞춰 입력창 높이를 늘린다.
  useEffect(() => {
    const element = input.current;
    if (!element) return;
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, MAX_INPUT_HEIGHT_PX)}px`;
  }, [draft]);

  const run = async (action: () => Promise<void>) => {
    setError(null);
    setSending(true);
    try {
      await action();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSending(false);
    }
  };

  const send = () => {
    const text = draft;
    if (text.trim().length === 0 || disabled) {
      return;
    }
    void run(async () => {
      if (home) {
        if (!(await actions.startThread(text))) return;
      } else {
        await actions.sendMessage(text);
      }
      setDraft("");
    });
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      send();
    }
  };

  const sendTitle = inProgress ? "추가 지시" : "보내기";
  return (
    <div className="composer-area">
      <div className="column">
        {above}
        {livePlan && livePlan.length > 0 && <PlanStrip steps={livePlan} />}
        {!connected && <p className="muted">연결이 끊겨 입력할 수 없습니다.</p>}
        {error && <p className="error">{error}</p>}
        <div className="composer">
          <textarea
            ref={input}
            value={draft}
            rows={1}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={onKeyDown}
            placeholder={inProgress ? "진행 중인 턴에 추가 지시 (steer)" : "무엇을 할까요?"}
            disabled={disabled}
            data-testid="composer-input"
          />
          <div className="composer-bar">
            <span className="bar-spacer" />
            {percent !== null && <ContextRing percent={percent} />}
            {inProgress && (
              <button type="button" className="round-button stop" title="중단" onClick={() => void run(actions.interrupt)} disabled={!connected} data-testid="composer-interrupt">
                <span className="stop-square" />
              </button>
            )}
            <button type="button" className="round-button send" title={sendTitle} aria-label={sendTitle} onClick={send} disabled={disabled || draft.trim().length === 0} data-testid="composer-send">
              <Icon name="arrowUp" />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
