type ManifestEntry = Readonly<{ kind: string; key: string; selected: boolean; reason: string; observedVersion: string | null }>;
export type ManifestLedger = Readonly<{ id: string; entries: readonly ManifestEntry[]; unverifiedCount: number }>;

const object = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null;

export function isManifestId(value: unknown): value is string {
  return typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
}

export function isManifestLedger(value: unknown): value is ManifestLedger {
  return object(value) && isManifestId(value.id) && typeof value.unverifiedCount === "number"
    && Number.isSafeInteger(value.unverifiedCount) && value.unverifiedCount >= 0
    && Array.isArray(value.entries) && value.entries.length <= 256
    && value.entries.every((entry: unknown) => object(entry) && typeof entry.kind === "string"
      && typeof entry.key === "string" && typeof entry.selected === "boolean" && typeof entry.reason === "string"
      && (entry.observedVersion === null || typeof entry.observedVersion === "string"));
}
