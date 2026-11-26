// 대화 영역의 오른쪽 클릭 메뉴이다 (#2842). 선택한 글의 인용·복사, 메시지·코드 복사, 찾기, 여기서 갈래를 준다.
// 되감기는 넣지 않는다. 메뉴를 그리면 운영체제의 편집 메뉴는 이 영역에서 뜨지 않는다.
import { useEffect, useRef } from "react";

import { useFittedMenuPosition } from "../views/menuPosition";
import { copyText } from "./copyText";

export interface ConversationMenuTarget {
  x: number;
  y: number;
  /** 대화 안에서 고른 글이다. 없으면 빈 글이다. */
  selection: string;
  /** 누른 자리의 메시지에 있는 복사 단추이다. 원문(Markdown)을 복사한다. */
  messageCopy: HTMLButtonElement | null;
  /** 누른 자리의 코드 블록 글이다. */
  code: string | null;
  /** 누른 자리의 링크 주소이다. 운영체제 메뉴의 링크 복사를 대신한다. */
  link: string | null;
}

/** 메뉴를 열 자리의 정보를 모은다. 대화 밖이면 null 이다. */
export function conversationMenuTarget(event: { clientX: number; clientY: number; target: EventTarget | null }, container: Element): ConversationMenuTarget | null {
  const target = event.target instanceof Element ? event.target : null;
  if (!target || !container.contains(target)) return null;
  const selected = window.getSelection();
  const selection = selected && !selected.isCollapsed && selected.anchorNode && container.contains(selected.anchorNode) ? selected.toString() : "";
  const message = target.closest('[data-testid="item-agent"], [data-testid="item-user"]');
  return {
    x: event.clientX,
    y: event.clientY,
    selection,
    messageCopy: message?.querySelector<HTMLButtonElement>(".message-copy") ?? null,
    code: target.closest("pre")?.textContent ?? null,
    link: target.closest<HTMLAnchorElement>("a[href]")?.href || null,
  };
}

export function ConversationMenu({ target, onQuote, onFind, onFork, onClose }: {
  target: ConversationMenuTarget;
  onQuote?(text: string): void;
  onFind(): void;
  onFork?(): void;
  onClose(): void;
}) {
  const menu = useRef<HTMLUListElement>(null);
  const position = useFittedMenuPosition(menu, target);
  useEffect(() => {
    menu.current?.querySelector<HTMLButtonElement>("button")?.focus();
    const close = (event: Event) => {
      if (event instanceof KeyboardEvent ? event.key === "Escape" : !menu.current?.contains(event.target as Node)) onClose();
    };
    window.addEventListener("pointerdown", close, true);
    window.addEventListener("keydown", close);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("pointerdown", close, true);
      window.removeEventListener("keydown", close);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);
  const item = (label: string, testId: string, action: () => void) => (
    <li key={testId}>
      <button type="button" role="menuitem" data-testid={testId} onClick={() => { onClose(); action(); }}>{label}</button>
    </li>
  );
  const { selection, messageCopy, code, link } = target;
  return (
    <ul ref={menu} className="conversation-menu" role="menu" aria-label="대화 메뉴" style={{ left: position.x, top: position.y }} data-testid="conversation-menu">
      {selection && onQuote && item("선택한 글 인용", "conversation-menu-quote", () => onQuote(selection))}
      {selection && item("선택한 글 복사", "conversation-menu-copy-selection", () => copyText(selection, "선택한 글을 복사했습니다"))}
      {link && item("링크 주소 복사", "conversation-menu-copy-link", () => copyText(link, "링크 주소를 복사했습니다"))}
      {messageCopy && item("메시지 복사", "conversation-menu-copy-message", () => messageCopy.click())}
      {code !== null && item("코드 복사", "conversation-menu-copy-code", () => copyText(code, "코드를 복사했습니다"))}
      {item("대화 안 찾기", "conversation-menu-find", onFind)}
      {onFork && item("여기서 갈래", "conversation-menu-fork", onFork)}
    </ul>
  );
}
