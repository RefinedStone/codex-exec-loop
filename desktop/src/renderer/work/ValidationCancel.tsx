// 게시·생성 중 실행되는 사용자 코드 검증기의 취소 단추이다 (#2858). 게시 요청이 실은 `validationId` 로 Gateway 에 취소를
// 따로 보낸다. 검증이 아직 등록되지 않았거나 결과가 이미 정해졌으면 Gateway 가 거절하고, 그 사실을 보인다. 창을 닫는 것은 취소가 아니다.
import { useState } from "react";

export const ISSUE_VALIDATION_CANCEL = "akra/issue/validation/cancel";

/** 게시 요청마다 새 검증 ID 이다. */
export function newValidationId(): string {
  return crypto.randomUUID();
}

export function ValidationCancel({ validationId, request, testId }: {
  validationId: string | null;
  request<T>(method: string, params: unknown): Promise<T>;
  testId: string;
}) {
  // retry: 거절은 끝이 아니다. Gateway 는 요청마다 따로 검증을 시작하므로 등록 전의 취소는 거절된다. 게시 요청이 끝날 때까지 다시 누를 수 있다.
  const [state, setState] = useState<{ id: string; text: string; retry: boolean } | null>(null);
  if (!validationId) return null;
  const shown = state?.id === validationId ? state : null;
  const cancel = () => {
    setState({ id: validationId, text: "검증을 취소하는 중…", retry: false });
    request<{ cancelled: boolean }>(ISSUE_VALIDATION_CANCEL, { validationId }).then(
      ({ cancelled }) => setState(cancelled
        ? { id: validationId, text: "검증을 취소했습니다. 게시하지 않습니다.", retry: false }
        : { id: validationId, text: "검증을 취소하지 못했습니다: 아직 시작 전이거나 이미 끝났습니다. 다시 시도할 수 있습니다.", retry: true }),
      (cause: unknown) => setState({ id: validationId, text: `검증을 취소하지 못했습니다: ${cause instanceof Error ? cause.message : String(cause)}`, retry: true }),
    );
  };
  return (
    <span className="validation-cancel">
      <button type="button" onClick={cancel} disabled={shown !== null && !shown.retry} data-testid={testId}>검증 취소</button>
      {shown && <span className="muted" data-testid={`${testId}-state`}>{shown.text}</span>}
    </span>
  );
}
