import { create } from "zustand";
import { isManifestId } from "./attachmentManifest";
import { ATTACHMENT_LIMIT, AttachmentError } from "./fileAttachment";

type Receipt = Readonly<{
  scope: string;
  requestId: number;
  manifestIds: readonly string[];
  turnId: string | null;
}>;
type StoredReceipt = Readonly<{ key: string; receipt: Receipt }>;

const prefix = "akra.attachment-receipt.v1:";
const readReceipts = (): readonly StoredReceipt[] => {
  if (typeof window === "undefined") return [];
  let storage: Storage;
  try {
    storage = window.localStorage;
  } catch (error) {
    if (error instanceof DOMException) return [];
    throw error;
  }
  return Object.keys(storage).filter(key => key.startsWith(prefix)).flatMap(key => {
    try {
      const value: unknown = JSON.parse(storage.getItem(key) ?? "null");
      if (!value || typeof value !== "object" || !("scope" in value) || typeof value.scope !== "string"
        || !("requestId" in value) || typeof value.requestId !== "number" || !Number.isSafeInteger(value.requestId)
        || !("manifestIds" in value) || !Array.isArray(value.manifestIds) || !value.manifestIds.length
        || value.manifestIds.length > ATTACHMENT_LIMIT.count || !value.manifestIds.every(isManifestId)
        || !("turnId" in value) || (value.turnId !== null && typeof value.turnId !== "string")) return [];
      const { scope, requestId, manifestIds, turnId } = value;
      return [{ key, receipt: { scope, requestId, manifestIds, turnId } }];
    } catch (error) {
      if (error instanceof SyntaxError) return [];
      throw error;
    }
  }).sort((a, b) => b.key.localeCompare(a.key));
};

export const useAttachmentReceipts = create<{ readonly records: readonly StoredReceipt[] }>(
  () => ({ records: readReceipts() }),
);
if (typeof window !== "undefined") window.addEventListener("storage", (event) => {
  if (event.key === null || event.key.startsWith(prefix)) useAttachmentReceipts.setState({ records: readReceipts() });
});

/** The durable initial state is deliberately unknown. Only a correlated RPC result can replace it. */
export function recordAttachmentRequest(scope: string, manifestIds: readonly string[], requestId: number) {
  if (!manifestIds.length || manifestIds.length > ATTACHMENT_LIMIT.count || !manifestIds.every(isManifestId)) {
    throw new AttachmentError("첨부의 manifest ID를 확인할 수 없어 전송하지 않았습니다.");
  }
  const key = `${prefix}${Date.now()}:${crypto.randomUUID()}`;
  const write = (turnId: string | null) => {
    const receipt = { scope, requestId, manifestIds, turnId };
    try {
      window.localStorage.setItem(key, JSON.stringify(receipt));
    } catch (error) {
      if (!(error instanceof DOMException)) throw error;
      throw new AttachmentError(turnId === null
        ? "첨부 접수 기록을 저장할 수 없어 전송하지 않았습니다. 초안을 보존했습니다."
        : "턴이 접수됐지만 로컬 기록을 갱신하지 못했습니다. 다시 보내기 전에 대화 기록을 확인하세요.");
    }
    const records = useAttachmentReceipts.getState().records;
    useAttachmentReceipts.setState({ records: [{ key, receipt }, ...records.filter(record => record.key !== key)] });
  };
  write(null);
  return (turnId: string) => write(turnId);
}
