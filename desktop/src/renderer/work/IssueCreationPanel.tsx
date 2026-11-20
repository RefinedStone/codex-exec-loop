// 원문은 이 패널의 메모리 안에만 존재한다. 생성 의도의 재개는 Gateway의 공유본 기록으로 한다.
import { useEffect, useRef, useState } from "react";

import "./IssueDraftPanel.css";
import { IssueDraftPreviewView, IssueDraftRequirements, IssueDraftTextFields } from "./IssueDraftFields";
import { shiftSpans, type IssueDraftField, type IssueDraftSpan } from "./issueDraft";
import {
  creationNeedsConfirmation, creationRestorePending, creationStatus, ISSUE_CREATE_METHOD, sameCreationApproval,
  type IssueCreationApproval, type IssueCreationIntent, type IssueCreationOpened,
  type IssueCreationPreview, type IssueCreationPublished, type IssueCreationTarget,
} from "./issueCreation";
import type { GatewayRequest } from "./notionHub";

const REFUSED: Record<string, string> = {
  approval_mismatch: "미리보기에서 승인한 값과 생성 요청이 다릅니다.",
  repository_changed: "저장소 식별자가 바뀌었습니다. 새 초안을 시작하세요.",
  intent_not_current: "더 최근의 생성 의도가 있습니다. 생성 기록을 다시 열어 주세요.",
  intent_abandoned: "종료된 생성 의도입니다. 새 초안을 시작하세요.",
  intent_requires_confirmation: "이미 보냈을 수 있는 요청입니다. 생성 결과를 먼저 확인하세요.",
  frozen_approval_changed: "기록된 승인과 현재 정책이 다릅니다. 새 초안을 시작하세요.",
  target: "승인한 저장소와 요청 대상이 다릅니다.",
  intent: "승인한 생성 의도와 요청이 다릅니다.",
  shared_copy: "승인한 공유본과 요청 내용이 다릅니다.",
  policy_version: "이슈 정책이 바뀌었습니다.",
  template_version: "템플릿이 바뀌었습니다.",
};
const errorMessage = (cause: unknown) => cause instanceof Error ? cause.message : String(cause);

