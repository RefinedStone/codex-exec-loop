// 열린 스레드의 대화 기록이다. 긴 대화는 보이는 턴만 그린다 (P3 §7, 가상 스크롤).
// 도구 호출은 한 줄 요약으로 접고, 계획은 체크 목록으로, 변경은 요약 카드로 보여 준다 (R10).
// 사용자가 맨 아래를 보고 있으면 새 내용이 올 때 맨 아래를 따라간다. 위로 올려 읽는 중이면 따라가지 않는다.
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import type { TurnPlanStep } from "../../shared/protocol/generated/v2/TurnPlanStep";
import { Icon, type IconName } from "../design/icons";
import { useConversationStore } from "../session/desktopStore";
import { openPanel, toast } from "../session/uiStore";
import { diffFileFromChange, totalCounts, turnDiffFiles, type DiffFile } from "./diff";
import { DiffCounts, DiffFileList } from "./DiffView";
import { ItemView } from "./ItemView";
import { Markdown } from "./Markdown";
import type { TurnView } from "./transcript";
import { activitySummary, formatDuration, type StepItem, type StepKind, stepKind, stepLabel, stepStatus, turnBlocks, type TurnBlock } from "./turnBlocks";

const ESTIMATED_TURN_HEIGHT = 320;
const BOTTOM_THRESHOLD_PX = 64;

const STEP_ICON: Record<StepKind, IconName> = { read: "file", search: "search", edit: "pencil", command: "terminal", tool: "bolt" };
const STEP_VERB: Record<StepKind, string> = { read: "읽음", search: "검색", edit: "수정", command: "실행", tool: "도구" };

/** 아이템 객체가 같으면 다시 그리지 않는다. */
const StepRow = memo(function StepRow({ step }: { step: StepItem }) {
  const status = stepStatus(step);
  const [open, setOpen] = useState(status === "failed");
  const kind = stepKind(step);
  const output = step.type === "commandExecution" ? step.aggregatedOutput : null;
  const hasDetail = step.type === "fileChange" ? step.changes.length > 0 : Boolean(output);
  const durationMs = step.type === "fileChange" ? null : step.durationMs;
  return (
    <li className={`step step-${status}`} data-testid={step.type === "commandExecution" ? "item-command" : step.type === "fileChange" ? "item-file-change" : undefined}>
      <button type="button" className="step-line" onClick={() => setOpen(!open)} disabled={!hasDetail} aria-expanded={open}>
        <Icon name={STEP_ICON[kind]} size={13} />
        <span className="step-verb">{STEP_VERB[kind]}</span>
        <code>{stepLabel(step)}</code>
        {status === "running" && <span className="spinner" />}
        {status === "failed" && <span className="badge badge-bad">{step.status === "declined" ? "거부" : "실패"}</span>}
        {durationMs !== null && <span className="step-duration">{formatDuration(durationMs)}</span>}
        {hasDetail && <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />}
      </button>
      {open && output && <pre className="step-output">{output}</pre>}
      {open && step.type === "fileChange" && <DiffFileList files={step.changes.map(diffFileFromChange)} defaultOpen />}
    </li>
  );
});

