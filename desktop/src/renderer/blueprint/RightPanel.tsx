// 오른쪽 패널이다. 탭으로 변경 검토, Akra 커널, 브라우저, 업무, 문서를 바꾼다. 왼쪽 경계를 끌어 너비를 바꾼다.
import { useEffect, useState } from "react";

import type { DiffFile } from "../conversation/diff";
import { Icon, type IconName } from "../design/icons";
import { DOCUMENTS, KERNEL, WORK_ITEMS, type MockThread } from "./mockData";
import { addReviewNote, handOffToNewThread, notConnected, openThread, REVIEW_FILES, runDelivery, toast, toggleViewed, useBlueprint, useDelivery, type PanelId } from "./store";

export const PANELS: { id: PanelId; label: string; icon: IconName }[] = [
  { id: "review", label: "변경", icon: "diff" },
  { id: "kernel", label: "Akra", icon: "bolt" },
  { id: "browser", label: "브라우저", icon: "globe" },
  { id: "work", label: "업무", icon: "briefcase" },
  { id: "docs", label: "문서", icon: "book" },
];

const MARKER = { add: "+", del: "−", context: " ", hunk: "", meta: "" } as const;

function ReviewDiff({ file }: { file: DiffFile }) {
  const notes = useBlueprint((state) => state.reviewNotes);
  const [commentLine, setCommentLine] = useState<number | null>(null);
  const [text, setText] = useState("");
  const save = (line: number) => {
    if (text.trim().length > 0) {
      addReviewNote({ path: file.path, line, text: text.trim() });
    }
    setText("");
    setCommentLine(null);
  };
  return (
    <div className="diff-lines">
      {file.lines.map((line, index) => (
        <div key={index}>
          <div className={`diff-line diff-${line.kind}`}>
            {line.kind !== "hunk" && line.kind !== "meta" && (
              <button type="button" className="diff-comment-button" title="이 줄에 메모" onClick={() => setCommentLine(index)}>
                <Icon name="plus" size={11} />
              </button>
            )}
            <span className="diff-marker">{MARKER[line.kind]}</span>
            {line.text}
          </div>
          {notes
            .filter((note) => note.path === file.path && note.line === index)
            .map((note) => (
              <div key={note.text} className="diff-note">
                <Icon name="message" size={12} />
                {note.text}
              </div>
            ))}
          {commentLine === index && (
            <form
              className="diff-note-form"
              onSubmit={(event) => {
                event.preventDefault();
                save(index);
              }}
            >
              <input autoFocus value={text} onChange={(event) => setText(event.target.value)} placeholder="에이전트에게 보낼 메모" onKeyDown={(event) => event.key === "Escape" && setCommentLine(null)} />
              <button type="submit" className="button primary">
                메모 추가
              </button>
            </form>
          )}
        </div>
      ))}
    </div>
  );
}

/**
 * 전달 띠이다: 변경에서 머지까지의 단계와 다음 행동 1 개. 단계는 안내이고 뒤 단계를 막지 않는다.
 * `onReview` 는 검토 단계의 행동이다 (아직 보지 않은 파일을 연다).
 */
function DeliveryStrip({ thread, onReview }: { thread: MockThread; onReview(): void }) {
  const delivery = useDelivery(thread);
  if (!delivery) return null;
  const next = delivery.next;
  // 추천은 안내이다. 검토·검증을 끝내지 않아도 다른 전달 작업을 고를 수 있다.
  const actions = [
    { id: "verify", label: "검증 맡기기", available: !!thread.criteria && thread.criteria.passed < thread.criteria.total },
    { id: "commit", label: "커밋", available: !thread.committed },
    { id: "push", label: "push", available: !thread.pushed },
    { id: "pr", label: "PR 만들기", available: !thread.pullRequest },
    { id: "merge", label: "GitHub 에서 열기", available: thread.pullRequest?.checks === "pass" && !thread.pullRequest.merged },
  ] as const;
  const otherActions = actions.filter((action) => next !== null && action.available && action.id !== next.id);
  return (
    <div className="delivery">
      <ol className="delivery-steps" aria-label="전달 단계">
        {delivery.steps.map((step) => (
          <li key={step.id} className={`delivery-${step.state}`}>
            {step.state === "done" && <Icon name="check" size={11} />}
            {step.label}
          </li>
        ))}
      </ol>
      <div className="delivery-next">
        {next ? (
          <>
            <span>
              <strong>다음: {next.label}</strong> <span className="muted">{delivery.hint}</span>
            </span>
            {next.id === "review" ? (
              <button type="button" className="button primary" onClick={onReview}>
                안 본 파일 열기
              </button>
            ) : (
              delivery.action && (
                <button type="button" className="button primary" onClick={() => runDelivery(thread.id, next.id)}>
                  {delivery.action}
                </button>
              )
            )}
          </>
        ) : (
          <span className="muted">머지까지 끝났습니다.</span>
        )}
      </div>
      {otherActions.length > 0 && (
        <details>
          <summary className="ghost">다른 전달 작업</summary>
          <div className="offer-buttons">
            {otherActions.map((action) => (
              <button key={action.id} type="button" className="button" onClick={() => runDelivery(thread.id, action.id)}>
                {action.label}
              </button>
            ))}
          </div>
        </details>
      )}
    </div>
  );
}

