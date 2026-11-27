// 이슈 초안 패널이다 (#2426). 기존 GitHub 이슈의 제목과 본문을 고치고, 민감 구간을 지정하고, 게시될 공유본을 보고,
// 승인한 뒤 게시한다. 검증, 마스킹, 승인 검사, 게시 의도 기록은 Gateway 가 한다. 이 패널은 답을 그리고 명령을 보낸다.
//
// 이 패널의 입력은 민감 입력 경로이다. 로컬 글은 이 컴포넌트의 상태에만 있다. 대화 입력창의 초안, 자동 저장,
// localStorage, 콘솔에 쓰지 않는다. 패널을 닫으면 사라진다.
import { useEffect, useRef, useState } from "react";

import "./IssueDraftPanel.css";
import type { DelegationActions } from "./DelegationPanel";
import { IssueDraftReview } from "./IssueDraftReview";
import { IssueDraftOpening } from "./IssueDraftOpening";
import { IssueDraftPublishResult } from "./IssueDraftPublishResult";
import { IssueReadinessPanel } from "./IssueReadinessPanel";
import { IssueDraftRequirements as Requirements, IssueDraftPreviewView as Preview, IssueDraftTextFields } from "./IssueDraftFields";
import {
  cancelledByUser, intentStatus, expectedLabels, ISSUE_METHOD, publishStatus, sameApproval, shiftSpans,
  type IssueDraftApproval, type IssueDraftField, type IssueDraftIntent, type IssueDraftOpened,
  type IssueDraftPreview, type IssueDraftPublished, type IssueDraftSpan, type IssueDraftTarget, type IssuePublishStatus,
} from "./issueDraft";
import type { GatewayRequest } from "./notionHub";
import { newValidationId, ValidationCancel } from "./ValidationCancel";

/** 입력이 멈춘 뒤 미리보기를 다시 만들 때까지의 시간이다. */
const PREVIEW_DELAY_MS = 300;



function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

type OpenState = { phase: "loading" } | { phase: "failed"; message: string } | { phase: "loaded"; opened: IssueDraftOpened };

