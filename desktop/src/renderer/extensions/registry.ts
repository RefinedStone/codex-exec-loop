import { lazy } from "react";

export const extensionPanels = [{
  id: "akra.tech-blog", version: 2, label: "기술 블로그", icon: "book", testId: "tech-blog-toggle",
  Panel: lazy(() => import("./TechBlogPanel")),
}] as const;
export type ExtensionPanelId = typeof extensionPanels[number]["id"];
export const panelForExtension = (id: string | null) => extensionPanels.find(p => p.id === id);
