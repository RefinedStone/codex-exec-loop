// 화면 영역 하나의 오류 경계이다 (#2845). 오른쪽 패널 탭, 대화, 사이드바가 그리다가 실패해도 다른 영역과 입력창은
// 남는다. 오류 글에는 영역 이름만 쓴다: 오류 메시지에는 경로, 대화 본문, 토큰이 들어 있을 수 있다.
import { Component, Fragment, type ReactNode } from "react";

interface AreaBoundaryState {
  failed: boolean;
  /** 다시 시도할 때마다 늘린다. 자식을 새로 만들어 이 영역만 다시 그린다. */
  attempt: number;
}

export class AreaBoundary extends Component<{ area: string; children: ReactNode }, AreaBoundaryState> {
  override state: AreaBoundaryState = { failed: false, attempt: 0 };

  static getDerivedStateFromError(): Partial<AreaBoundaryState> {
    return { failed: true };
  }

  override componentDidCatch(error: unknown): void {
    // 메시지와 스택은 남기지 않는다. 종류만 개발자 도구에 남긴다.
    console.error(`[${this.props.area}] render failed: ${error instanceof Error ? error.name : typeof error}`);
  }

  retry = (): void => {
    this.setState((state) => ({ failed: false, attempt: state.attempt + 1 }));
  };

  override render(): ReactNode {
    if (this.state.failed) {
      return (
        <div role="alert" className="empty-row area-error" data-testid="area-error">
          <p>{this.props.area}을(를) 그리지 못했습니다. 다른 화면은 그대로입니다.</p>
          <button type="button" className="button" onClick={this.retry} data-testid="area-retry">다시 시도</button>
        </div>
      );
    }
    return <Fragment key={this.state.attempt}>{this.props.children}</Fragment>;
  }
}
