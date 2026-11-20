// 민감 이슈 초안의 공용 표시이다. 원문은 호출한 패널의 메모리 상태만 사용한다.
import { useRef, useState } from "react";
import {
  addSpan,
  validationErrorText,
  type IssueDraftField,
  type IssueDraftOpened,
  type IssueDraftPreview,
  type IssueDraftSpan,
  type IssueSpanAction,
  type IssueSpanClass,
} from "./issueDraft";

const CLASS_LABEL: Record<IssueSpanClass, string> = { secret: "비밀값", name: "이름", path: "경로", log: "로그" };
const ACTION_LABEL: Record<IssueSpanAction, string> = { remove: "제거", restorable_mask: "표식으로 가림", user_confirm: "사용자가 정함" };
const FIELD_LABEL: Record<IssueDraftField, string> = { title: "제목", body: "본문" };
const TEMPLATE_PROBLEM: Record<string, string> = {
  missing: "정책이 가리킨 템플릿 파일이 없습니다.",
  unreadable: "템플릿 파일을 읽지 못했습니다.",
  invalid: "템플릿 파일을 해석하지 못했습니다.",
};
export function IssueDraftRequirements({ opened }: { opened: Pick<IssueDraftOpened, "template" | "requiredSections"> }) {
  const { template, requiredSections } = opened;
  const problem = TEMPLATE_PROBLEM[template.status];
  return (
    <section className="work-section" data-testid="issue-draft-requirements">
      <h3>요구 사항</h3>
      {template.status === "recovery_only" && <p className="muted" data-testid="issue-draft-recovery-only">
        기록된 생성 요청의 결과를 확인하는 화면입니다. 새 게시에는 새 초안의 규약을 다시 검사합니다.
      </p>}
      {problem && (
        <div className="error" data-testid="issue-draft-template-problem">
          {problem} 검증할 수 없어서 게시를 막습니다. {template.path} {template.reason}
        </div>
      )}
      {template.status === "not_configured" && requiredSections.length === 0 && <p className="muted">이 프로젝트는 템플릿과 필수 섹션이 없습니다. 제목만 검사합니다.</p>}
      <ul>
        {template.requiredInputs.map((input, index) => (
          <li key={`input-${index}`}>
            템플릿 필드 "{input.label}"{input.options.length > 0 ? ` (${input.options.join(", ")})` : ""}
          </li>
        ))}
        {requiredSections.map((title) => (
          <li key={`section-${title}`}>필수 섹션 "{title}"</li>
        ))}
        {template.unsupportedInputs.map((input, index) => (
          <li key={`unsupported-${index}`} className="issue-draft-warning">
            직접 확인: "{input.label}" ({input.reason})
          </li>
        ))}
      </ul>
    </section>
  );
}

export function IssueDraftPreviewView({ preview, showMarkers = true }: { preview: IssueDraftPreview | null; showMarkers?: boolean }) {
  if (!preview) return <p className="muted work-section">미리보기를 만드는 중…</p>;
  if (preview.status === "needs_confirmation") {
    return (
      <p className="error work-section" data-testid="issue-draft-needs-decision">
        처리를 정하지 않은 민감 구간이 있습니다. 아래 구간에서 "제거" 또는 "표식으로 가림"을 고르세요.
      </p>
    );
  }
  if (preview.status === "invalid_span") {
    return <p className="error work-section">민감 구간 {preview.spanIndex + 1}의 범위가 맞지 않습니다. 그 구간을 지우고 다시 지정하세요.</p>;
  }
  return (
    <section className="work-section" data-testid="issue-draft-preview">
      <h3>게시될 공유본</h3>
      <div className="issue-draft-shared" data-testid="issue-draft-shared-title">
        {preview.sharedCopy.title}
      </div>
      <pre className="issue-draft-shared" data-testid="issue-draft-shared-body">
        {preview.sharedCopy.body}
      </pre>
      {showMarkers && <>
        <h3>남는 가림 표식 ({preview.markers.length})</h3>
        {preview.markers.length === 0 ? (
          <p className="muted">표식이 없습니다.</p>
        ) : (
          <ul data-testid="issue-draft-markers">
            {preview.markers.map((marker) => (
              <li key={marker.marker}>
                <code>{marker.marker}</code> <span className="muted">{FIELD_LABEL[marker.field]}</span>
              </li>
            ))}
          </ul>
        )}
      </>}
      {preview.validatorAbsent && <div className="error">검증기가 없어서 게시할 수 없습니다. 템플릿을 고치세요.</div>}
      {preview.errors.length > 0 && (
        <ul className="error" data-testid="issue-draft-errors">
          {preview.errors.map((error, index) => (
            <li key={index}>{validationErrorText(error)}</li>
          ))}
        </ul>
      )}
    </section>
  );
}


