// 따로 모양을 정하지 않은 아이템의 표시이다 (P3 §6.3). 메시지, 생각, 도구 호출은 `turnBlocks` 가 묶어서 대화 화면이 그린다.
// 모르는 종류도 오류 없이 종류와 상태만 보여 준다.
import { memo } from "react";

import type { ThreadItem } from "../../shared/protocol/generated/v2/ThreadItem";

function statusOf(item: ThreadItem): string | null {
  const status = (item as { status?: unknown }).status;
  return typeof status === "string" ? status : null;
}

/** 상태 모델은 바뀐 아이템만 새 객체로 만들므로, 같은 객체면 다시 그리지 않는다. */
export const ItemView = memo(function ItemView({ item }: { item: ThreadItem }) {
  if (item.type === "plan") {
    return (
      <div className="item item-plan">
        <span className="tool-label">계획</span>
        <pre>{item.text}</pre>
      </div>
    );
  }
  const status = statusOf(item);
  return (
    <div className="item item-other muted">
      {item.type}
      {status ? ` · ${status}` : ""}
    </div>
  );
});
