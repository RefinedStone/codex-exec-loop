import { currentFileTarget } from "./attachmentSource";
import { targetKey } from "./fileAttachment";

export function threadDraftKey(threadId: string): string {
  const target = currentFileTarget();
  return target?.threadId === threadId ? targetKey(target) : threadId;
}
