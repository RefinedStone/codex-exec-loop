// 입력창이다. 위에는 승인 요청, 계획 진행, 대기 메시지를 붙인다. 아래에는 모델, 추론 수준, 권한 선택이 있다.
import { type KeyboardEvent, useEffect, useRef, useState } from "react";

import { PlanList } from "./Conversation";
import { Icon } from "../design/icons";
import { Picker } from "../design/Picker";
import { EFFORTS, MENTION_FILES, MODELS, PERMISSIONS, SLASH_COMMANDS, type PlanStep } from "./mockData";
import { answerApproval, editQueued, interrupt, notConnected, removeQueued, sendMessage, startThread, toast, useBlueprint } from "./store";

function ApprovalCard({ threadId }: { threadId: string }) {
  const approval = useBlueprint((state) => state.approvals[threadId]);
  useEffect(() => {
    if (!approval) return;
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey)) return;
      if (event.key === "Enter") {
        event.preventDefault();
        answerApproval(threadId, true);
      } else if (event.key === "Backspace") {
        event.preventDefault();
        answerApproval(threadId, false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [approval, threadId]);
  if (!approval) return null;
  return (
    <div className="approval-card" role="alertdialog" aria-label="명령 실행 승인">
      <header>
        <Icon name="alert" />
        <strong>명령 실행 승인</strong>
        <span className="approval-cwd">{approval.cwd}</span>
      </header>
      <pre className="approval-command">
        <span className="prompt">$</span> {approval.command}
      </pre>
      <p>{approval.reason}</p>
      <div className="approval-buttons">
        <button type="button" className="button primary" onClick={() => answerApproval(threadId, true)}>
          허용 <kbd>⌘⏎</kbd>
        </button>
        <button
          type="button"
          className="button"
          onClick={() => {
            toast("이 세션 동안 같은 명령을 허용합니다 (mock)");
            answerApproval(threadId, true);
          }}
        >
          이 세션 동안 허용
        </button>
        <button type="button" className="button" onClick={() => answerApproval(threadId, false)}>
          거부 <kbd>⌘⌫</kbd>
        </button>
      </div>
    </div>
  );
}

function PlanStrip({ steps }: { steps: readonly PlanStep[] }) {
  const [open, setOpen] = useState(false);
  const done = steps.filter((step) => step.state === "done").length;
  const active = steps.find((step) => step.state === "active");
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
        {active && <span className="plan-active">{active.text}</span>}
      </button>
      {open && <PlanList steps={steps} />}
    </div>
  );
}

function QueuedCards({ threadId }: { threadId: string }) {
  const queued = useBlueprint((state) => state.queued[threadId]) ?? [];
  const [editing, setEditing] = useState<number | null>(null);
  if (queued.length === 0) return null;
  return (
    <ul className="queued">
      {queued.map((text, index) => (
        <li key={`${index}:${text}`}>
          <Icon name="clock" size={13} />
          {editing === index ? (
            <input
              className="queued-edit"
              autoFocus
              defaultValue={text}
              aria-label="대기 메시지 편집"
              onBlur={(event) => {
                editQueued(threadId, index, event.currentTarget.value.trim() || text);
                setEditing(null);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.nativeEvent.isComposing) event.currentTarget.blur();
                if (event.key === "Escape") setEditing(null);
              }}
            />
          ) : (
            <button type="button" className="queued-text" title="눌러서 편집" onClick={() => setEditing(index)}>
              {text}
            </button>
          )}
          <button
            type="button"
            className="ghost"
            onClick={() => {
              removeQueued(threadId, index);
              toast("진행 중인 턴에 바로 전달했습니다 (mock)");
            }}
          >
            지금 지시
          </button>
          <button type="button" className="icon-button" title="대기 메시지 삭제" onClick={() => removeQueued(threadId, index)}>
            <Icon name="close" size={13} />
          </button>
        </li>
      ))}
    </ul>
  );
}

/** 입력 끝의 `/명령` 또는 `@파일` 조각을 찾는다. 없으면 null 이다. */
function completionOf(draft: string): { trigger: "/" | "@"; query: string } | null {
  const slash = /^\/(\S*)$/.exec(draft);
  if (slash) return { trigger: "/", query: slash[1] ?? "" };
  const mention = /(?:^|\s)@(\S*)$/.exec(draft);
  if (mention) return { trigger: "@", query: mention[1] ?? "" };
  return null;
}

function ContextRing({ percent }: { percent: number }) {
  const circumference = 2 * Math.PI * 7;
  return (
    <span className="context-ring" title={`컨텍스트 ${percent}% 사용 (mock)`}>
      <svg width="18" height="18" viewBox="0 0 18 18">
        <circle cx="9" cy="9" r="7" fill="none" stroke="var(--line-strong)" strokeWidth="2" />
        <circle cx="9" cy="9" r="7" fill="none" stroke="var(--accent)" strokeWidth="2" strokeDasharray={circumference} strokeDashoffset={circumference * (1 - percent / 100)} transform="rotate(-90 9 9)" strokeLinecap="round" />
      </svg>
      {percent}%
    </span>
  );
}

