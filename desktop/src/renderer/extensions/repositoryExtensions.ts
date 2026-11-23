import { useCallback, useEffect, useRef, useState } from "react";
import type { GatewayRequest } from "../work/notionHub";
import { extensionPanels, type ExtensionPanelId } from "./panelDefinitions";
import { readCatalog, reconcileCatalog, type ExtensionChange, type ExtensionManifest } from "./extensionState";

export const extensionMethod = "akra/kernel/extensions/call";
export const extensionEvent = "akra/kernel/extensions/changed";
export type ExtensionCall = <T>(action: string, input?: unknown, idempotencyKey?: string) => Promise<T>;
export type ChangeSubscription = (handler: (kernel: string, change?: ExtensionChange) => void) => () => void;

export function useRepositoryExtensions(request: GatewayRequest, subscribe: ChangeSubscription, kernel: string | null) {
  const [epoch, invalidate] = useState(0);
  const [catalog, setCatalog] = useState<{ kernel: string; items: ExtensionManifest[]; error: string } | null>(null);
  const [pending, setPending] = useState<readonly string[]>([]);
  const lifetime = useRef({ kernel, serial: 0 });
  if (lifetime.current.kernel !== kernel) lifetime.current = { kernel, serial: lifetime.current.serial + 1 };
  const invoke = useCallback(<T,>(extension: string, action: string, input: unknown = {}, idempotencyKey?: string): Promise<T> => {
    if (!kernel) return Promise.reject(new Error("확장 호스트에 연결되어 있지 않습니다."));
    const version = extensionPanels.find(panel => panel.id === extension)?.version ?? 1;
    return request(extensionMethod, { kernel, extension, version, action, input, ...(idempotencyKey ? { idempotency_key: idempotencyKey } : {}) });
  }, [request, kernel]);
  useEffect(() => {
    if (!kernel) return;
    const refresh = () => invalidate(value => value + 1);
    const unsubscribe = subscribe(changed => { if (changed === kernel) refresh(); });
    window.addEventListener("focus", refresh);
    return () => { unsubscribe(); window.removeEventListener("focus", refresh); };
  }, [kernel, subscribe]);
  useEffect(() => {
    if (!kernel) { setCatalog(null); setPending([]); return; }
    let active = true;
    request<unknown>("akra/kernel/extensions/list", { kernel }).then(
      value => {
        if (!active) return;
        try {
          const items = readCatalog(value);
          setCatalog(previous => ({ kernel, items: reconcileCatalog(previous?.kernel === kernel ? previous.items : [], items), error: "" }));
        } catch (error) { setCatalog(previous => ({ kernel, items: previous?.kernel === kernel ? previous.items : [], error: String(error) })); }
      },
      error => { if (active) setCatalog(previous => ({ kernel, items: previous?.kernel === kernel ? previous.items : [], error: String(error) })); },
    );
    return () => { active = false; };
  }, [kernel, request, epoch]);
  const current = catalog?.kernel === kernel ? catalog : null;
  const configure = async (id: string, enabled: boolean) => {
    const scope = lifetime.current;
    const item = current?.items.find(value => value.id === id);
    if (!item || current?.error) throw new Error("확장 상태를 다시 확인하세요.");
    setPending(values => [...values, id]);
    try {
      await invoke(id, "configure", { enabled, expected_revision: item.revision });
      if (lifetime.current !== scope) throw new Error("변경한 범위가 바뀌었습니다. 현재 호스트 상태를 확인하세요.");
    } finally {
      if (lifetime.current === scope) {
        setPending(values => values.filter(value => value !== id));
        invalidate(value => value + 1);
      }
    }
  };
  const enabledIds = (current?.items ?? []).filter(item => item.enabled && item.compatible && item.lifecycle === "enabled"
    && extensionPanels.some(panel => panel.id === item.id && panel.version === item.version && panel.scope === item.scope)).map(item => item.id);
  return { kernel, invoke, epoch, items: current?.items ?? [], enabledIds, ready: !!current && !current.error,
    error: current?.error, pending, configure, refresh: () => invalidate(value => value + 1) };
}
export type RepositoryExtensions = ReturnType<typeof useRepositoryExtensions>;
export function useExtensionCall(extensions: RepositoryExtensions, id: ExtensionPanelId): ExtensionCall {
  return useCallback((action, input, key) => extensions.invoke(id, action, input, key), [extensions.invoke, id]);
}

/** 저장소 attach를 바꾸지 않는 별도 사용자 호스트 연결이다. 연결과 목록 조회는 스캔을 시작하지 않는다. */
export function useLocalUserExtensions(request: GatewayRequest, subscribe: ChangeSubscription, connected: boolean, endpoint: string) {
  const [attachment, setAttachment] = useState<{ kernel: string; generation: string | number; endpoint: string } | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    setAttachment(null); setError("");
    if (!connected) return;
    let active = true;
    let attached: string | null = null;
    void request<{ kernel: string; generation: string | number }>("akra/kernel/extensions/attach", { scope: "local_user", client_name: "Akra Desktop" }).then(value => {
      if (!value || typeof value.kernel !== "string") throw new Error("로컬 사용자 확장 호스트를 사용할 수 없습니다.");
      attached = value.kernel;
      if (active) setAttachment({ ...value, endpoint });
      else void request("akra/kernel/extensions/detach", { kernel: value.kernel }).catch(() => undefined);
    }).catch(failure => { if (active) setError(String(failure)); });
    return () => {
      active = false;
      if (attached) void request("akra/kernel/extensions/detach", { kernel: attached }).catch(() => undefined);
    };
  }, [request, connected, endpoint]);
  const extensions = useRepositoryExtensions(request, subscribe, connected && attachment?.endpoint === endpoint ? attachment.kernel : null);
  return { ...extensions, error: error || extensions.error, generation: attachment?.generation ?? 0 };
}
