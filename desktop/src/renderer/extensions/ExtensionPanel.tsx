import { Suspense } from "react";
import { extensionPanels, type ExtensionPanelId } from "./registry";
import { useExtensionCall, type RepositoryExtensions } from "./repositoryExtensions";

export function ExtensionPanel({id, extensions}:{id: ExtensionPanelId; extensions: RepositoryExtensions}) {
  const call = useExtensionCall(extensions, id);
  const panel = extensionPanels.find(p => p.id === id);
  if (!panel || !extensions.enabledIds.includes(id)) return <p>저장소 확장 상태를 확인하고 있습니다.</p>;
  const Panel = panel.Panel;
  return <Suspense fallback={<p>편집기를 여는 중…</p>}><Panel call={call} epoch={extensions.epoch}/></Suspense>;
}
