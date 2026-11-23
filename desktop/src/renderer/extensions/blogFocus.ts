// 알림을 누르면 그 글을 연다 (plan 31 §6.5). 패널이 닫혀 있으면 열릴 때 가져가고, 열려 있으면 이벤트로 받는다.
const EVENT = "akra:blog-focus";
let pending: string | null = null;

/**
 * `announce` 가 거짓이면 열린 패널에 알리지 않는다. 저장소를 바꾸는 중이면 지금 패널은 다른 저장소의 것이다:
 * 새 저장소의 패널이 열릴 때 가져간다.
 */
export function focusBlogArticle(id: string, announce = true): void {
  pending = id;
  if (announce) window.dispatchEvent(new CustomEvent(EVENT));
}

/** 기다리는 글 ID 를 한 번만 준다. */
export function takeBlogFocus(): string | null {
  const id = pending;
  pending = null;
  return id;
}

export function onBlogFocus(handler: () => void): () => void {
  window.addEventListener(EVENT, handler);
  return () => window.removeEventListener(EVENT, handler);
}

/** 패널이 지금 보이는 글이다. 그 글이 끝나면 화면이 바뀌므로 토스트를 띄우지 않는다. */
let viewing: string | null = null;

export function setViewingBlogArticle(id: string | null): void {
  viewing = id;
}

export function viewingBlogArticle(): string | null {
  return viewing;
}
