import { toast } from "../session/uiStore";

export const SHOW_COMMIT_ACTIONS = "akra:show-commit-actions";

export function openCommitActions(): void {
  if (window.dispatchEvent(new Event(SHOW_COMMIT_ACTIONS, { cancelable: true }))) {
    toast("커밋 그래프에서 작업할 커밋을 선택하십시오.");
  }
}
