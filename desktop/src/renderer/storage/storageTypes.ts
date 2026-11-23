export const STORAGE_EXTENSION = "akra.storage-analysis";
export type ByteString = string;
export interface StorageVolume { id: string; label: string; root_path: string; kind: string; supported: boolean; }
export interface StorageIssue { code: string; message: string; count: number; }
export interface StorageRatio { numerator_bytes: ByteString | null; denominator_bytes: ByteString | null; reason: string | null; provisional: boolean; }
export interface StorageNode {
  snapshot_id: string; revision: number; id: string; parent_id: string | null; name: string;
  kind: "directory" | "file" | "direct_files" | "shared_files";
  allocated_bytes: ByteString | null; known_allocated_bytes: ByteString; logical_bytes: ByteString;
  ratio: StorageRatio; state: "complete" | "partial" | "pending"; issues: StorageIssue[];
  protection: { code: string; detail: string; observed_at: string | number }[];
  classification: { kind: string; evidence: string }[];
  allocation_owner_id?: string | null;
}
export interface StorageSnapshot {
  schema_version: number; id: string; revision: number; volume_id: string; root_node_id: string;
  state: "running" | "complete" | "partial" | "cancelled" | "failed" | "interrupted";
  started_at: string | number; finished_at: string | number | null; files_scanned: number; directories_scanned: number; issues: StorageIssue[];
}
export interface StoragePage<T = StorageNode> { snapshot_id: string; revision: number; items: T[]; total_count: number | null; next_cursor: string | null; }
export interface StorageHotspot { node: StorageNode; role: "folder" | "remainder"; reason: string; included_bytes: string; provisional: boolean; }
export interface ExtensionJob {
  id: string; extension: string; action: string;
  state: "queued" | "running" | "cancelling" | "completed" | "cancelled" | "failed" | "interrupted";
  generation: string | number; revision: number; sequence: number; progress: unknown;
  result: StorageSnapshot | null; error: string | null; cleanup: string;
  created_at_ms: number; updated_at_ms: number;
}
export interface SharePreview {
  id: string; snapshot_id: string; revision: number; node_id: string; thread_id: string;
  root: StorageNode; shared_fields: string[]; expires_at_ms: number;
}
export interface StorageGrant {
  id: string; host_id: string; thread_id: string; snapshot_id: string; revision: number; root_node_id: string;
  expires_at_ms: number; reference: string; prompt: string;
}
export interface StorageEvidence {
  snapshot_id: string; revision: number; node_id: string; ancestors: StorageNode[]; node: StorageNode;
}
export const jobLabels: Record<ExtensionJob["state"], string> = {
  queued: "대기", running: "분석 중", cancelling: "취소 중", completed: "분석 완료",
  cancelled: "취소됨", failed: "실패", interrupted: "호스트 재시작으로 중단됨",
};
export function isActiveJob(job: ExtensionJob | null): boolean {
  return !!job && ["queued", "running", "cancelling"].includes(job.state);
}
export const completenessLabels = { complete: "확인 완료", partial: "일부 확인", pending: "대기", running: "확인 중", cancelled: "일부 확인", failed: "일부 확인", interrupted: "일부 확인" } as const;
