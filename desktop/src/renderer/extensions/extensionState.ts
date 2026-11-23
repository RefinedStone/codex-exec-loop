export interface ExtensionManifest {
  id: string;
  version: number;
  scope: "repository" | "local_user";
  label?: string;
  enabled: boolean;
  desired_enabled: boolean;
  compatible: boolean;
  compatibility_reason?: string | null;
  lifecycle: "enabled" | "disabled" | "disabling" | "error";
  revision: number;
  generation: string | number;
  error?: string | null;
}
export interface ExtensionChange {
  kernel: string;
  extension?: string;
  generation?: string | number;
  revision?: number;
  sequence?: number;
  job_id?: string;
}
export function readCatalog(value: unknown): ExtensionManifest[] {
  if (!Array.isArray(value)) throw new Error("Gateway의 확장 목록을 읽을 수 없습니다.");
  return value.map(item => {
    if (!item || typeof item.id !== "string" || typeof item.enabled !== "boolean" || !Number.isInteger(item.version)) throw new Error("확장 목록의 형식이 맞지 않습니다.");
    if ((item.scope !== undefined && item.scope !== "repository" && item.scope !== "local_user")
      || item.version < 1
      || (item.revision !== undefined && (!Number.isSafeInteger(item.revision) || item.revision < 0))
      || (item.lifecycle !== undefined && !["enabled", "disabled", "disabling", "error"].includes(item.lifecycle))
      || (item.generation !== undefined && typeof item.generation !== "string" && !(Number.isSafeInteger(item.generation) && item.generation >= 0))) {
      throw new Error("확장 범위와 상태 형식이 맞지 않습니다.");
    }
    return {
      ...item,
      scope: item.scope === "local_user" ? "local_user" : "repository",
      desired_enabled: typeof item.desired_enabled === "boolean" ? item.desired_enabled : item.enabled,
      compatible: item.compatible !== false,
      lifecycle: item.lifecycle ?? (item.enabled ? "enabled" : "disabled"),
      revision: Number.isSafeInteger(item.revision) ? item.revision : 0,
      generation: item.generation ?? 0,
    } as ExtensionManifest;
  });
}
/** 같은 호스트 세대에서 늦은 목록/명령 응답은 확인한 revision을 되돌릴 수 없다. */
export function reconcileCatalog(previous: readonly ExtensionManifest[], incoming: readonly ExtensionManifest[]): ExtensionManifest[] {
  return incoming.map(next => {
    const old = previous.find(item => item.id === next.id);
    return old && old.generation === next.generation && old.revision > next.revision ? old : next;
  });
}
