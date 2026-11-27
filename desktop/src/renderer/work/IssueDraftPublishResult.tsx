import type { IssuePublishStatus } from "./issueDraft";

const REFUSED_REASON: Record<string, string> = {
  target: "승인한 이슈가 아닙니다.",
  shared_copy: "승인한 뒤에 제목이나 본문이 바뀌었습니다.",
  policy_version: "승인한 뒤에 이슈 정책이 바뀌었습니다.",
  template_version: "승인한 뒤에 템플릿이 바뀌었습니다.",
};

type Props = { readonly status: IssuePublishStatus; readonly busy: boolean; readonly canShare: boolean; onClose(): void; onRecheck(): void; onReload(): void; onDiscard(): void; onAbandon(id: string): void; onSharedOnly(): void; onDismiss(): void };
export function IssueDraftPublishResult({status, busy, canShare, onClose, onRecheck, onReload, onDiscard, onAbandon, onSharedOnly, onDismiss}: Props) {
  return (<section className="work-section work-form" data-testid="issue-draft-status" data-status={status.kind}>
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
                <button type="button" onClick={() => onRecheck()} disabled={busy} data-testid="issue-draft-recheck">
                  원격을 다시 확인
                </button>
                <button type="button" onClick={() => onAbandon(status.intentId)} disabled={busy} data-testid="issue-draft-abandon">
                  결과 확인을 그만두기
                </button>
              </>
            )}
            {status.kind === "external_change" && (
              <>
                <strong>알려진 외부 변경</strong>
                <p className="muted">편집을 시작한 뒤에 원격의 글이 바뀌었습니다. 쓰지 않았습니다.</p>
                <button type="button" onClick={() => onReload()} disabled={busy} data-testid="issue-draft-reload">
                  원격 글을 다시 읽고 내 글을 유지
                </button>
                <button type="button" onClick={() => onDiscard()} disabled={busy} data-testid="issue-draft-discard">
                  내 글을 버리고 원격 글로 시작
                </button>
              </>
            )}
            {status.kind === "held" && (
              <>
                <strong>게시 보류</strong>
                <p className="muted">표식으로 가린 구간을 복원용으로 저장하지 못했습니다. 쓰지 않았습니다. 공유본만 게시하면 그 구간은 복원할 수 없습니다.</p>
                <button type="button" onClick={() => onSharedOnly()} disabled={!canShare || busy} data-testid="issue-draft-shared-only">
                  공유본만 게시
                </button>
                <button type="button" onClick={() => onDismiss()} disabled={busy}>
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
          </section>);
}