/** 진행 중이거나 실패가 있으면 펼친다. 끝난 작업 묶음은 한 줄로 접는다. */
function Activity({ steps }: { steps: StepItem[] }) {
  const live = steps.some((step) => stepStatus(step) === "running");
  const failed = steps.filter((step) => stepStatus(step) === "failed").length;
  const [open, setOpen] = useState<boolean | null>(null);
  const shown = open ?? (live || failed > 0);
  return (
    <div className="activity" data-testid="item-activity">
      <button type="button" className="activity-summary" onClick={() => setOpen(!shown)} aria-expanded={shown}>
        <Icon name={shown ? "chevronDown" : "chevronRight"} size={12} />
        <span>{activitySummary(steps)}</span>
        {failed > 0 && <span className="badge badge-bad">실패 {failed}</span>}
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

const PLAN_STATE: Record<TurnPlanStep["status"], string> = { completed: "done", inProgress: "active", pending: "todo" };

export function PlanList({ steps }: { steps: readonly TurnPlanStep[] }) {
  return (
    <ol className="plan-steps">
      {steps.map((step, index) => (
        <li key={index} className={`plan-step plan-${PLAN_STATE[step.status]}`}>
          <span className="plan-mark">{step.status === "completed" && <Icon name="check" size={11} />}</span>
          {step.step}
        </li>
      ))}
    </ol>
  );
}

function Changes({ files }: { files: readonly DiffFile[] }) {
  const counts = totalCounts(files);
  return (
    <div className="changes-card" data-testid="turn-changes">
      <header>
        <Icon name="diff" />
        <span>파일 {files.length}개 변경</span>
        <DiffCounts added={counts.added} removed={counts.removed} />
        <button type="button" className="button" onClick={() => openPanel("review")}>
          변경 검토
        </button>
      </header>
      <DiffFileList files={files} />
    </div>
  );
}

/** `turnBlocks` 는 덩어리 객체를 매번 새로 만든다. 그래서 내용(원본 아이템과 글)으로 비교한다. */
function sameBlock(previous: { block: TurnBlock }, next: { block: TurnBlock }): boolean {
  const a = previous.block;
  const b = next.block;
  if (a.type === "activity") {
    return b.type === "activity" && a.steps.length === b.steps.length && a.steps.every((step, index) => step === b.steps[index]);
  }
  if (a.type === "other") {
    return b.type === "other" && a.item === b.item;
  }
  return a.type === b.type && a.id === b.id && a.text === b.text;
}

const Block = memo(function Block({ block }: { block: TurnBlock }) {
  switch (block.type) {
    case "user":
      return (
        <div className="user-message" data-testid="item-user">
          <p>{block.text}</p>
        </div>
      );
    case "agent":
      return (
        <div className="agent-message" data-testid="item-agent">
          <Markdown text={block.text} />
        </div>
      );
    case "reasoning":
      return (
        <details className="reasoning">
          <summary>
            <Icon name="sparkle" size={13} />
            생각
          </summary>
          <p>{block.text}</p>
        </details>
      );
    case "activity":
      return <Activity steps={block.steps} />;
    case "other":
      return <ItemView item={block.item} />;
  }
}, sameBlock);

function useElapsed(startedAtSeconds: number | null): string | null {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  return startedAtSeconds === null ? null : formatDuration(Math.max(0, Math.round(now / 1000 - startedAtSeconds)) * 1000);
}

function workingLabel(turn: TurnView, waitingForApproval: boolean): string {
  if (waitingForApproval) return "승인을 기다리는 중";
  const last = turn.items.at(-1);
  if (last?.type === "commandExecution" && last.status === "inProgress") return "명령을 실행하는 중";
  if (last?.type === "agentMessage") return "답을 쓰는 중";
  return "생각하는 중";
}

function Working({ turn }: { turn: TurnView }) {
  const waiting = useConversationStore((state) => state.pendingApprovals.some((approval) => approval.turnId === turn.id));
  const elapsed = useElapsed(turn.startedAt);
  return (
    <div className={waiting ? "working waiting" : "working"}>
      <span className="working-dot" />
      <span className="shimmer">{workingLabel(turn, waiting)}</span>
      {elapsed && <span className="working-elapsed">{elapsed}</span>}
    </div>
  );
}

function copyAnswer(turn: TurnView): void {
  const text = turn.items.flatMap((item) => (item.type === "agentMessage" ? [item.text] : [])).join("\n\n");
  navigator.clipboard.writeText(text).then(
    () => toast("답을 복사했습니다"),
    () => toast("복사하지 못했습니다"),
  );
}

const Turn = memo(function Turn({ turn }: { turn: TurnView }) {
  const blocks = useMemo(() => turnBlocks(turn.items), [turn.items]);
  const changedFiles = useMemo(() => turnDiffFiles(turn.diff, turn.items), [turn.diff, turn.items]);
  return (
    <article className="turn" data-testid="turn" data-status={turn.status}>
      {blocks.map((block) => (
        <Block key={block.id} block={block} />
      ))}
      {turn.plan && turn.plan.length > 0 && (
        <div className="plan-card">
          <header>
            <Icon name="check" size={13} />
            계획 · {turn.plan.filter((step) => step.status === "completed").length}/{turn.plan.length}
          </header>
          <PlanList steps={turn.plan} />
        </div>
      )}
      {changedFiles.length > 0 && <Changes files={changedFiles} />}
      {turn.status === "inProgress" && <Working turn={turn} />}
      {turn.status === "interrupted" && <div className="turn-note">중단했습니다</div>}
      {turn.status === "failed" && <div className="turn-note">실패했습니다</div>}
      {turn.errorMessage && <div className="turn-note">{turn.errorMessage}</div>}
      {turn.status === "completed" && (
        <footer className="turn-footer">
          {turn.durationMs !== null && <span>{formatDuration(turn.durationMs)}</span>}
          <button type="button" className="ghost" onClick={() => copyAnswer(turn)}>
            복사
          </button>
        </footer>
      )}
    </article>
  );
});

export function ConversationView({ onLoadOlder }: { onLoadOlder(): Promise<void> }) {
  const openThreadId = useConversationStore((state) => state.openThreadId);
  const transcript = useConversationStore((state) => state.transcript);
  const transcriptError = useConversationStore((state) => state.transcriptError);
  const scrollRef = useRef<HTMLElement>(null);
  const followBottom = useRef(true);
  const [atBottom, setAtBottom] = useState(true);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [olderError, setOlderError] = useState<string | null>(null);
  /** 이전 턴을 붙이기 전의 첫 턴 id 이다. 붙인 뒤 그 턴이 보던 자리에 오도록 스크롤한다. */
  const anchorTurnId = useRef<string | null>(null);
  const turns = transcript?.turns ?? [];

  const virtualizer = useVirtualizer({
    count: turns.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ESTIMATED_TURN_HEIGHT,
    getItemKey: (index) => turns[index]?.id ?? index,
    overscan: 3,
  });

  // 스레드를 새로 열면 맨 아래(최근 턴)부터 보여 준다.
  useEffect(() => {
    followBottom.current = true;
    setAtBottom(true);
    // 이전 스레드에서 걸어 둔 "이전 턴" 앵커는 새 스레드에 없으므로 버린다.
    anchorTurnId.current = null;
  }, [openThreadId]);

  // 내용이 바뀔 때마다, 맨 아래를 보고 있었으면 맨 아래로 간다. 이전 턴을 붙였으면 보던 턴으로 돌아간다.
  useLayoutEffect(() => {
    const anchor = anchorTurnId.current;
    if (anchor) {
      const index = turns.findIndex((turn) => turn.id === anchor);
      if (index > 0) {
        anchorTurnId.current = null;
        virtualizer.scrollToIndex(index, { align: "start" });
      }
      return;
    }
    if (followBottom.current && turns.length > 0) {
      virtualizer.scrollToIndex(turns.length - 1, { align: "end" });
    }
  }, [transcript, turns, virtualizer]);

  const loadOlder = async () => {
    setOlderError(null);
    setLoadingOlder(true);
    followBottom.current = false;
    anchorTurnId.current = turns[0]?.id ?? null;
    try {
      await onLoadOlder();
    } catch (error) {
      anchorTurnId.current = null;
      setOlderError(error instanceof Error ? error.message : String(error));
    } finally {
      setLoadingOlder(false);
    }
  };

  if (!openThreadId) {
    return <p className="muted empty">왼쪽에서 스레드를 고르세요.</p>;
  }
  if (transcriptError) {
    return <p className="error empty">{transcriptError}</p>;
  }
  if (!transcript) {
    return <p className="muted empty">불러오는 중…</p>;
  }

  const onScroll = () => {
    const element = scrollRef.current;
    if (element) {
      followBottom.current = element.scrollHeight - element.scrollTop - element.clientHeight < BOTTOM_THRESHOLD_PX;
      setAtBottom(followBottom.current);
    }
  };

  return (
    <div className="conversation-wrap">
      <section className="conversation" data-testid="conversation" ref={scrollRef} onScroll={onScroll}>
        <div className="column">
          {transcript.olderTurnsCursor && (
            <div className="load-older">
              <button type="button" className="button" onClick={() => void loadOlder()} disabled={loadingOlder} data-testid="load-older">
                {loadingOlder ? "불러오는 중…" : "이전 턴 더 보기"}
              </button>
              {olderError && <span className="error">{olderError}</span>}
            </div>
          )}
          {turns.length === 0 && <p className="muted">아직 메시지가 없습니다.</p>}
          <div className="turn-list" style={{ height: virtualizer.getTotalSize() }}>
            {virtualizer.getVirtualItems().map((row) => {
              const turn = turns[row.index];
              return turn ? (
                <div key={row.key} data-index={row.index} ref={virtualizer.measureElement} className="turn-row" style={{ transform: `translateY(${row.start}px)` }}>
                  <Turn turn={turn} />
                </div>
              ) : null;
            })}
          </div>
        </div>
      </section>
      {!atBottom && (
        <button
          type="button"
          className="jump-bottom"
          title="맨 아래로"
          onClick={() => {
            followBottom.current = true;
            setAtBottom(true);
            virtualizer.scrollToIndex(turns.length - 1, { align: "end" });
          }}
        >
          <Icon name="chevronDown" />
        </button>
      )}
    </div>
  );
}
