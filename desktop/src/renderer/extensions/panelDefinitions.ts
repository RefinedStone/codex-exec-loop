
/** 화면 ID의 유일한 정의. manifest는 실행 코드 경로를 지정하지 않는다. */
export const corePanels = [
  { id: "review", label: "변경", icon: "diff", testId: "review-toggle" },
  { id: "graph", label: "그래프", icon: "branch", testId: "git-graph-toggle" },
  { id: "kernel", label: "Akra", icon: "bolt", testId: "kernel-toggle" },
  { id: "issues", label: "이슈", icon: "eye", testId: "issues-toggle" },
  { id: "browser", label: "브라우저", icon: "globe", testId: "browser-toggle" },
  { id: "delegation", label: "위임", icon: "message", testId: "delegation-toggle" },
  { id: "work", label: "업무", icon: "briefcase", testId: "work-toggle" },
  { id: "docs", label: "문서", icon: "book", testId: "documents-toggle" },
] as const satisfies readonly { id: string; label: string; icon: string; testId: string }[];

export const extensionPanels = [
  { id: "akra.tech-blog", version: 3, scope: "repository", label: "기술 블로그", icon: "book", testId: "tech-blog-toggle" },
  { id: "akra.storage-analysis", version: 1, scope: "local_user", label: "저장 공간", icon: "folder", testId: "storage-toggle" },
] as const satisfies readonly { id: string; version: number; scope: string; label: string; icon: string; testId: string }[];

export const lazyCorePanels = [
  { id: "skills", label: "스킬", icon: "stack", testId: "skills-toggle" },
] as const satisfies readonly { id: string; label: string; icon: string; testId: string }[];

export const panels = [...corePanels, ...extensionPanels, ...lazyCorePanels] as const;
export type PanelId = typeof panels[number]["id"];
export type ExtensionPanelId = typeof extensionPanels[number]["id"];
export type RegisteredPanelId = ExtensionPanelId | typeof lazyCorePanels[number]["id"];
export const panelForExtension = (id: string | null) => extensionPanels.find(panel => panel.id === id);
export const registeredPanel = (id: string | null) => [...extensionPanels, ...lazyCorePanels].find(panel => panel.id === id);
export const isPanelId = (id: unknown): id is PanelId => typeof id === "string" && panels.some(panel => panel.id === id);