function ReviewPanel() {
  const files = REVIEW_FILES;
  const [selected, setSelected] = useState(0);
  const noteCount = useBlueprint((state) => state.reviewNotes.length);
  const thread = useBlueprint((state) => state.threads.find((item) => item.id === state.openThreadId));
  const viewed = useBlueprint((state) => (thread ? state.viewed[thread.id] : undefined)) ?? [];
  const reviewPath = useBlueprint((state) => state.reviewPath);
  const file = files[selected];

  // 대화의 변경 카드에서 파일을 누르면 그 파일을 보인다.
  useEffect(() => {
    if (reviewPath === null) return;
    const index = files.findIndex((item) => item.path === reviewPath);
    if (index >= 0) setSelected(index);
    useBlueprint.setState({ reviewPath: null });
  }, [files, reviewPath]);

  const openUnviewed = () => {
    const index = files.findIndex((item) => !viewed.includes(item.path));
    if (index >= 0) setSelected(index);
  };

  return (
    <div className="review">
      {thread && <DeliveryStrip thread={thread} onReview={openUnviewed} />}
      <ul className="review-files">
        {files.map((item, index) => {
          const seen = viewed.includes(item.path);
          return (
            <li key={item.path} className={seen ? "viewed" : undefined}>
              <button type="button" className={index === selected ? "selected" : undefined} onClick={() => setSelected(index)}>
                <Icon name="file" size={13} />
                <span className="review-file-name">{item.path.split("/").at(-1)}</span>
                <span className="review-file-dir">{item.path.split("/").slice(0, -1).join("/")}</span>
                <span className="counts">
                  <span className="count-add">+{item.added}</span> <span className="count-del">−{item.removed}</span>
                </span>
              </button>
              {thread && (
                <button type="button" className="viewed-toggle" aria-pressed={seen} title={seen ? "봄 표시 지우기" : "본 파일로 표시"} onClick={() => toggleViewed(thread.id, item.path)}>
                  <Icon name={seen ? "check" : "eye"} size={13} />
                </button>
              )}
            </li>
          );
        })}
      </ul>
      {file && <ReviewDiff key={file.path} file={file} />}
      {noteCount > 0 && <div className="review-footer">메모 {noteCount}개는 다음 메시지와 함께 에이전트에게 갑니다.</div>}
    </div>
  );
}

function KernelPanel() {
  const threads = useBlueprint((state) => state.threads);
  const [dismissed, setDismissed] = useState<string[]>([]);
  const offers = KERNEL.offers.filter((offer) => !dismissed.includes(offer.id));
  return (
    <div className="kernel">
      <ol className="stages">
        {KERNEL.stages.map((stage, index) => (
          <li key={stage} className={index < KERNEL.activeStage ? "done" : index === KERNEL.activeStage ? "active" : undefined}>
            <span className="stage-mark">{index < KERNEL.activeStage ? <Icon name="check" size={11} /> : index + 1}</span>
            {stage}
          </li>
        ))}
      </ol>
      <p className="kernel-decision">{KERNEL.decision}</p>

      <h4>제안</h4>
      {offers.length === 0 && <p className="muted">지금은 제안이 없습니다.</p>}
      {offers.map((offer) => (
        <div key={offer.id} className="offer">
          <strong>{offer.title}</strong>
          <p>{offer.detail}</p>
          <div className="offer-buttons">
            <button
              type="button"
              className="button primary"
              onClick={() => {
                setDismissed([...dismissed, offer.id]);
                toast("제안을 수락했습니다 (mock)");
              }}
            >
              수락
            </button>
            <button type="button" className="button" onClick={() => setDismissed([...dismissed, offer.id])}>
              무시
            </button>
          </div>
        </div>
      ))}

      <h4>병렬 슬롯</h4>
      <ul className="slots">
        {KERNEL.slots.map((slot) => {
          const thread = threads.find((item) => item.id === slot.threadId);
          return (
            <li key={slot.name}>
              <code>{slot.name}</code>
              <span className="slot-state">{slot.state}</span>
              {thread ? (
                <button type="button" className="ghost" onClick={() => openThread(thread.id)}>
                  {thread.title}
                </button>
              ) : (
                <span className="muted">—</span>
              )}
            </li>
          );
        })}
      </ul>

      <h4>커널 기록</h4>
      <ul className="records">
        {KERNEL.records.map((record) => (
          <li key={record.seq}>
            <code>{record.seq}</code>
            {record.text}
          </li>
        ))}
      </ul>
    </div>
  );
}

