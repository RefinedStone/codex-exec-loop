// 열린 스레드의 대화 기록이다. 긴 대화는 보이는 턴만 그린다 (P3 §7, 가상 스크롤).
// 도구 호출은 한 줄 요약으로 접고, 계획은 체크 목록으로, 변경은 요약 카드로 보여 준다 (R10).
// 사용자가 맨 아래를 보고 있으면 새 내용이 올 때 맨 아래를 따라간다. 위로 올려 읽는 중이면 따라가지 않는다.
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import type { TurnPlanStep } from "../../shared/protocol/generated/v2/TurnPlanStep";
import type { TurnStatus } from "../../shared/protocol/generated/v2/TurnStatus";
import { Icon, type IconName } from "../design/icons";
import { useConversationStore } from "../session/desktopStore";
import { openPanel, useUiStore } from "../session/uiStore";
import { copyText } from "./copyText";
import { diffFileFromChange, totalCounts, turnDiffFiles, type DiffFile } from "./diff";
import { DiffCounts, DiffFileList } from "./DiffView";
import { findTurns, matchRanges } from "./find";
import { ItemView } from "./ItemView";
import { Markdown } from "./Markdown";
import type { TurnView } from "./transcript";
import { activitySummary, formatDuration, type StepItem, type StepKind, stepDetail, stepKind, stepStatus, stepSummary, turnBlocks, type TurnBlock } from "./turnBlocks";

const ESTIMATED_TURN_HEIGHT = 320;
const BOTTOM_THRESHOLD_PX = 64;

const STEP_ICON: Record<StepKind, IconName> = { read: "file", search: "search", edit: "pencil", command: "terminal", tool: "bolt", web: "globe" };
const STEP_VERB: Record<StepKind, string> = { read: "읽음", search: "검색", edit: "수정", command: "실행", tool: "도구", web: "웹 검색" };

const STEP_TEST_ID: Partial<Record<StepItem["type"], string>> = { commandExecution: "item-command", fileChange: "item-file-change" };

/** 도구 실행 카드이다 (#2378). 접힌 채로 시작하고, 펼치면 보존된 출력과 오류를 보인다. 아이템 객체와 턴 상태가 같으면 다시 그리지 않는다. */
const StepRow = memo(function StepRow({ step, turnStatus }: { step: StepItem; turnStatus: TurnStatus }) {
  const status = stepStatus(step, turnStatus);
  const [open, setOpen] = useState(false);
  const kind = stepKind(step);
  const detail = stepDetail(step);
  const hasDetail = step.type === "fileChange" ? step.changes.length > 0 : Boolean(detail.output || detail.error);
  const durationMs = "durationMs" in step ? step.durationMs : null;
  return (
    <li className={`step step-${status}`} data-testid={STEP_TEST_ID[step.type] ?? "item-tool"} data-status={status}>
      <button type="button" className="step-line" onClick={() => setOpen(!open)} disabled={!hasDetail} aria-expanded={open}>
        <Icon name={STEP_ICON[kind]} size={13} />
        <span className="step-verb">{STEP_VERB[kind]}</span>
        <code>{stepSummary(step)}</code>
        {status === "running" && <span className="spinner" />}
        {/* 웹 검색 아이템에는 상태가 없다. 성공 표시를 하지 않는다. */}
        {status === "ok" && step.type !== "webSearch" && <span className="badge badge-ok">성공</span>}
        {status === "failed" && <span className="badge badge-bad">{"status" in step && step.status === "declined" ? "거부" : "실패"}</span>}
        {status === "interrupted" && <span className="badge badge-warn">중단</span>}
        {status === "unknown" && <span className="badge badge-warn">확인 못 함</span>}
        {durationMs !== null && <span className="step-duration">{formatDuration(durationMs)}</span>}
        {hasDetail && <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />}
      </button>
      {open && detail.error && <p className="step-error">{detail.error}</p>}
      {open && detail.output && <pre className="step-output">{detail.output}</pre>}
      {open && step.type === "fileChange" && (
        <>
          <DiffFileList files={step.changes.map(diffFileFromChange)} defaultOpen />
          <button type="button" className="ghost step-review" onClick={() => openPanel("review")}>
            변경 검토 열기
          </button>
        </>
      )}
    </li>
  );
});

