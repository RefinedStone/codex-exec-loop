import type { AttentionItem } from "../../shared/kernel-protocol/generated";

export type AttentionPreference = { read: boolean; pinned: boolean; snoozedUntil: number | null };
export type AttentionPreferenceField = keyof AttentionPreference;
const PREFIX = "akra:attention:v1:";
const EMPTY: AttentionPreference = { read: false, pinned: false, snoozedUntil: null };

/** A preference belongs to the local profile and exact kernel authority revision.
 * Reading, snoozing and pinning are personal display choices, not resolution.
 * Separate field keys avoid clobbering another window's independent edits.
 */
export function attentionPreferenceKey(kernel: string, item: Pick<AttentionItem, "id" | "revision">, field: AttentionPreferenceField): string {
  return PREFIX + JSON.stringify([kernel, item.id, item.revision, field]);
}

export function readAttentionPreference(storage: Pick<Storage, "getItem">, kernel: string, item: AttentionItem): AttentionPreference {
  try {
    const read = storage.getItem(attentionPreferenceKey(kernel, item, "read")) === "true";
    const pinned = storage.getItem(attentionPreferenceKey(kernel, item, "pinned")) === "true";
    const raw = storage.getItem(attentionPreferenceKey(kernel, item, "snoozedUntil"));
    const until = raw === null ? NaN : Number(raw);
    return { read, pinned, snoozedUntil: Number.isSafeInteger(until) && until > 0 ? until : null };
  } catch { return { ...EMPTY }; }
}

export function writeAttentionPreference<K extends AttentionPreferenceField>(storage: Pick<Storage, "setItem" | "removeItem">,
  kernel: string, item: AttentionItem, field: K, value: AttentionPreference[K]): boolean {
  try {
    const key = attentionPreferenceKey(kernel, item, field);
    if (value === null || value === false) storage.removeItem(key);
    else storage.setItem(key, String(value));
    return true;
  } catch { return false; }
}

export function isAttentionPreferenceEvent(key: string | null): boolean {
  return key === null || key.startsWith(PREFIX);
}