function BrowserPanel() {
  return (
    <div className="browser">
      <div className="browser-bar">
        <span className="chip">프로필: 기본</span>
        <input readOnly value="http://localhost:5173/" aria-label="주소" />
        <button type="button" className="button" onClick={() => notConnected("요소 메모")}>
          요소 선택
        </button>
      </div>
      <div className="browser-page">
        <div className="skeleton wide" />
        <div className="skeleton" />
        <div className="skeleton short" />
        <p className="muted">브라우저 화면 자리입니다 (mock). 실제 앱에서는 프로필별 세션으로 사이트를 엽니다.</p>
      </div>
    </div>
  );
}

type WorkSource = keyof typeof WORK_ITEMS;
const WORK_SOURCES = Object.keys(WORK_ITEMS) as WorkSource[];

function WorkPanel() {
  const [source, setSource] = useState<WorkSource>("GitHub");
  return (
    <div className="work">
      <div className="segmented">
        {WORK_SOURCES.map((name) => (
          <button key={name} type="button" className={source === name ? "selected" : undefined} onClick={() => setSource(name)}>
            {name}
          </button>
        ))}
      </div>
      <ul className="work-items">
        {WORK_ITEMS[source].map((item) => (
          <li key={item.title}>
            <span className="work-key">{item.key}</span>
            <span className="work-text">
              <strong>{item.title}</strong>
              <span className="muted">{item.meta}</span>
            </span>
            <button type="button" className="button" onClick={() => handOffToNewThread(`${source} ${item.key} "${item.title}" 을 처리해줘.`)}>
              스레드로 넘기기
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function DocsPanel() {
  return (
    <ul className="work-items">
      {DOCUMENTS.map((document) => (
        <li key={document.title}>
          <Icon name="book" />
          <span className="work-text">
            <strong>{document.title}</strong>
            <span className="muted">{document.kind}</span>
          </span>
          <button type="button" className="button" onClick={() => notConnected("문서 열기")}>
            열기
          </button>
        </li>
      ))}
    </ul>
  );
}

const MIN_WIDTH = 360;
const MAX_WIDTH = 900;

export function RightPanel({ panel }: { panel: PanelId }) {
  const [width, setWidth] = useState(520);
  const startResize = (event: React.PointerEvent<HTMLDivElement>) => {
    const startX = event.clientX;
    const startWidth = width;
    const onMove = (move: PointerEvent) => setWidth(Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, startWidth + startX - move.clientX)));
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };
  return (
    <aside className="right-panel" style={{ width }}>
      <div className="resize-handle" onPointerDown={startResize} />
      <header className="panel-tabs">
        {PANELS.map((item) => (
          <button key={item.id} type="button" className={item.id === panel ? "selected" : undefined} onClick={() => useBlueprint.setState({ panel: item.id })}>
            <Icon name={item.icon} size={14} />
            {item.label}
          </button>
        ))}
        <span className="bar-spacer" />
        <button type="button" className="icon-button" title="패널 닫기" onClick={() => useBlueprint.setState({ panel: null })}>
          <Icon name="close" />
        </button>
      </header>
      <div className="panel-body">
        {panel === "review" && <ReviewPanel />}
        {panel === "kernel" && <KernelPanel />}
        {panel === "browser" && <BrowserPanel />}
        {panel === "work" && <WorkPanel />}
        {panel === "docs" && <DocsPanel />}
      </div>
    </aside>
  );
}