export function IssueDraftPanel({ request, reviewJobs, target, onClose }: { readonly target: IssueDraftTarget; readonly request: GatewayRequest; readonly reviewJobs: Pick<DelegationActions, "detail" | "cancel">; onClose(): void }) {
  const [openState, setOpenState] = useState<OpenState>({ phase: "loading" });
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [spans, setSpans] = useState<IssueDraftSpan[]>([]);
  const [labelInput, setLabelInput] = useState("");
  const previewRequest = useRef(0);
  const epoch = useRef(0);
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

  /** 원격을 읽는다. `keepDraft` 이면 사용자의 글을 두고 편집 기준만 바꾼다 (외부 변경 뒤). */
  const open = async (keepDraft: boolean, generation = epoch.current) => {
    setError(null);
    if (!keepDraft) {
      previewRequest.current += 1;
      setPreview(null);
      setApproval(null);
      setOpenState({ phase: "loading" });
    }
    try {
      const opened = await request<IssueDraftOpened>(ISSUE_METHOD.open, target);
      if (generation !== epoch.current) return null;
      setOpenState({ phase: "loaded", opened });
      setApproval(null);
      setStatus(keepDraft ? null : intentStatus(opened.intent));
      setRemoteText(keepDraft ? { title: opened.title, body: opened.body } : null);
      if (!keepDraft) {
        setTitle(opened.title);
        setBody(opened.body);
        setSpans([]);
        setLabelInput("");
      }
      return opened;
    } catch (cause) {
      if (generation !== epoch.current) return null;
      if (keepDraft) setError(message(cause));
      else setOpenState({ phase: "failed", message: message(cause) });
      return null;
    }
  };

  useEffect(() => {
    const generation = ++epoch.current;
    void open(false, generation);
    return () => { epoch.current += 1; previewRequest.current += 1; };
    // 대상이 바뀌면 처음부터 다시 읽는다.
  }, [request, target.cwd, target.owner, target.repository, target.number]);

  const loaded = openState.phase === "loaded";
  useEffect(() => {
    const requestId = ++previewRequest.current;
    const generation = epoch.current;
    setPreview(null);
    if (!loaded) return;
    let current = true;
    const timer = window.setTimeout(() => {
      request<IssueDraftPreview>(ISSUE_METHOD.preview, { target, text: { title, body, spans }, expectedLabels: expectedLabels(labelInput) }).then(
        (next) => {
          if (!current || requestId !== previewRequest.current || generation !== epoch.current) return;
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
          if (!current || requestId !== previewRequest.current || generation !== epoch.current) return;
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
  }, [request, target.cwd, target.owner, target.repository, target.number, loaded, title, body, spans, labelInput, previewCount]);

  const invalidatePreview = (changedText = false) => {
    previewRequest.current += 1;
    setPreview(null);
    if (changedText) {
      if (approvalRef.current) setNotice("내용이나 정책이 바뀌어서 승인을 지웠습니다. 미리보기를 보고 다시 승인하세요.");
      setApproval(null);
    }
  };

  const edit = (field: IssueDraftField, next: string, reviewedSpans?: IssueDraftSpan[]) => {
    invalidatePreview(true);
    setSpans((current) => reviewedSpans ?? shiftSpans(current, field, field === "title" ? title : body, next));
    if (field === "title") setTitle(next);
    else setBody(next);
    // 끝난 게시의 결과는 새 글의 상태가 아니다.
    if (status?.kind === "confirmed" || status?.kind === "refused") setStatus(null);
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

  const [validationId, setValidationId] = useState<string | null>(null);
  const publish = (sharedCopyOnly: boolean) =>
    run(async () => {
      if (openState.phase !== "loaded" || !approval) return;
      const id = newValidationId();
      setValidationId(id);
      const published = await request<IssueDraftPublished>(ISSUE_METHOD.publish, {
        target,
        text: { title, body, spans },
        approval,
        editBasis: openState.opened.editBasis,
        sharedCopyOnly,
        validationId: id,
      }).finally(() => setValidationId(null));
      if (published.status === "blocked") {
        setPreview(published.preview);
        setApproval(null);
        const errors = published.preview.status === "ready" ? published.preview.errors : [];
        setNotice(cancelledByUser(errors) ? "검증을 취소해서 게시하지 않았습니다." : "검증을 통과하지 못해서 게시하지 않았습니다.");
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
  if (openState.phase !== "loaded") return <IssueDraftOpening name={name} state={openState} onClose={onClose} />;

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
        <IssueDraftTextFields
          opened={opened} title={title} body={body} spans={spans} needsDecision={needsDecision}
          disabled={editingDisabled} onEdit={edit} onSpansChange={(next) => { invalidatePreview(true); setSpans(next); }} onNotice={setNotice}
        />
        {remoteText && (
          <details className="work-section" data-testid="issue-draft-remote">
            <summary>지금 원격의 글 (게시하면 내 글로 바뀝니다)</summary>
            <div className="issue-draft-shared">{remoteText.title}</div>
            <pre className="issue-draft-shared">{remoteText.body}</pre>
          </details>
        )}
        <Preview preview={preview} />
        <IssueDraftReview request={request} jobs={reviewJobs} target={target} preview={ready} text={{title,body,spans}} disabled={editingDisabled || waiting} onEdit={edit} />
        <IssueReadinessPanel result={preview?.readiness} pending={preview === null && error === null}
          labels={labelInput} disabled={editingDisabled}
          onLabelsChange={(value) => { invalidatePreview(); setLabelInput(value); }}
          onRefresh={() => { invalidatePreview(); setPreviewCount((count) => count + 1); }} />
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
          <ValidationCancel validationId={validationId} request={request} testId="issue-draft-cancel-validation" />
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
        {status && <IssueDraftPublishResult status={status} busy={busy} canShare={approval !== null} onClose={onClose}
          onRecheck={() => void recheck()} onReload={() => void run(async () => void (await open(true)))} onDiscard={() => void open(false)}
          onAbandon={id => void abandon(id)} onSharedOnly={() => void publish(true)} onDismiss={() => setStatus(null)} />}
      </div>
    </aside>
  );
}