export function IssueDraftTextFields({
  opened, title, body, spans, needsDecision, disabled: editingDisabled,
  onEdit: edit, onSpansChange: setSpans, onNotice: setNotice,
}: {
  opened: Pick<IssueDraftOpened, "spanActions">;
  title: string;
  body: string;
  spans: IssueDraftSpan[];
  needsDecision: number[];
  disabled: boolean;
  onEdit(field: IssueDraftField, value: string): void;
  onSpansChange(spans: IssueDraftSpan[]): void;
  onNotice(message: string | null): void;
}) {
  const [spanClass, setSpanClass] = useState<IssueSpanClass | "">("secret");
  const titleInput = useRef<HTMLInputElement>(null);
  const bodyInput = useRef<HTMLTextAreaElement>(null);
  const markSelection = (field: IssueDraftField) => {
    const input = field === "title" ? titleInput.current : bodyInput.current;
    const next = input && addSpan(spans, { field, start: input.selectionStart ?? 0, end: input.selectionEnd ?? 0, class: spanClass || null, decision: null });
    if (next) {
      setSpans(next);
      setNotice(null);
    } else {
      setNotice("가릴 글을 먼저 선택하세요. 이미 지정한 구간과 겹치면 지정할 수 없습니다.");
    }
  };

  return (
        <section className="work-section work-form">
          <p className="muted">이 입력은 대화 초안과 자동 저장에 들어가지 않습니다. 패널을 닫으면 사라집니다.</p>
          <label>
            제목
            <input ref={titleInput} disabled={editingDisabled} value={title} onChange={(event) => edit("title", event.target.value)} data-testid="issue-draft-title" />
          </label>
          <label>
            본문
            <textarea ref={bodyInput} disabled={editingDisabled} value={body} onChange={(event) => edit("body", event.target.value)} rows={12} data-testid="issue-draft-body" />
          </label>
          <div className="issue-draft-mark">
            <label>
              민감 구간의 종류
              <select disabled={editingDisabled} value={spanClass} onChange={(event) => setSpanClass(event.target.value as IssueSpanClass | "")} data-testid="issue-draft-span-class">
                {(Object.keys(CLASS_LABEL) as IssueSpanClass[]).map((value) => (
                  <option key={value} value={value}>
                    {CLASS_LABEL[value]} ({ACTION_LABEL[opened.spanActions[value]]})
                  </option>
                ))}
                <option value="">모름 (직접 정함)</option>
              </select>
            </label>
            <button type="button" disabled={editingDisabled} onClick={() => markSelection("title")} data-testid="issue-draft-mark-title">
              제목의 선택을 가리기
            </button>
            <button type="button" disabled={editingDisabled} onClick={() => markSelection("body")} data-testid="issue-draft-mark-body">
              본문의 선택을 가리기
            </button>
          </div>
          {spans.length > 0 && (
            <ul data-testid="issue-draft-spans">
              {spans.map((span, index) => {
                const action = span.class ? opened.spanActions[span.class] : "user_confirm";
                const asks = action === "user_confirm" || needsDecision.includes(index) || span.decision !== null;
                return (
                  <li key={`${span.field}-${span.start}`} className={needsDecision.includes(index) ? "issue-draft-warning" : undefined} data-testid="issue-draft-span">
                    {FIELD_LABEL[span.field]} {span.start}–{span.end} · {span.class ? CLASS_LABEL[span.class] : "모름"} ·{" "}
                    {asks ? (
                      <select
                        disabled={editingDisabled}
                        value={span.decision ?? ""}
                        onChange={(event) =>
                          setSpans(spans.map((other, at) => (at === index ? { ...other, decision: (event.target.value || null) as IssueDraftSpan["decision"] } : other)))
                        }
                        data-testid="issue-draft-span-decision"
                      >
                        <option value="">처리를 고르세요</option>
                        <option value="remove">{ACTION_LABEL.remove}</option>
                        <option value="restorable_mask">{ACTION_LABEL.restorable_mask}</option>
                      </select>
                    ) : (
                      ACTION_LABEL[action]
                    )}{" "}
                    <button type="button" disabled={editingDisabled} onClick={() => setSpans(spans.filter((_, at) => at !== index))}>
                      지정 취소
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </section>
  );
}
