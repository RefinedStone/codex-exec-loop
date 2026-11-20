// 이슈 초안 패널이다 (#2426). 기존 GitHub 이슈의 제목과 본문을 고치고, 민감 구간을 지정하고, 게시될 공유본을 보고,
// 승인한 뒤 게시한다. 검증, 마스킹, 승인 검사, 게시 의도 기록은 Gateway 가 한다. 이 패널은 답을 그리고 명령을 보낸다.
//
// 이 패널의 입력은 민감 입력 경로이다. 로컬 글은 이 컴포넌트의 상태에만 있다. 대화 입력창의 초안, 자동 저장,
// localStorage, 콘솔에 쓰지 않는다. 패널을 닫으면 사라진다.
import { useEffect, useRef, useState } from "react";

import "./IssueDraftPanel.css";
import {
  addSpan,
  intentStatus,
  ISSUE_METHOD,
  type IssueDraftApproval,
  type IssueDraftField,
  type IssueDraftIntent,
  type IssueDraftOpened,
  type IssueDraftPreview,
  type IssueDraftPublished,
  type IssueDraftSpan,
  type IssueDraftTarget,
  type IssuePublishStatus,
  type IssueSpanAction,
  type IssueSpanClass,
  publishStatus,
  sameApproval,
  shiftSpans,
  validationErrorText,
} from "./issueDraft";
import type { GatewayRequest } from "./notionHub";

/** 입력이 멈춘 뒤 미리보기를 다시 만들 때까지의 시간이다. */
const PREVIEW_DELAY_MS = 300;

const CLASS_LABEL: Record<IssueSpanClass, string> = { secret: "비밀값", name: "이름", path: "경로", log: "로그" };
const ACTION_LABEL: Record<IssueSpanAction, string> = { remove: "제거", restorable_mask: "표식으로 가림", user_confirm: "사용자가 정함" };
const FIELD_LABEL: Record<IssueDraftField, string> = { title: "제목", body: "본문" };
const TEMPLATE_PROBLEM: Record<string, string> = {
  missing: "정책이 가리킨 템플릿 파일이 없습니다.",
  unreadable: "템플릿 파일을 읽지 못했습니다.",
  invalid: "템플릿 파일을 해석하지 못했습니다.",
};
const REFUSED_REASON: Record<string, string> = {
  target: "승인한 이슈가 아닙니다.",
  shared_copy: "승인한 뒤에 제목이나 본문이 바뀌었습니다.",
  policy_version: "승인한 뒤에 이슈 정책이 바뀌었습니다.",
  template_version: "승인한 뒤에 템플릿이 바뀌었습니다.",
};

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

type OpenState = { phase: "loading" } | { phase: "failed"; message: string } | { phase: "loaded"; opened: IssueDraftOpened };

