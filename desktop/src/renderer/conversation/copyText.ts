import { toast } from "../session/uiStore";

/** 글을 클립보드에 넣고 결과를 알린다. */
export function copyText(text: string, done: string): void {
  navigator.clipboard.writeText(text).then(
    () => toast(done),
    () => toast("복사하지 못했습니다"),
  );
}
