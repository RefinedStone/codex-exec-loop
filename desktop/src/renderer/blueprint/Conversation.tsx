// 대화 화면이다. 도구 호출은 한 줄 요약으로 접고, 계획은 체크 목록으로, 변경은 요약 카드로 보여 준다.
import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { Markdown } from "../conversation/Markdown";
import { Icon, type IconName } from "./icons";
import type { ChangedFile, MockItem, MockStep, MockTurn, PlanStep, StepKind } from "./mockData";
import { notConnected, useBlueprint } from "./store";

const STEP_ICON: Record<StepKind, IconName> = { read: "file", search: "search", edit: "pencil", command: "terminal", tool: "bolt" };
const STEP_VERB: Record<StepKind, string> = { read: "읽음", search: "검색", edit: "수정", command: "실행", tool: "도구" };

function StepRow({ step }: { step: MockStep }) {
  const [open, setOpen] = useState(step.status === "failed");
  return (
    <li className={`step step-${step.status}`}>
      <button type="button" className="step-line" onClick={() => setOpen(!open)} disabled={!step.output} aria-expanded={open}>
        <Icon name={STEP_ICON[step.kind]} size={13} />
        <span className="step-verb">{STEP_VERB[step.kind]}</span>
        <code>{step.label}</code>
        {step.status === "running" && <span className="spinner" />}
        {step.status === "failed" && <span className="badge badge-bad">실패</span>}
        {step.duration && <span className="step-duration">{step.duration}</span>}
        {step.output && <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />}
      </button>
      {open && step.output && <pre className="step-output">{step.output}</pre>}
    </li>
  );
}

function activitySummary(steps: readonly MockStep[]): string {
  const count = (kind: StepKind) => steps.filter((step) => step.kind === kind).length;
  const parts = [
    count("read") > 0 ? `파일 ${count("read")}개 읽음` : null,
    count("search") > 0 ? `검색 ${count("search")}회` : null,
    count("edit") > 0 ? `파일 ${count("edit")}개 수정` : null,
    count("command") > 0 ? `명령 ${count("command")}개 실행` : null,
  ];
  return parts.filter(Boolean).join(" · ");
}

/** 진행 중이거나 실패가 있으면 펼친다. 끝난 작업 묶음은 한 줄로 접는다. */
function Activity({ steps }: { steps: MockStep[] }) {
  const live = steps.some((step) => step.status === "running");
  const failed = steps.some((step) => step.status === "failed");
  const [open, setOpen] = useState<boolean | null>(null);
  const shown = open ?? (live || failed);
  return (
    <div className="activity">
      <button type="button" className="activity-summary" onClick={() => setOpen(!shown)} aria-expanded={shown}>
        <Icon name={shown ? "chevronDown" : "chevronRight"} size={12} />
        <span>{activitySummary(steps)}</span>
        {failed && <span className="badge badge-bad">실패 1</span>}
      </button>
      {shown && (
        <ul className="steps">
          {steps.map((step) => (
            <StepRow key={step.id} step={step} />
          ))}
        </ul>
      )}
    </div>
  );
}

export function PlanList({ steps }: { steps: readonly PlanStep[] }) {
  return (
    <ol className="plan-steps">
      {steps.map((step) => (
        <li key={step.text} className={`plan-step plan-${step.state}`}>
          <span className="plan-mark">{step.state === "done" && <Icon name="check" size={11} />}</span>
          {step.text}
        </li>
      ))}
    </ol>
  );
}

