import { useSyncExternalStore } from "react";
export const STORAGE_EVIDENCE_EVENT = "akra:storage-evidence";
export function isStorageReference(value: unknown): value is string {
  return typeof value === "string" && value.length <= 8192 && /^akra-storage:[A-Za-z0-9_-]+$/.test(value);
}
export function requestStorageEvidence(reference: string): void {
  if (isStorageReference(reference)) window.dispatchEvent(new CustomEvent(STORAGE_EVIDENCE_EVENT, { detail: reference }));
}
export interface EvidenceRequest { reference: string; id: number; threadId: string; navigationRevision: number; }
let current: EvidenceRequest | null = null;
let sequence = 0;
const listeners = new Set<() => void>();
export function queueStorageEvidence(reference: string, threadId: string, navigationRevision: number): void {
  if (!isStorageReference(reference)) return;
  current = { reference, threadId, navigationRevision, id: ++sequence };
  for (const listener of listeners) listener();
}
export function clearStorageEvidence(id?: number): void {
  if (id !== undefined && current?.id !== id) return;
  current = null;
  for (const listener of listeners) listener();
}
export function useStorageEvidence(): EvidenceRequest | null {
  return useSyncExternalStore(listener => { listeners.add(listener); return () => { listeners.delete(listener); }; }, () => current, () => null);
}
