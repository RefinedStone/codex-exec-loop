import { Component, Suspense, lazy, useMemo, useState, type ReactNode } from "react";
import { panelLoaders, type RegisteredPanelProps } from "./registry";
import type { RegisteredPanelId } from "./panelDefinitions";

class PanelBoundary extends Component<{ children: ReactNode; retry(): void }, { failed: boolean }> {
  override state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  override render() {
    if (this.state.failed) return <div role="alert" className="empty-row">
      <p>이 패널을 열지 못했습니다. 확장의 활성 설정은 유지됩니다.</p>
      <button type="button" onClick={this.props.retry}>패널 다시 열기</button>
    </div>;
    return this.props.children;
  }
}

/** 로딩·실패는 이 화면의 수명에만 속한다. close/disable 뒤 module이 와도 패널을 다시 열지 않는다. */
export function ExtensionPanel({ id, ...props }: RegisteredPanelProps & { id: RegisteredPanelId }) {
  const [attempt, setAttempt] = useState(0);
  const loader = panelLoaders[id];
  const Panel = useMemo(() => lazy(loader), [loader, attempt]);
  // 새 커널의 활성 상태가 확인되기 전에는 대기 중인 글 선택을 소비하지 않는다.
  if (!props.enabled) return <p className="empty-row" role="status">확장 상태를 확인하고 있습니다.</p>;
  return <PanelBoundary key={`${id}:${props.scopeKey}:${attempt}`} retry={() => setAttempt(value => value + 1)}>
    <Suspense fallback={<p className="empty-row" role="status">패널을 여는 중…</p>}>
      <Panel {...props} />
    </Suspense>
  </PanelBoundary>;
}
