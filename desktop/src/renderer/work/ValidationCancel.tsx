// 게시·생성 중 실행되는 사용자 코드 검증기의 취소 단추이다 (#2858). 게시 요청이 실은 `validationId` 로 Gateway 에 취소를
// 따로 보낸다. 결과가 이미 정해졌으면 Gateway 가 거절하고, 그 사실을 보인다. 창을 닫는 것은 취소가 아니다.
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
  const [state, setState] = useState<{ id: string; text: string } | null>(null);
  if (!validationId) return null;
  const shown = state?.id === validationId ? state.text : null;
  const cancel = () => {
    setState({ id: validationId, text: "검증을 취소하는 중…" });
    request<{ cancelled: boolean }>(ISSUE_VALIDATION_CANCEL, { validationId }).then(
      ({ cancelled }) => setState({ id: validationId, text: cancelled ? "검증을 취소했습니다. 게시하지 않습니다." : "검증 결과가 이미 정해져 취소하지 못했습니다." }),
      (cause: unknown) => setState({ id: validationId, text: `검증을 취소하지 못했습니다: ${cause instanceof Error ? cause.message : String(cause)}` }),
    );
  };
  return (
    <span className="validation-cancel">
      <button type="button" onClick={cancel} disabled={shown !== null} data-testid={testId}>검증 취소</button>
      {shown && <span className="muted" data-testid={`${testId}-state`}>{shown}</span>}
    </span>
  );
}
