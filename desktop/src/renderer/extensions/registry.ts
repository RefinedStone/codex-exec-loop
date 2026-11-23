import type { ComponentProps, ComponentType } from "react";
import type { SkillsPanel } from "../skills/SkillsPanel";
import type { ExtensionCall } from "./repositoryExtensions";
import type { RegisteredPanelId } from "./panelDefinitions";

export { extensionPanels, panelForExtension, registeredPanel } from "./panelDefinitions";
export type { ExtensionPanelId } from "./panelDefinitions";

export interface RegisteredPanelProps {
  call: ExtensionCall;
  epoch: number;
  scopeKey: string;
  threadId: string | null;
  workspace: string | null;
  enabled: boolean;
  skills: ComponentProps<typeof SkillsPanel>;
}

type PanelLoader = () => Promise<{ default: ComponentType<RegisteredPanelProps> }>;
/** 빌드에 포함된 신뢰된 모듈만 가져온다. 새 패널은 셸 분기 없이 여기에 등록한다. */
export const panelLoaders: Record<RegisteredPanelId, PanelLoader> = {
  "akra.tech-blog": () => import("./TechBlogPanel"),
  "akra.storage-analysis": () => import("../storage/StoragePanel"),
  skills: () => import("./SkillsRegisteredPanel"),
};