export function IssueCreationPanel({ request, target, onClose, openExternal }: {
  request: GatewayRequest;
  target: IssueCreationTarget;
  onClose(): void;
  openExternal(url: string): void | Promise<void>;
}) {
  const [opened, setOpened] = useState<IssueCreationOpened | null>(null);
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [spans, setSpans] = useState<IssueDraftSpan[]>([]);
  const [preview, setPreview] = useState<IssueCreationPreview | null>(null);
  const [approval, setApproval] = useState<IssueCreationApproval | null>(null);
  const [busy, setBusy] = useState(false);
  const [unknownRequest, setUnknownRequest] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [resultUrl, setResultUrl] = useState("");
  const [acknowledgeUncertain, setAcknowledgeUncertain] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const epoch = useRef(0);
  const busyRef = useRef(false);
  const approvalRef = useRef(approval);
  approvalRef.current = approval;

  const installOpened = (next: IssueCreationOpened) => {
    setOpened(next);
    // Frozen intents expose only the approved shared copy, never a rewritten raw draft.
    setTitle(next.intent.state === "prepared" ? next.title : next.intent.sharedCopy?.title ?? "");
    setBody(next.intent.state === "prepared" ? next.body : next.intent.sharedCopy?.body ?? "");
    setSpans([]);
    setApproval(null);
    setPreview(null);
    setUnknownRequest(false);
    setResultUrl("");
    setAcknowledgeUncertain(false);
  };

  const installIntent = (intent: IssueCreationIntent) => {
    setOpened((current) => current ? { ...current, intent } : current);
    if (intent.state !== "prepared") {
      setTitle(intent.sharedCopy?.title ?? "");
      setBody(intent.sharedCopy?.body ?? "");
      setSpans([]);
    }
    setApproval(null);
    setPreview(null);
    setUnknownRequest(false);
    setAcknowledgeUncertain(false);
  };

  useEffect(() => {
    const generation = ++epoch.current;
    busyRef.current = false;
    setBusy(false);
    setUnknownRequest(false);
    setApproval(null);
    setPreview(null);
    setTitle("");
    setBody("");
    setSpans([]);
    setResultUrl("");
    setAcknowledgeUncertain(false);
    setNotice(null);
    setOpened(null);
    setError(null);
    void request<IssueCreationOpened>(ISSUE_CREATE_METHOD.open, target).then(
      (next) => { if (epoch.current === generation) installOpened(next); },
      (cause: unknown) => { if (epoch.current === generation) setError(errorMessage(cause)); },
    );
    return () => { epoch.current += 1; };
  }, [request, target.cwd, target.owner, target.repository]);

  const intent = opened?.intent;
  const editable = intent?.state === "prepared" && !unknownRequest;
  const resumable = intent?.state === "held" || intent?.state === "recorded";
  useEffect(() => {
    if (!opened || busy || unknownRequest || (!editable && !resumable)) return;
    let current = true;
    const timer = window.setTimeout(() => {
      void request<IssueCreationPreview>(ISSUE_CREATE_METHOD.preview, {
        target, intentId: opened.intent.intentId,
        text: editable ? { title, body, spans } : { title: opened.intent.sharedCopy?.title ?? "", body: opened.intent.sharedCopy?.body ?? "", spans: [] },
      }).then(
        (next) => {
          if (!current) return;
          setPreview(next);
          const approved = approvalRef.current;
          if (approved && (!next.approval || !sameCreationApproval(approved, next.approval))) {
            setApproval(null);
            setNotice("공유본이나 정책이 바뀌어서 승인을 지웠습니다. 미리보기를 다시 확인하세요.");
          }
        },
        (cause: unknown) => {
          if (!current) return;
          setPreview(null);
          setApproval(null);
          setError(errorMessage(cause));
        },
      );
    }, 300);
    return () => { current = false; window.clearTimeout(timer); };
  }, [request, target.cwd, target.owner, target.repository, intent?.intentId, intent?.state, title, body, spans, refresh, busy, unknownRequest]);

  const perform = async <T,>(method: string, params: unknown, accept: (result: T) => void, uncertainOnFailure = false) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    setNotice(null);
    const generation = epoch.current;
    try {
      const result = await request<T>(method, params);
      if (generation === epoch.current) accept(result);
    } catch (cause) {
      if (generation !== epoch.current) return;
      setError(errorMessage(cause));
      if (uncertainOnFailure) {
        setUnknownRequest(true);
        setApproval(null);
        setTitle("");
        setBody("");
        setSpans([]);
      }
    } finally {
      if (generation === epoch.current) {
        busyRef.current = false;
        setBusy(false);
      }
    }
  };

  const invalidateApproval = () => {
    if (approvalRef.current) setNotice("내용이 바뀌어서 승인을 지웠습니다. 새 미리보기를 확인하세요.");
    setApproval(null);
    setPreview(null);
  };
  const edit = (field: IssueDraftField, value: string) => {
    if (!editable || busyRef.current) return;
    invalidateApproval();
    setSpans((current) => shiftSpans(current, field, field === "title" ? title : body, value));
    if (field === "title") setTitle(value);
    else setBody(value);
  };
  const changeSpans = (next: IssueDraftSpan[]) => {
    if (!editable || busyRef.current) return;
    invalidateApproval();
    setSpans(next);
  };

  const publish = (sharedCopyOnly: boolean) => {
    if (!intent || !approval || (!editable && !(resumable && sharedCopyOnly))) return;
    const text = editable ? { title, body, spans } : { title: intent.sharedCopy?.title ?? "", body: intent.sharedCopy?.body ?? "", spans: [] };
    return perform<IssueCreationPublished>(ISSUE_CREATE_METHOD.publish, {
      target, intentId: intent.intentId, text, approval, sharedCopyOnly,
    }, (published) => {
      if (published.status === "blocked") {
        setPreview(published.preview);
        setApproval(null);
        setNotice("검증을 통과하지 못해 생성하지 않았습니다.");
      } else if (published.status === "refused") {
        setApproval(null);
        setRefresh((value) => value + 1);
        const recoverIntent = published.reason === "intent_not_current" || published.reason === "intent_requires_confirmation";
        if (recoverIntent) {
          setUnknownRequest(true);
          setPreview(null);
          setTitle("");
          setBody("");
          setSpans([]);
        }
        const reason = REFUSED[published.reason] ?? "생성 요청이 현재 기록과 일치하지 않아 거절되었습니다.";
        setNotice(reason + (recoverIntent ? "" : " 미리보기 또는 새 초안을 확인하세요."));
      } else {
        installIntent(published.intent);
      }
    }, true);
  };

  const recheck = () => intent && perform<IssueCreationIntent>(ISSUE_CREATE_METHOD.recheck, { target, intentId: intent.intentId }, installIntent);
  const confirm = () => intent && resultUrl.trim() && perform<IssueCreationIntent>(
    ISSUE_CREATE_METHOD.confirm, { target, intentId: intent.intentId, url: resultUrl.trim() }, installIntent,
  );
  const uncertain = unknownRequest || (intent ? creationNeedsConfirmation(intent) : false);
  const restart = () => {
    if (!intent || (uncertain && !acknowledgeUncertain)) return;
    return perform<IssueCreationOpened>(ISSUE_CREATE_METHOD.restart, {
      target, previousIntentId: intent.intentId, acknowledgeUncertain: uncertain && acknowledgeUncertain,
    }, installOpened, true);
  };
  const reopen = () => perform<IssueCreationOpened>(ISSUE_CREATE_METHOD.open, target, installOpened);
  const name = target.owner + "/" + target.repository;
  const ready = preview?.preview.status === "ready" ? preview.preview : null;
  const canApprove = Boolean(ready?.publishable && preview?.approval && !busy && !unknownRequest && (editable || resumable));
  const approvedNow = Boolean(approval && preview?.approval && sameCreationApproval(approval, preview.approval));
  const needsDecision = preview?.preview.status === "needs_confirmation" ? preview.preview.spanIndexes : [];

  return (
    <aside className="work-panel issue-draft" data-testid="issue-create-panel">
      <div className="work-header">
        <strong>새 이슈 · {name}</strong>
        <button type="button" onClick={onClose} data-testid="issue-create-close">닫기</button>
      </div>
      <div className="work-scroll">
        {!opened && !error && <p className="muted work-section">생성 의도와 템플릿을 읽는 중…</p>}
        {!opened && error && <button type="button" className="work-section" onClick={() => void reopen()} disabled={busy}>다시 읽기</button>}
        {opened && intent && <>
          {intent.predecessorIntentId && <p className="muted work-section" data-testid="issue-create-predecessor">
            이전 시도에서 새 초안으로 전환했으며, 이 초안의 게시는 별도의 새 이슈를 만듭니다.
          </p>}
          <IssueDraftRequirements opened={opened} />
          {editable && <IssueDraftTextFields
            opened={opened} title={title} body={body} spans={spans} needsDecision={needsDecision}
            disabled={busy} onEdit={edit} onSpansChange={changeSpans} onNotice={setNotice}
          />}
          {!editable && intent.sharedCopy && <section className="work-section" data-testid="issue-create-frozen">
            <h3>기록된 공유본</h3>
            <p className="muted">이 생성 의도에 승인된 내용입니다. 내용을 바꾸려면 새 초안을 시작하세요.</p>
            <div className="issue-draft-shared" data-testid="issue-create-frozen-title">{intent.sharedCopy.title}</div>
            <pre className="issue-draft-shared" data-testid="issue-create-frozen-body">{intent.sharedCopy.body}</pre>
          </section>}
          {(editable || resumable) && !unknownRequest && <IssueDraftPreviewView preview={preview?.preview ?? null} showMarkers={editable} />}
          {resumable && preview && !preview.approval && <p className="error work-section" data-testid="issue-create-frozen-policy">
            기록된 승인과 현재 정책 또는 템플릿이 달라 이 공유본을 다시 보낼 수 없습니다. 새 초안을 시작하고 다시 승인하세요.
          </p>}
          {(editable || resumable) && !unknownRequest && <section className="work-section work-form">
            <label className="issue-draft-approve">
              <input type="checkbox" checked={approval !== null} disabled={!canApprove}
                onChange={(event) => setApproval(event.target.checked ? preview?.approval ?? null : null)}
                data-testid="issue-create-approve" />
              {resumable ? "복원 연결 없이 기록된 공유본만 새 이슈로 게시하는 것을 승인합니다." : name + "에 위 공유본으로 새 이슈를 생성하는 것을 승인합니다."}
            </label>
            {editable && <button type="button" disabled={!approvedNow || !canApprove} onClick={() => void publish(false)} data-testid="issue-create-publish">{busy ? "처리 중…" : "새 이슈 생성"}</button>}
            {resumable && <button type="button" disabled={!approvedNow || !canApprove} onClick={() => void publish(true)} data-testid="issue-create-shared-only">공유본만 게시</button>}
          </section>}
          <section className="work-section work-form" data-testid="issue-create-status" data-status={unknownRequest ? "needs_confirmation" : creationStatus(intent)}>
            {intent.state === "prepared" && !unknownRequest && <p className="muted">아직 GitHub에 보내지 않았습니다.</p>}
            {resumable && !unknownRequest && <>
              <strong>{intent.state === "held" ? "생성 보류 · 아직 보내지 않음" : "생성 준비 기록됨 · 아직 보내지 않음"}</strong>
              <p className="muted">기록된 공유본은 바꾸지 않습니다. 복원 연결 없이 게시할지 승인하거나 새 초안을 시작하세요.</p>
            </>}
            {uncertain && <>
              <strong>생성 결과 확인 필요</strong>
              <p className="muted">이슈가 이미 생성됐을 수 있어 자동으로 다시 보내지 않습니다. 결과를 다시 조회하거나 생성된 이슈의 정확한 URL을 입력하세요.</p>
              {unknownRequest
                ? <button type="button" disabled={busy} onClick={() => void reopen()} data-testid="issue-create-reopen">생성 기록 다시 읽기</button>
                : <>
                  <button type="button" disabled={busy} onClick={() => void recheck()} data-testid="issue-create-recheck">결과 다시 확인</button>
                  <label>생성된 이슈 URL
                    <input value={resultUrl} onChange={(event) => setResultUrl(event.target.value)} disabled={busy}
                      placeholder={"https://github.com/" + name + "/issues/번호"} data-testid="issue-create-result-url" />
                  </label>
                  <button type="button" disabled={busy || !resultUrl.trim()} onClick={() => void confirm()} data-testid="issue-create-confirm">이 URL의 생성 결과 확인</button>
                </>}
            </>}
            {intent.state === "confirmed" && !unknownRequest && <>
              <strong>{creationRestorePending(intent) ? "생성 완료 · 복원 연결 대기" : "생성 확인"}</strong>
              <p className="muted">{creationRestorePending(intent) ? "이슈는 생성됐습니다. 복원 연결만 다시 확인하며 이슈를 다시 만들지 않습니다." : "GitHub의 저장소와 이슈, 승인한 공유본이 확인됐습니다."}</p>
              {intent.created && <button type="button" disabled={busy} onClick={() => void openExternal(intent.created!.url)} data-testid="issue-create-open-result">생성된 이슈 열기</button>}
              {creationRestorePending(intent) && <button type="button" disabled={busy} onClick={() => void recheck()} data-testid="issue-create-recheck">복원 연결 다시 확인</button>}
            </>}
            {intent.state === "abandoned" && <p className="muted">이전 생성 의도의 확인을 종료했습니다. 새 초안에서 시작하세요.</p>}
            {(!editable || unknownRequest) && <>
              {uncertain && <label className="issue-draft-approve">
                <input type="checkbox" checked={acknowledgeUncertain} disabled={busy} onChange={(event) => setAcknowledgeUncertain(event.target.checked)} data-testid="issue-create-restart-ack" />
                이전 요청으로 이미 생성되어 새 초안 게시 시 이슈가 중복될 수 있음을 확인합니다.
              </label>}
              <button type="button" disabled={busy || (uncertain && !acknowledgeUncertain)} onClick={() => void restart()} data-testid="issue-create-restart">새 초안 시작</button>
            </>}
          </section>
        </>}
        {notice && <p className="muted work-section" data-testid="issue-create-notice">{notice}</p>}
        {error && <div className="error work-section" data-testid="issue-create-error">{error}</div>}
      </div>
    </aside>
  );
}