/** 진행 중이거나 실패가 있으면 펼친다. 끝난 작업 묶음은 한 줄로 접는다. */
function Activity({ steps, turnStatus }: { steps: StepItem[]; turnStatus: TurnStatus }) {
  const statuses = steps.map((step) => stepStatus(step, turnStatus));
  const live = statuses.includes("running");
  const failed = statuses.filter((status) => status === "failed").length;
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
            <StepRow key={step.id} step={step} turnStatus={turnStatus} />
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
function sameBlock(previous: { block: TurnBlock; turnStatus: TurnStatus }, next: { block: TurnBlock; turnStatus: TurnStatus }): boolean {
  const a = previous.block;
  const b = next.block;
  if (a.type === "activity") {
    return b.type === "activity" && previous.turnStatus === next.turnStatus && a.steps.length === b.steps.length && a.steps.every((step, index) => step === b.steps[index]);
  }
  if (a.type === "other") {
    return b.type === "other" && a.item === b.item;
  }
  return a.type === b.type && a.id === b.id && a.text === b.text;
}

/** 메시지 하나를 복사하는 단추이다 (plan 26 R2). 메시지에 마우스를 올리면 보인다. */
function MessageCopy({ text }: { text: string }) {
  return (
    <button type="button" className="ghost message-copy" onClick={() => copyText(text, "메시지를 복사했습니다")}>
      복사
    </button>
  );
}

const Block = memo(function Block({ block, turnStatus }: { block: TurnBlock; turnStatus: TurnStatus }) {
  switch (block.type) {
    case "user":
      return (
        <div className="user-message" data-testid="item-user">
          <p>{block.text}</p>
          <MessageCopy text={block.text} />
        </div>
      );
    case "agent":
      return (
        <div className="agent-message" data-testid="item-agent">
          <Markdown text={block.text} />
          <MessageCopy text={block.text} />
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
      return <Activity steps={block.steps} turnStatus={turnStatus} />;
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
  copyText(turn.items.flatMap((item) => (item.type === "agentMessage" ? [item.text] : [])).join("\n\n"), "답을 복사했습니다");
}

const Turn = memo(function Turn({ turn }: { turn: TurnView }) {
  const blocks = useMemo(() => turnBlocks(turn.items), [turn.items]);
  const changedFiles = useMemo(() => turnDiffFiles(turn.diff, turn.items), [turn.diff, turn.items]);
  return (
    <article className="turn" data-testid="turn" data-status={turn.status}>
      {blocks.map((block) => (
        <Block key={block.id} block={block} turnStatus={turn.status} />
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

const FIND_ALL = "akra-find";
const FIND_CURRENT = "akra-find-current";

/** `root` 안의 사용자 글과 에이전트 답에서 `needle`(소문자)이 있는 자리이다. 단추의 글은 뺀다. */
function findRanges(root: HTMLElement, needle: string): Range[] {
  const ranges: Range[] = [];
  for (const message of root.querySelectorAll(".user-message, .agent-message")) {
    const walker = document.createTreeWalker(message, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (node.parentElement?.closest("button")) continue;
      for (const { start, end } of matchRanges(node.nodeValue ?? "", needle)) {
        const range = document.createRange();
        range.setStart(node, start);
        range.setEnd(node, end);
        ranges.push(range);
      }
    }
  }
  return ranges;
}

/** 스레드마다 새로 만든다. 스크롤 자리와 "이전 턴" 읽기의 상태(읽는 중, 오류)가 다른 스레드로 넘어가지 않는다. */
export function ConversationView({ onLoadOlder }: { onLoadOlder(): Promise<void> }) {
  const openThreadId = useConversationStore((state) => state.openThreadId);
  if (!openThreadId) {
    return <p className="muted empty">왼쪽에서 스레드를 고르세요.</p>;
  }
  return <ThreadConversation key={openThreadId} onLoadOlder={onLoadOlder} />;
}

function ThreadConversation({ onLoadOlder }: { onLoadOlder(): Promise<void> }) {
  const transcript = useConversationStore((state) => state.transcript);
  const transcriptError = useConversationStore((state) => state.transcriptError);
  const scrollRef = useRef<HTMLElement>(null);
  // 스레드를 새로 열면 맨 아래(최근 턴)부터 보여 준다.
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

  // 대화 안 찾기이다 (plan 26 R1). 기록에서 찾은 턴으로 스크롤하고, 화면에 그린 턴 안의 글에 표시한다.
  const findOpen = useUiStore((state) => state.findOpen);
  const [query, setQuery] = useState("");
  const [findAt, setFindAt] = useState(0);
  const matches = useMemo(() => (findOpen ? findTurns(turns, query) : []), [findOpen, turns, query]);
  const matchAt = Math.min(findAt, Math.max(0, matches.length - 1));
  const currentTurn = matches[matchAt];
  /** 찾은 턴으로 옮긴 뒤, 그 턴 안의 첫 표시가 화면 밖이면 한 번 보이게 한다. */
  const revealMatch = useRef(false);

  const goToMatch = (position: number) => {
    if (matches.length === 0) return;
    const next = (position + matches.length) % matches.length;
    const index = matches[next];
    if (index === undefined) return;
    setFindAt(next);
    followBottom.current = false;
    revealMatch.current = true;
    virtualizer.scrollToIndex(index, { align: "start" });
  };
  const goToMatchRef = useRef(goToMatch);
  goToMatchRef.current = goToMatch;
  // 찾는 글을 바꾸면 처음 찾은 턴으로 간다.
  useEffect(() => {
    if (findOpen) goToMatchRef.current(0);
  }, [findOpen, query]);

  // 그릴 때마다 표시를 다시 만든다: 스크롤하면 그려진 턴이 바뀐다. 찾기 막대가 열려 있을 때만 일한다.
  useLayoutEffect(() => {
    if (typeof CSS === "undefined" || !("highlights" in CSS)) return;
    const needle = query.trim().toLowerCase();
    const root = scrollRef.current;
    if (!findOpen || needle.length === 0 || !root) {
      CSS.highlights.delete(FIND_ALL);
      CSS.highlights.delete(FIND_CURRENT);
      return;
    }
    const others: Range[] = [];
    const current: Range[] = [];
    for (const row of root.querySelectorAll<HTMLElement>(".turn-row")) {
      (Number(row.dataset.index) === currentTurn ? current : others).push(...findRanges(row, needle));
    }
    CSS.highlights.set(FIND_ALL, new Highlight(...others));
    CSS.highlights.set(FIND_CURRENT, new Highlight(...current));
    const first = current[0];
    if (revealMatch.current && first) {
      revealMatch.current = false;
      const rect = first.getBoundingClientRect();
      const view = root.getBoundingClientRect();
      if (rect.top < view.top || rect.bottom > view.bottom) first.startContainer.parentElement?.scrollIntoView({ block: "center" });
    }
  });
  useEffect(
    () => () => {
      if (typeof CSS !== "undefined" && "highlights" in CSS) {
        CSS.highlights.delete(FIND_ALL);
        CSS.highlights.delete(FIND_CURRENT);
      }
    },
    [],
  );

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
      {findOpen && (
        <div className="find-bar" role="search" data-testid="find-bar">
          <input
            autoFocus
            data-find-input
            value={query}
            placeholder="대화에서 찾기"
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                event.preventDefault();
                goToMatch(matchAt + (event.shiftKey ? -1 : 1));
              }
            }}
            data-testid="find-input"
          />
          <span className="find-count" data-testid="find-count">
            {matches.length > 0 ? `턴 ${matchAt + 1}/${matches.length}` : query.trim() ? "없음" : ""}
          </span>
          <button type="button" className="ghost" title="이전 (⇧⏎)" onClick={() => goToMatch(matchAt - 1)} disabled={matches.length === 0}>
            이전
          </button>
          <button type="button" className="ghost" title="다음 (⏎)" onClick={() => goToMatch(matchAt + 1)} disabled={matches.length === 0}>
            다음
          </button>
          <button type="button" className="icon-button" title="닫기 (esc)" onClick={() => useUiStore.setState({ findOpen: false })}>
            <Icon name="close" />
          </button>
        </div>
      )}
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