function Changes({ files }: { files: ChangedFile[] }) {
  const added = files.reduce((sum, file) => sum + file.added, 0);
  const removed = files.reduce((sum, file) => sum + file.removed, 0);
  return (
    <div className="changes-card">
      <header>
        <Icon name="diff" />
        <span>파일 {files.length}개 변경</span>
        <span className="counts">
          <span className="count-add">+{added}</span> <span className="count-del">−{removed}</span>
        </span>
        <button type="button" className="button" onClick={() => useBlueprint.setState({ panel: "review" })}>
          변경 검토
        </button>
      </header>
      <ul>
        {files.map((file) => (
          <li key={file.path}>
            <code>{file.path}</code>
            <span className="counts">
              <span className="count-add">+{file.added}</span> <span className="count-del">−{file.removed}</span>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function Item({ item }: { item: MockItem }) {
  switch (item.type) {
    case "reasoning":
      return (
        <details className="reasoning">
          <summary>
            <Icon name="sparkle" size={13} />
            {item.seconds}초 동안 생각함
          </summary>
          <p>{item.text}</p>
        </details>
      );
    case "activity":
      return <Activity steps={item.steps} />;
    case "plan":
      return (
        <div className="plan-card">
          <header>
            <Icon name="check" size={13} />
            계획 · {item.steps.filter((step) => step.state === "done").length}/{item.steps.length}
          </header>
          <PlanList steps={item.steps} />
        </div>
      );
    case "changes":
      return <Changes files={item.files} />;
    case "agent":
      return (
        <div className="agent-message">
          <Markdown text={item.text} />
        </div>
      );
  }
}

function useElapsed(startedAt: number): string {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const seconds = Math.max(0, Math.round((now - startedAt) / 1000));
  return seconds >= 60 ? `${Math.floor(seconds / 60)}분 ${seconds % 60}초` : `${seconds}초`;
}

function workingLabel(turn: MockTurn, waitingForApproval: boolean): string {
  if (waitingForApproval) return "승인을 기다리는 중";
  const last = turn.items.at(-1);
  if (last?.type === "activity" && last.steps.some((step) => step.status === "running")) return "명령을 실행하는 중";
  if (last?.type === "agent") return "답을 쓰는 중";
  return "생각하는 중";
}

function Working({ turn, threadId }: { turn: MockTurn; threadId: string }) {
  const startedAt = useBlueprint((state) => state.runStartedAt[threadId]) ?? Date.now();
  const waiting = useBlueprint((state) => state.approvals[threadId] !== undefined);
  const elapsed = useElapsed(startedAt);
  return (
    <div className={waiting ? "working waiting" : "working"}>
      <span className="working-dot" />
      <span className="shimmer">{workingLabel(turn, waiting)}</span>
      <span className="working-elapsed">{elapsed}</span>
    </div>
  );
}

function Turn({ turn, threadId }: { turn: MockTurn; threadId: string }) {
  return (
    <article className="turn">
      <div className="user-message">
        {turn.attachments.map((name) => (
          <span key={name} className="chip">
            <Icon name="paperclip" size={12} />
            {name}
          </span>
        ))}
        <p>{turn.user}</p>
      </div>
      {turn.items.map((item) => (
        <Item key={item.id} item={item} />
      ))}
      {turn.status === "running" && <Working turn={turn} threadId={threadId} />}
      {turn.status === "interrupted" && <div className="turn-note">중단했습니다</div>}
      {turn.status === "done" && (
        <footer className="turn-footer">
          <span>{turn.summary}</span>
          <button type="button" className="ghost" onClick={() => notConnected("복사")}>
            복사
          </button>
          <button type="button" className="ghost" onClick={() => notConnected("이 지점으로 되돌리기")}>
            여기로 되돌리기
          </button>
        </footer>
      )}
    </article>
  );
}

const BOTTOM_THRESHOLD_PX = 64;
const USER_SCROLL_WINDOW_MS = 600;

export function Conversation({ threadId }: { threadId: string }) {
  const turns = useBlueprint((state) => state.turns[threadId]) ?? [];
  const scrollRef = useRef<HTMLDivElement>(null);
  const [atBottom, setAtBottom] = useState(true);
  const follow = useRef(true);

  useEffect(() => {
    follow.current = true;
    setAtBottom(true);
  }, [threadId]);

  // 맨 아래를 보고 있으면 새 내용을 따라간다. 위로 올려 읽는 중이면 따라가지 않는다.
  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (element && follow.current) {
      element.scrollTop = element.scrollHeight;
    }
  }, [turns, threadId]);

  // 입력창 위에 승인 요청이나 대기 메시지가 생겨 대화 영역이 줄어도 맨 아래를 유지한다.
  useEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    const observer = new ResizeObserver(() => {
      if (follow.current) {
        element.scrollTop = element.scrollHeight;
      }
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  /** 사용자가 직접 스크롤한 때이다. 영역 크기가 바뀌어 생긴 스크롤 이벤트는 따라가기를 끄지 않는다. */
  const userScrollAt = useRef(0);
  const markUserScroll = () => {
    userScrollAt.current = Date.now();
  };

  const onScroll = () => {
    const element = scrollRef.current;
    if (!element) return;
    const nearBottom = element.scrollHeight - element.scrollTop - element.clientHeight < BOTTOM_THRESHOLD_PX;
    const byUser = Date.now() - userScrollAt.current < USER_SCROLL_WINDOW_MS;
    if (nearBottom || byUser) {
      follow.current = nearBottom;
      setAtBottom(nearBottom);
    }
  };

  return (
    <div className="conversation-wrap">
      <div className="conversation" ref={scrollRef} onScroll={onScroll} onWheel={markUserScroll} onPointerDown={markUserScroll} onKeyDown={markUserScroll} onTouchMove={markUserScroll}>
        <div className="column">
          {turns.map((turn) => (
            <Turn key={turn.id} turn={turn} threadId={threadId} />
          ))}
        </div>
      </div>
      {!atBottom && (
        <button
          type="button"
          className="jump-bottom"
          title="맨 아래로"
          onClick={() => {
            follow.current = true;
            scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
          }}
        >
          <Icon name="chevronDown" />
        </button>
      )}
    </div>
  );
}
