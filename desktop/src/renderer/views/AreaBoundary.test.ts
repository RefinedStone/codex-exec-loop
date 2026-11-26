import { isValidElement, type ReactElement, type ReactNode } from "react";
import { describe, expect, it } from "vitest";

import { AreaBoundary } from "./AreaBoundary";

/** 렌더 결과를 글과 단추로 훑는다. DOM 없이 경계의 동작만 본다. */
function texts(node: ReactNode): string[] {
  if (typeof node === "string" || typeof node === "number") return [String(node)];
  if (Array.isArray(node)) return node.flatMap(texts);
  if (isValidElement(node)) return texts((node.props as { children?: ReactNode }).children);
  return [];
}

function retryButton(node: ReactNode): ReactElement<{ onClick(): void }> | null {
  if (Array.isArray(node)) return node.map(retryButton).find(Boolean) ?? null;
  if (!isValidElement(node)) return null;
  const props = node.props as { children?: ReactNode; "data-testid"?: string };
  return props["data-testid"] === "area-retry" ? node as ReactElement<{ onClick(): void }> : retryButton(props.children);
}

describe("area error boundary (#2845)", () => {
  it("shows only the area name and redraws only its children on retry", () => {
    const boundary = new AreaBoundary({ area: "이 패널", children: "panel body" });
    Object.assign(boundary.state, AreaBoundary.getDerivedStateFromError());
    const failed = boundary.render();
    const shown = texts(failed).join("");
    expect(shown).toContain("이 패널");

    let next = boundary.state;
    boundary.setState = ((update: (state: typeof next) => typeof next) => { next = { ...next, ...update(next) }; }) as typeof boundary.setState;
    retryButton(failed)!.props.onClick();
    expect(next).toEqual({ failed: false, attempt: 1 });
    boundary.state = next;
    const redrawn = boundary.render() as ReactElement;
    expect(redrawn.key).toBe("1");
    expect(texts(redrawn)).toEqual(["panel body"]);
  });

  it("clears the failure when the viewed thread or panel changes without remounting the children", () => {
    const boundary = new AreaBoundary({ area: "대화", resetKey: "thread-a", children: "body" });
    boundary.state = { failed: true, attempt: 0 };
    let next = boundary.state;
    boundary.setState = ((update: typeof next) => { next = { ...next, ...update }; }) as typeof boundary.setState;
    boundary.componentDidUpdate({ resetKey: "thread-a" });
    expect(next.failed).toBe(true);
    Object.assign(boundary, { props: { area: "대화", resetKey: "thread-b", children: "body" } });
    boundary.componentDidUpdate({ resetKey: "thread-a" });
    expect(next).toEqual({ failed: false, attempt: 0 });
  });
});
