import { useState } from "react";
import { useDesktopStore } from "../session/desktopStore";
import type { DelegationActions, DelegationDetail } from "./DelegationPanel";
import type { IssueDraftField, IssueDraftPreview, IssueDraftTarget, IssueDraftText } from "./issueDraft";
import { readReview, revisedField } from "./draftReviewSuggestions";
import type { GatewayRequest } from "./notionHub";

type Preview = Extract<IssueDraftPreview, { status: "ready" }>;
type Job = { readonly id: string; readonly basis: Preview; readonly detail?: DelegationDetail };
type Props = {
  readonly target: IssueDraftTarget; readonly preview: Preview | null; readonly text: IssueDraftText;
  readonly jobs: Pick<DelegationActions, "detail" | "cancel">; readonly request: GatewayRequest;
  readonly disabled: boolean; onEdit(field: IssueDraftField, next: string, spans: IssueDraftText["spans"]): void;
};
export function IssueDraftReview({ target, preview, text, jobs, request, disabled, onEdit }: Props) {
  const attached = useDesktopStore(state => state.kernel.phase === "attached" && state.kernel.workspace === target.cwd);
  const [job, setJob] = useState<Job | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setNotice("");
    try { await action(); }
    catch (error) { setNotice(error instanceof Error ? error.message : "검토 요청을 확인하지 못했습니다."); }
    finally { setBusy(false); }
  };
  const start = () => run(async () => {
    if (!preview?.publishable || disabled || !attached) return;
    const { id } = await request<{id: string}>("akra/issue/review", { draft: { target, text }, approval: preview.approval, idempotencyKey: crypto.randomUUID() });
    const queued = { id, basis: preview };
    setJob(queued);
    setJob({ ...queued, detail: await jobs.detail(id) });
  });
  const refresh = (cancel: boolean) => run(async () => {
    if (!job || !attached) return;
    if (cancel) await jobs.cancel(job.id);
    setJob({ ...job, detail: await jobs.detail(job.id) });
  });
  const detail = job?.detail;
  const reply = detail?.result ? readReview(detail.result) : null;
  const stale = job !== null && preview !== job.basis;
  return <section className="work-section work-form" aria-label="공유본 품질 검토">
    <strong>공유본 품질 검토 (선택)</strong>
    <p className="muted">검토를 요청하면 검증된 공유본을 모델에 보냅니다. 검토 없이도 승인하고 게시할 수 있습니다.</p>
    {!attached && <p>검토하려면 이 저장소의 작업 폴더를 여세요.</p>}
    <button type="button" disabled={disabled || busy || !attached || !preview?.publishable || Boolean(job && (!detail || detail.canCancel))} onClick={() => void start()}>공유본 검토 요청</button>
    {job && <div>
      <p role="status">{detail?.status ?? "접수됨"} {detail?.reason}</p>
      <button type="button" disabled={busy || !attached} onClick={() => void refresh(false)}>검토 결과 확인</button>
      {detail?.canCancel && <button type="button" disabled={busy || !attached} onClick={() => void refresh(true)}>검토 취소</button>}
      {stale && <p>초안이나 정책이 바뀌었습니다. 이 검토의 권고는 적용할 수 없습니다.</p>}
      {detail?.result && !reply && <p>검토 결과 형식을 읽을 수 없습니다.</p>}
      {reply && ("refusal" in reply ? <p>검토 거절: {reply.refusal}</p> : reply.suggestions.length === 0 ? <p>권고가 없습니다.</p> : <ul>{reply.suggestions.map((item, index) => {
        const next = preview && !stale ? revisedField(text, preview.sharedCopy, item) : null;
        return <li key={index}><strong>{item.field === "title" ? "제목" : "본문"}: {item.problem}</strong><pre>{item.originalText}</pre><pre>{item.suggestedText}</pre>
          <button type="button" disabled={disabled || busy || !attached || next === null} onClick={() => { if (next !== null) onEdit(next.field, next.value, next.spans); }}>권고 적용</button></li>;
      })}</ul>)}
    </div>}
    {notice && <p role="alert">{notice}</p>}
  </section>;
}
