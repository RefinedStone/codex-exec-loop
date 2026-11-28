// 창마다 따로 쓰는 초안을 같은 localStorage에 남긴다. 저장은 동기식이라 창 종료 전 비동기 큐를 기다리지 않는다.
import { isFileAttachment, type FileAttachment } from "../conversation/fileAttachment";

const LEGACY_KEY = "akra.drafts";
const RECORD_PREFIX = "akra.draft-record:";
const MAX_SAVED_DRAFTS = 50;
const MAX_SAVED_DRAFT_CHARS = 20_000;

interface DraftRecord {
  storageKey: string;
  key: string;
  text: string;
  revision: number;
  attachments?: readonly FileAttachment[];
}

function legacyRecords(storage: Storage): DraftRecord[] {
  try {
    const parsed: unknown = JSON.parse(storage.getItem(LEGACY_KEY) ?? "{}");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return [];
    return Object.entries(parsed).flatMap(([key, text]) =>
      typeof text === "string" && text.length > 0 && text.length <= MAX_SAVED_DRAFT_CHARS
        ? [{ storageKey: RECORD_PREFIX + "legacy:" + encodeURIComponent(key), key, text, revision: 0 }] : []);
  } catch { return []; }
}

function readRecords(storage: Storage): DraftRecord[] {
  const result = legacyRecords(storage);
  const keys = Array.from({ length: storage.length }, (_, index) => storage.key(index))
    .filter((key): key is string => key !== null && key.startsWith(RECORD_PREFIX));
  for (const storageKey of keys) {
    try {
      const parsed: unknown = JSON.parse(storage.getItem(storageKey) ?? "null");
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) continue;
      const record = parsed as Record<string, unknown>;
      if (typeof record.key === "string" && typeof record.text === "string"
        && record.text.length <= MAX_SAVED_DRAFT_CHARS && typeof record.revision === "number"
        && Number.isSafeInteger(record.revision) && record.revision >= 0) {
        if (record.schemaVersion !== undefined && record.schemaVersion !== 1) continue;
        const attachments: unknown = record.attachments ?? [];
        if (!Array.isArray(attachments) || !attachments.every(isFileAttachment)) continue;
        result.push({ storageKey, key: record.key, text: record.text, revision: record.revision, attachments });
      }
    } catch { /* 읽던 레코드가 다른 창에서 정리되었거나 손상됐다. */ }
  }
  return result;
}

/** 관측한 버전 뒤에 쓴다. 같은 버전의 동시 쓰기는 고유 키 순서로 모든 창에서 같은 값을 고른다. */
function compareRecords(left: DraftRecord, right: DraftRecord): number {
  return left.revision - right.revision || (left.storageKey < right.storageKey ? -1 : left.storageKey > right.storageKey ? 1 : 0);
}

function latestRecords(records: readonly DraftRecord[]): Map<string, DraftRecord> {
  const latest = new Map<string, DraftRecord>();
  for (const record of records) {
    const previous = latest.get(record.key);
    if (!previous || compareRecords(previous, record) < 0) latest.set(record.key, record);
  }
  return latest;
}

function writeRecord(storage: Storage, record: DraftRecord): void {
  storage.setItem(record.storageKey, JSON.stringify({ schemaVersion: 1, key: record.key, text: record.text,
    attachments: record.attachments ?? [], revision: record.revision }));
}

/** 구 저장 형식은 가장 오래된 불변 버전이다. 동시 이전도 새 버전이나 삭제 표시를 덮지 않는다. */
function migrateLegacy(storage: Storage): void {
  for (const record of legacyRecords(storage)) {
    if (storage.getItem(record.storageKey) === null) writeRecord(storage, record);
  }
  storage.removeItem(LEGACY_KEY);
}

/** 조회도 50개로 제한한다. 쓰기가 겹쳐 정리가 아직 끝나지 않았어도 오래된 초안을 다시 보여 주지 않는다. */
export function storedDraftRecords(): readonly DraftRecord[] {
  try {
    return [...latestRecords(readRecords(window.localStorage)).values()]
      .filter((record) => record.text.length > 0 || record.attachments?.length).sort(compareRecords).slice(-MAX_SAVED_DRAFTS);
  } catch { return []; }
}

export function storedDrafts(): Record<string, string> {
  return Object.fromEntries(storedDraftRecords().map((record) => [record.key, record.text]));
}

function trimRecords(storage: Storage, records: readonly DraftRecord[]): void {
  const latest = latestRecords(records);
  const active = [...latest.values()].filter((record) => record.text.length > 0 || record.attachments?.length).sort(compareRecords);
  for (const record of active.slice(0, Math.max(0, active.length - MAX_SAVED_DRAFTS))) {
    // 오래된 이 버전만 버린다. 더 새 revision이나 같은 revision의 더 뒤인 동시 쓰기는 이 표시보다 우선한다.
    const removed = { ...record, storageKey: record.storageKey + "~", text: "", attachments: [] };
    writeRecord(storage, removed);
    latest.set(record.key, removed);
  }
  const keep = new Set([...latest.values()].map((record) => record.storageKey));
  // key는 불변 버전 하나만 가리킨다. 다른 창이 지금 쓴 새 key를 snapshot 비교 뒤 지우지 않는다.
  for (const record of records) {
    if (!keep.has(record.storageKey)) storage.removeItem(record.storageKey);
  }
}

/** 빈 값은 최신 삭제 버전으로 남겨 오래된 창의 레코드나 구 형식이 초안을 되살리지 못하게 한다. */
export function saveDraft(key: string, text: string, attachments: readonly FileAttachment[] = []): boolean {
  try {
    if (text.length > MAX_SAVED_DRAFT_CHARS) return false;
    const storage = window.localStorage;
    migrateLegacy(storage);
    const records = readRecords(storage);
    const active = [...latestRecords(records).values()].filter((record) => record.text.length || record.attachments?.length);
    // Keep the existing newest-50 text policy, but never evict a captured
    // file snapshot to make room: the old content may no longer exist on disk.
    if ((text.length || attachments.length) && active.length >= MAX_SAVED_DRAFTS && !active.some(record => record.key === key)
        && active.sort(compareRecords)[0]?.attachments?.length) return false;
    const revision = records.reduce((latest, record) => Math.max(latest, record.revision), Date.now()) + 1;
    const record: DraftRecord = {
      storageKey: RECORD_PREFIX + crypto.randomUUID(), key, revision,
      text: text.trim().length > 0 ? text : "", attachments,
    };
    writeRecord(storage, record);
    trimRecords(storage, [...records, record]);
    return true;
  } catch {
    // 저장소가 가득 찼거나 막혀도 입력은 uiStore의 메모리에 남는다.
    return false;
  }
}
