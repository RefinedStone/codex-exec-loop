import { useCallback, useEffect, useState } from "react";
import type { GatewayRequest } from "../work/notionHub";

import { extensionPanels, type ExtensionPanelId } from "./registry";
type Manifest = { id: string; version: number; scope: string; enabled: boolean };
export const extensionMethod = "akra/kernel/extensions/call";
export const extensionEvent = "akra/kernel/extensions/changed";

export type ExtensionCall = <T>(action: string, input?: unknown) => Promise<T>;
export type ChangeSubscription = (handler: (kernel: string) => void) => () => void;

export function useRepositoryExtensions(request: GatewayRequest, subscribe: ChangeSubscription, kernel: string | null) {
  const [epoch, invalidate] = useState(0);
  const [catalog, setCatalog] = useState<{ kernel: string; items: Manifest[]; error: string } | null>(null);
  const invoke = useCallback(<T,>(extension: string, action: string, input: unknown = {}): Promise<T> => {
    if (!kernel) return Promise.reject(new Error("저장소를 먼저 선택하세요."));
    const version = extensionPanels.find(p => p.id === extension)?.version ?? 1;
    return request(extensionMethod, { kernel, extension, version, action, input });
  }, [request, kernel]);
  useEffect(() => {
    if (!kernel) return;
    const refresh = () => invalidate(n => n + 1);
    const unsubscribe = subscribe(changed => { if (changed === kernel) refresh(); });
    window.addEventListener("focus", refresh);
    return () => { unsubscribe(); window.removeEventListener("focus", refresh); };
  }, [kernel, subscribe]);
  useEffect(() => {
    if (!kernel) { setCatalog(null); return; }
    let active = true;
    request<Manifest[]>("akra/kernel/extensions/list", { kernel }).then(
      value => {
        if (!active) return;
        if (!Array.isArray(value)) {
          setCatalog({ kernel, items: [], error: "Gateway의 확장 목록을 읽을 수 없습니다." });
          return;
        }
        const items = value.filter(item => item && typeof item.id === "string" && typeof item.enabled === "boolean");
        setCatalog({ kernel, items, error: "" });
      },
      err => { if (active) setCatalog({ kernel, items: [], error: String(err) }); },
    );
    return () => { active = false; };
  }, [kernel, request, epoch]);
  const configure = async (id: string, enabled: boolean) => {
    await invoke(id, "configure", { enabled });
    invalidate(n => n + 1);
  };
  const current = catalog && catalog.kernel === kernel ? catalog : null;
  const enabledIds = (current?.items ?? []).filter(i => i.enabled && extensionPanels.some(p => p.id === i.id && p.version === i.version)).map(i=>i.id);
  return { kernel, invoke, epoch, items: current?.items ?? [], enabledIds, ready: !!current && !current.error,
    error: current?.error, configure };
}
export type RepositoryExtensions = ReturnType<typeof useRepositoryExtensions>;
export function useExtensionCall(extensions: RepositoryExtensions, id: ExtensionPanelId): ExtensionCall {
  return useCallback((action, input) => extensions.invoke(id, action, input), [extensions.invoke, id]);
}
