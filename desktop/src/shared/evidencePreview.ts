export type EvidencePreview = {
  readonly workspace: string;
  readonly task_id: string;
  readonly criterion_id: string;
  readonly scope_id: string;
  readonly revision: number;
  readonly event_id: string;
  readonly sha256: string;
  readonly kind: "worker" | "host";
};

export function isEvidencePreview(value: unknown): value is EvidencePreview {
  if (typeof value !== "object" || value === null) return false;
  const id = (v: unknown) => typeof v === "string" && v.length > 0 && v.length <= 4096 && !/\p{Cc}/u.test(v);
  const hash = (v: unknown) => typeof v === "string" && v.length === 64 && !/[^a-f0-9]/.test(v);
  return Object.keys(value).length === 8 && "workspace" in value && id(value.workspace)
    && "task_id" in value && id(value.task_id) && "criterion_id" in value && id(value.criterion_id)
    && "scope_id" in value && hash(value.scope_id) && "sha256" in value && hash(value.sha256)
    && "revision" in value && typeof value.revision === "number" && Number.isSafeInteger(value.revision) && value.revision >= 1
    && "event_id" in value && id(value.event_id) && "kind" in value && (value.kind === "worker" || value.kind === "host");
}
