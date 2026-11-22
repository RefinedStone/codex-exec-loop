// native helper 전용 wire 검증이다. 일반 Gateway나 로그에 복원본을 전달하지 않는다.
import {
  isReconnectableHold, type VaultReconnectReview, type VaultRestoreHold,
} from "../shared/vaultRestore";
import type { VaultHeldReply } from "./vaultRestoreSession";

export type NativeVaultReply =
  | { kind: "prepared"; capabilities: { reconnect: boolean } }
  | { kind: "restored"; title: string; body: string }
  | { kind: "review"; review: VaultReconnectReview }
  | { kind: "reconnected" }
  | VaultHeldReply;

const HELPER_HOLDS = new Set<VaultRestoreHold>([
  "source_unavailable", "platform_unsupported", "confirmation_cancelled", "locked",
  "latest_read_failed", "not_bound", "published_copy_changed", "marker_moved",
  "marker_duplicated", "marker_missing", "changed_during_review", "mapping_rejected",
  "record_unavailable", "write_outcome_unknown",
]);

export function parseNativeVaultReply(value: unknown, id: string): NativeVaultReply | null {
  if (!record(value) || !fields(value, ["version", "id", "result"]) ||
    value.version !== 2 || value.id !== id || !record(value.result)) return null;
  const result = value.result;
  if (result.kind === "prepared" && fields(result, ["kind", "capabilities"]) &&
    record(result.capabilities) && fields(result.capabilities, ["reconnect"]) &&
    typeof result.capabilities.reconnect === "boolean") {
    return { kind: "prepared", capabilities: { reconnect: result.capabilities.reconnect } };
  }
  if (result.kind === "held" && fields(result, ["kind", "reason", "can_reconnect"]) &&
    typeof result.reason === "string" && HELPER_HOLDS.has(result.reason as VaultRestoreHold) &&
    typeof result.can_reconnect === "boolean") {
    const reason = result.reason as VaultRestoreHold;
    if (result.can_reconnect && !isReconnectableHold(reason)) return null;
    return { kind: "held", reason, ...(result.can_reconnect ? { canReconnect: true } : {}) };
  }
  if (result.kind === "restored" && fields(result, ["kind", "title", "body"]) &&
    text(result.title) && text(result.body)) {
    return { kind: "restored", title: result.title, body: result.body };
  }
  if (result.kind === "reconnected" && fields(result, ["kind"])) return { kind: "reconnected" };
  if (result.kind === "review" && fields(result, ["kind", "review"])) {
    const review = parseReview(result.review);
    return review ? { kind: "review", review } : null;
  }
  return null;
}

function parseReview(value: unknown): VaultReconnectReview | null {
  if (!record(value) || !fields(value, ["reviewed_hash", "published_title", "published_body", "markers"]) ||
    !canonicalHash(value.reviewed_hash) || !text(value.published_title) ||
    !text(value.published_body) || !Array.isArray(value.markers)) return null;
  const bytes = { title: Buffer.from(value.published_title, "utf8"), body: Buffer.from(value.published_body, "utf8") };
  const markers: VaultReconnectReview["markers"] = [];
  const seen = new Set<string>();
  for (const marker of value.markers) {
    if (!record(marker) || !fields(marker, ["marker", "previous", "candidates"]) ||
      !text(marker.marker) || marker.marker.length === 0 || seen.has(marker.marker) ||
      !place(marker.previous) || !Array.isArray(marker.candidates)) return null;
    seen.add(marker.marker);
    const markerBytes = Buffer.from(marker.marker, "utf8");
    const candidates: VaultReconnectReview["markers"][number]["candidates"] = [];
    const places = new Set<string>();
    for (const candidate of marker.candidates) {
      if (!record(candidate) || !fields(candidate, ["field", "position", "context"]) ||
        !coordinate(candidate) || !text(candidate.context)) return null;
      const key = candidate.field + ":" + candidate.position;
      const content = bytes[candidate.field];
      if (places.has(key) || candidate.position > content.length ||
        markerBytes.length > content.length - candidate.position ||
        !content.subarray(candidate.position, candidate.position + markerBytes.length).equals(markerBytes)) return null;
      places.add(key);
      candidates.push({ field: candidate.field, position: candidate.position, context: candidate.context });
    }
    markers.push({
      marker: marker.marker,
      // 이전 판의 위치는 지금 게시본의 길이와 무관하다.
      previous: { field: marker.previous.field, position: marker.previous.position },
      candidates,
    });
  }
  return {
    reviewedHash: value.reviewed_hash, publishedTitle: value.published_title,
    publishedBody: value.published_body, markers,
  };
}

function canonicalHash(value: unknown): value is string {
  if (typeof value !== "string" || !/^[1-9]\d{0,9}:[0-9a-f]{64}$/.test(value)) return false;
  return Number(value.slice(0, value.indexOf(":"))) <= 0xffff_ffff;
}
function coordinate(value: Record<string, unknown>): value is Record<string, unknown> & { field: "title" | "body"; position: number } {
  return (value.field === "title" || value.field === "body") &&
    typeof value.position === "number" && Number.isSafeInteger(value.position) && value.position >= 0;
}
function place(value: unknown): value is { field: "title" | "body"; position: number } {
  return record(value) && fields(value, ["field", "position"]) && coordinate(value);
}
function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function fields(value: Record<string, unknown>, expected: readonly string[]): boolean {
  const keys = Object.keys(value);
  return keys.length === expected.length && keys.every((key) => expected.includes(key));
}
function text(value: unknown): value is string {
  return typeof value === "string" && Buffer.from(value, "utf8").toString("utf8") === value;
}