function Requirements({ opened }: { opened: IssueDraftOpened }) {
  const { template, requiredSections } = opened;
  const problem = TEMPLATE_PROBLEM[template.status];
  return (
    <section className="work-section" data-testid="issue-draft-requirements">
      <h3>요구 사항</h3>
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

function Preview({ preview }: { preview: IssueDraftPreview | null }) {
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

export function IssueDraftPanel({ request, target, onClose }: { request: GatewayRequest; target: IssueDraftTarget; onClose(): void }) {
  const [openState, setOpenState] = useState<OpenState>({ phase: "loading" });
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [spans, setSpans] = useState<IssueDraftSpan[]>([]);
  const [spanClass, setSpanClass] = useState<IssueSpanClass | "">("secret");
  const [preview, setPreview] = useState<IssueDraftPreview | null>(null);
  const [approval, setApproval] = useState<IssueDraftApproval | null>(null);
  const [status, setStatus] = useState<IssuePublishStatus | null>(null);
  // 외부 변경 뒤에 다시 읽은 원격 글이다. 사용자가 자기 글과 견준다.
  const [remoteText, setRemoteText] = useState<{ title: string; body: string } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // 정책 파일은 이 화면 밖에서 바뀐다. 이 값을 올리면 미리보기를 다시 만든다.
  const [previewCount, setPreviewCount] = useState(0);
  const approvalRef = useRef<IssueDraftApproval | null>(null);
  approvalRef.current = approval;
  const titleInput = useRef<HTMLInputElement>(null);
  const bodyInput = useRef<HTMLTextAreaElement>(null);

  /** 원격을 읽는다. `keepDraft` 이면 사용자의 글을 두고 편집 기준만 바꾼다 (외부 변경 뒤). */
  const open = async (keepDraft: boolean) => {
    setError(null);
    if (!keepDraft) setOpenState({ phase: "loading" });
    try {
      const opened = await request<IssueDraftOpened>(ISSUE_METHOD.open, target);
      setOpenState({ phase: "loaded", opened });
      setApproval(null);
      setStatus(keepDraft ? null : intentStatus(opened.intent));
      setRemoteText(keepDraft ? { title: opened.title, body: opened.body } : null);
      if (!keepDraft) {
        setTitle(opened.title);
        setBody(opened.body);
        setSpans([]);
      }
      return opened;
    } catch (cause) {
      if (keepDraft) setError(message(cause));
      else setOpenState({ phase: "failed", message: message(cause) });
      return null;
    }
  };

  useEffect(() => {
    void open(false);
    // 대상이 바뀌면 처음부터 다시 읽는다.
  }, [request, target.cwd, target.owner, target.repository, target.number]);

  const loaded = openState.phase === "loaded";
  useEffect(() => {
    if (!loaded) return;
    let current = true;
    const timer = window.setTimeout(() => {
      request<IssueDraftPreview>(ISSUE_METHOD.preview, { target, text: { title, body, spans } }).then(
        (next) => {
          if (!current) return;
          setPreview(next);
          setError(null);
          const approved = approvalRef.current;
          // 승인은 그때의 공유본, 정책 판, 템플릿 판에 묶여 있다. 하나라도 바뀌면 다시 승인한다.
          if (approved && !(next.status === "ready" && sameApproval(approved, next.approval))) {
            setApproval(null);
            setNotice("내용이나 정책이 바뀌어서 승인을 지웠습니다. 미리보기를 보고 다시 승인하세요.");
          }
        },
        (cause: unknown) => {
          if (!current) return;
          setPreview(null);
          setApproval(null);
          setError(message(cause));
        },
      );
    }, PREVIEW_DELAY_MS);
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [loaded, title, body, spans, previewCount]);

  const edit = (field: IssueDraftField, next: string) => {
    setSpans((current) => shiftSpans(current, field, field === "title" ? title : body, next));
    if (field === "title") setTitle(next);
    else setBody(next);
    // 끝난 게시의 결과는 새 글의 상태가 아니다.
    if (status?.kind === "confirmed" || status?.kind === "refused") setStatus(null);
  };

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

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await action();
    } catch (cause) {
      setError(message(cause));
    } finally {
      setBusy(false);
    }
  };

  const publish = (sharedCopyOnly: boolean) =>
    run(async () => {
      if (openState.phase !== "loaded" || !approval) return;
      const published = await request<IssueDraftPublished>(ISSUE_METHOD.publish, {
        target,
        text: { title, body, spans },
        approval,
        editBasis: openState.opened.editBasis,
        sharedCopyOnly,
      });
      if (published.status === "blocked") {
        setPreview(published.preview);
        setApproval(null);
        setNotice("검증을 통과하지 못해서 게시하지 않았습니다.");
        return;
      }
      setStatus(publishStatus(published));
      // 게시한 뒤의 편집 기준은 게시한 공유본이다. 로컬 글은 버리고 원격 글에서 다시 시작한다.
      if (published.status === "confirmed") await open(false);
      if (published.status === "refused") {
        setApproval(null);
        setPreviewCount((count) => count + 1);
      }
    });

  const recheck = () =>
    run(async () => {
      const { intent } = await request<{ intent: IssueDraftIntent | null }>(ISSUE_METHOD.recheck, target);
      setStatus(intentStatus(intent));
      if (intent?.state === "confirmed") await open(false);
      if (intent?.state === "needs_confirmation") setNotice("원격의 글이 아직 승인한 공유본과 다릅니다. 게시가 되었는지 알 수 없습니다.");
    });

  const abandon = (intentId: string) =>
    run(async () => {
      await request(ISSUE_METHOD.abandon, { cwd: target.cwd, intentId });
      setStatus(null);
      setApproval(null);
      setNotice("결과 확인을 그만두었습니다. 다시 게시하려면 원격 글을 다시 읽고 승인하세요.");
    });

  const name = `${target.owner}/${target.repository}#${target.number}`;
  if (openState.phase !== "loaded") {
    return (
      <aside className="work-panel issue-draft" data-testid="issue-draft-panel">
        <div className="work-header">
          <strong>이슈 수정 {name}</strong>
          <button type="button" onClick={onClose}>
            닫기
          </button>
        </div>
        {openState.phase === "loading" && <p className="muted work-section">읽는 중…</p>}
        {openState.phase === "failed" && (
          <div className="error work-section" data-testid="issue-draft-error">
            이슈를 읽을 수 없습니다. gh 로그인과 이슈 정책 파일을 확인하세요. {openState.message}
          </div>
        )}
      </aside>
    );
  }

  const opened = openState.opened;
  const needsDecision = preview?.status === "needs_confirmation" ? preview.spanIndexes : [];
  const ready = preview?.status === "ready" ? preview : null;
  // 결과를 모르는 게시가 있으면 새 게시를 보내지 않는다. Gateway 도 막는다.
  const waiting = status?.kind === "needs_confirmation" || status?.kind === "held";
  // 게시한 글을 다시 읽을 때 아직 게시하지 않은 편집을 덮어쓰지 않도록 결과 확인까지 입력을 잠근다.
  const editingDisabled = busy || status?.kind === "needs_confirmation";
  return (
    <aside className="work-panel issue-draft" data-testid="issue-draft-panel">
      <div className="work-header">
        <strong>이슈 수정 {name}</strong>
        <button type="button" onClick={onClose} data-testid="issue-draft-close">
          닫기
        </button>
      </div>
      <div className="work-scroll">
        <Requirements opened={opened} />
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
        {remoteText && (
          <details className="work-section" data-testid="issue-draft-remote">
            <summary>지금 원격의 글 (게시하면 내 글로 바뀝니다)</summary>
            <div className="issue-draft-shared">{remoteText.title}</div>
            <pre className="issue-draft-shared">{remoteText.body}</pre>
          </details>
        )}
        <Preview preview={preview} />
        <section className="work-section work-form">
          <label className="issue-draft-approve">
            <input
              type="checkbox"
              checked={approval !== null}
              disabled={!ready?.publishable || busy || waiting}
              onChange={(event) => setApproval(event.target.checked && ready ? ready.approval : null)}
              data-testid="issue-draft-approve"
            />
            위의 공유본을 {name} 의 제목과 본문으로 게시하는 것을 승인합니다.
          </label>
          <button type="button" onClick={() => void publish(false)} disabled={!approval || busy || waiting} data-testid="issue-draft-publish">
            {busy ? "처리 중…" : "게시"}
          </button>
          {notice && (
            <p className="muted" data-testid="issue-draft-notice">
              {notice}
            </p>
          )}
          {error && (
            <div className="error" data-testid="issue-draft-action-error">
              {error}
            </div>
          )}
        </section>
        {status && (
          <section className="work-section work-form" data-testid="issue-draft-status" data-status={status.kind}>
            {status.kind === "confirmed" && (
              <>
                <strong>게시 확인</strong>
                <p className="muted">원격의 글이 승인한 공유본과 같습니다. 입력창의 글은 지금 원격의 글입니다.</p>
                <button type="button" onClick={onClose} disabled={busy} data-testid="issue-draft-done">
                  닫기
                </button>
              </>
            )}
            {status.kind === "needs_confirmation" && (
              <>
                <strong>게시 결과 확인 필요</strong>
                <p className="muted">쓰기를 보냈지만 결과를 모릅니다. 자동으로 다시 보내지 않습니다. 결과 확인을 마치거나 그만둘 때까지 초안 편집을 잠급니다.</p>
                <button type="button" onClick={() => void recheck()} disabled={busy} data-testid="issue-draft-recheck">
                  원격을 다시 확인
                </button>
                <button type="button" onClick={() => void abandon(status.intentId)} disabled={busy} data-testid="issue-draft-abandon">
                  결과 확인을 그만두기
                </button>
              </>
            )}
            {status.kind === "external_change" && (
              <>
                <strong>알려진 외부 변경</strong>
                <p className="muted">편집을 시작한 뒤에 원격의 글이 바뀌었습니다. 쓰지 않았습니다.</p>
                <button type="button" onClick={() => void run(async () => void (await open(true)))} disabled={busy} data-testid="issue-draft-reload">
                  원격 글을 다시 읽고 내 글을 유지
                </button>
                <button type="button" onClick={() => void open(false)} disabled={busy} data-testid="issue-draft-discard">
                  내 글을 버리고 원격 글로 시작
                </button>
              </>
            )}
            {status.kind === "held" && (
              <>
                <strong>게시 보류</strong>
                <p className="muted">표식으로 가린 구간을 복원용으로 저장하지 못했습니다. 쓰지 않았습니다. 공유본만 게시하면 그 구간은 복원할 수 없습니다.</p>
                <button type="button" onClick={() => void publish(true)} disabled={!approval || busy} data-testid="issue-draft-shared-only">
                  공유본만 게시
                </button>
                <button type="button" onClick={() => setStatus(null)} disabled={busy}>
                  게시하지 않기
                </button>
              </>
            )}
            {status.kind === "refused" && (
              <>
                <strong>승인 갱신 필요</strong>
                <p className="muted">{REFUSED_REASON[status.reason] ?? status.reason} 쓰지 않았습니다. 미리보기를 보고 다시 승인하세요.</p>
              </>
            )}
          </section>
        )}
      </div>
    </aside>
  );
}
