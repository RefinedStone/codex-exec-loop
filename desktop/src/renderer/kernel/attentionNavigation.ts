import type { AttentionItem } from "../../shared/kernel-protocol/generated";

export function nextAttentionThread(items: readonly AttentionItem[], current: string | null): string | null {
  const ids = [...new Set(items.filter(item => item.subject.kind === "thread").map(item => item.subject.id))];
  if (!ids.length) return null;
  return ids[(ids.indexOf(current ?? "") + 1) % ids.length] ?? null;
}

/** Only newly observed IDs/revisions trigger one notification per snapshot. */
export function newAttentionCount(previous: readonly AttentionItem[] | null, current: readonly AttentionItem[] | null): number {
  if (!previous || !current) return 0;
  const seen = new Set(previous.map(item => JSON.stringify([item.id, item.revision])));
  return new Set(current.map(item => JSON.stringify([item.id, item.revision])).filter(key => !seen.has(key))).size;
}