/** `threadId` 가 null 이면 홈 화면의 입력창이다. 보내면 새 스레드를 만든다. */
export function Composer({ threadId }: { threadId: string | null }) {
  const [draft, setDraft] = useState("");
  const input = useRef<HTMLTextAreaElement>(null);
  const running = useBlueprint((state) => {
    const status = state.threads.find((thread) => thread.id === threadId)?.status;
    return status === "running" || status === "approval";
  });
  const livePlan = useBlueprint((state) => {
    const turn = threadId ? state.turns[threadId]?.at(-1) : undefined;
    if (!turn || turn.status !== "running") return null;
    const plan = turn.items.findLast((item) => item.type === "plan");
    return plan?.type === "plan" ? plan.steps : null;
  });
  const model = useBlueprint((state) => state.model);
  const effort = useBlueprint((state) => state.effort);
  const permission = useBlueprint((state) => state.permission);
  const noteCount = useBlueprint((state) => state.reviewNotes.length);
  const completion = completionOf(draft);
  const choices =
    completion?.trigger === "/"
      ? SLASH_COMMANDS.filter((command) => command.name.startsWith(`/${completion.query}`)).map((command) => ({ value: command.name, hint: command.hint }))
      : completion?.trigger === "@"
        ? MENTION_FILES.filter((path) => path.toLowerCase().includes(completion.query.toLowerCase())).map((path) => ({ value: `@${path}`, hint: "" }))
        : [];

  const handOff = useBlueprint((state) => state.handOff);

  useEffect(() => {
    input.current?.focus();
  }, [threadId]);

  // 업무 패널이 넘긴 글은 새 스레드 입력창에 채운다. 바로 보내지 않는다.
  useEffect(() => {
    if (handOff !== null && threadId === null) {
      setDraft(handOff);
      useBlueprint.setState({ handOff: null });
    }
  }, [handOff, threadId]);

  // 내용에 맞춰 입력창 높이를 늘린다.
  useEffect(() => {
    const element = input.current;
    if (!element) return;
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, 220)}px`;
  }, [draft]);

  const choose = (value: string) => {
    setDraft(completion?.trigger === "/" ? `${value} ` : draft.replace(/@\S*$/, `${value} `));
    input.current?.focus();
  };

  const send = () => {
    const text = draft.trim();
    if (text.length === 0) return;
    const withNotes = noteCount > 0 ? `${text}\n\n(리뷰 메모 ${noteCount}개 첨부)` : text;
    if (threadId) {
      sendMessage(threadId, withNotes);
    } else {
      startThread(withNotes);
    }
    useBlueprint.setState({ reviewNotes: [] });
    setDraft("");
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      const first = choices[0];
      if (completion && first) {
        choose(first.value);
      } else {
        send();
      }
    }
  };

  return (
    <div className="composer-area">
      <div className="column">
        {threadId && <ApprovalCard threadId={threadId} />}
        {livePlan && <PlanStrip steps={livePlan} />}
        {threadId && <QueuedCards threadId={threadId} />}
        <div className="composer">
          {choices.length > 0 && (
            <ul className="menu completion">
              {choices.map((choice, index) => (
                <li key={choice.value}>
                  <button type="button" className={index === 0 ? "first" : undefined} onClick={() => choose(choice.value)}>
                    <code>{choice.value}</code>
                    <span className="menu-hint">{choice.hint}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
          {noteCount > 0 && (
            <div className="composer-chips">
              <span className="chip">
                <Icon name="message" size={12} />
                리뷰 메모 {noteCount}개
                <button type="button" className="chip-close" title="리뷰 메모 삭제" onClick={() => useBlueprint.setState({ reviewNotes: [] })}>
                  <Icon name="close" size={11} />
                </button>
              </span>
            </div>
          )}
          <textarea
            ref={input}
            value={draft}
            rows={1}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={onKeyDown}
            placeholder={running ? "추가 메시지는 대기 목록에 들어갑니다" : "무엇을 할까요?  @ 파일 · / 명령"}
          />
          <div className="composer-bar">
            <button type="button" className="icon-button" title="파일 첨부" onClick={() => notConnected("파일 첨부")}>
              <Icon name="paperclip" />
            </button>
            <button
              type="button"
              className="icon-button"
              title="파일 언급"
              onClick={() => {
                setDraft(`${draft}${draft.length === 0 || draft.endsWith(" ") ? "" : " "}@`);
                input.current?.focus();
              }}
            >
              <Icon name="at" />
            </button>
            <span className="bar-divider" />
            <Picker value={model} options={MODELS} onChange={(value) => useBlueprint.setState({ model: value })} />
            <Picker icon={<Icon name="sparkle" size={12} />} value={effort} options={EFFORTS} onChange={(value) => useBlueprint.setState({ effort: value })} />
            <Picker icon={<Icon name="eye" size={12} />} value={permission} options={PERMISSIONS} onChange={(value) => useBlueprint.setState({ permission: value })} />
            <span className="bar-spacer" />
            {threadId && <ContextRing percent={38} />}
            {running && threadId && (
              <button type="button" className="round-button stop" title="중단" onClick={() => interrupt(threadId)}>
                <span className="stop-square" />
              </button>
            )}
            <button type="button" className="round-button send" title={running ? "대기 목록에 넣기" : "보내기"} onClick={send} disabled={draft.trim().length === 0}>
              <Icon name="arrowUp" />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
