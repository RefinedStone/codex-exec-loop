import type { Application } from "pixi.js";

export type StatusSeverity =
  | "normal"
  | "success"
  | "warning"
  | "danger"
  | "info"
  | "muted";

export type VisualState =
  | "idle"
  | "starting"
  | "working"
  | "awaiting_review"
  | "blocked"
  | "delivering"
  | "cleanup";

export type StaticPose = "neutral" | "laptop" | "callout" | "alert" | "sit";
export type PresenceKind = "active" | "configured_standby";
export type Facing = "down" | "side" | "up";
export type AgentAnimationKind = "rest" | "idle" | "walk" | "working" | "blocked";
export type ArchetypeKey = "planner" | "coffee_addict" | "ai_researcher" | "designer";
export type SemanticZoomLevel = "overview" | "operations" | "detail";

export interface Point {
  x: number;
  y: number;
}

export interface GameActorProjection {
  actorId: string;
  agentId: string;
  taskId: string;
  slotId: string;
  seatIndex: number;
  displayName: string;
  archetypeKey: string;
  roleLabel: string;
  visualState: VisualState;
  staticPose: StaticPose;
  severity: StatusSeverity;
  statusLabel: string;
  taskTitle: string;
  bubbleLabel: string;
}

export interface GameStandbyProjection {
  characterId: string;
  agentId: string;
  locationIndex: number;
  displayName: string;
  archetypeKey: string;
  roleLabel: string;
  presenceKind: "configured_standby";
  visualState: VisualState;
  staticPose: StaticPose;
  severity: StatusSeverity;
  statusLabel: string;
  bubbleLabel: string;
}

export interface GameSceneProjection {
  actors: GameActorProjection[];
  standbyCharacters: GameStandbyProjection[];
}

export interface DashboardSceneSnapshot {
  planningRevision: number | null;
  scene: GameSceneProjection;
}

export interface SceneInspectionActor {
  actorId: string;
  agentId: string;
  slotId: string;
  visualState: VisualState;
  pose: StaticPose;
  animationKind: AgentAnimationKind;
  animationFrameIndex: number | null;
  resolvedAtlasFrameIndex: number | null;
  poseFallback: boolean;
  displayWidth: number;
  displayHeight: number;
  opacity: number;
  boardX: number;
  boardY: number;
  x: number;
  y: number;
}

export interface SceneInspectionStandby {
  characterId: string;
  presenceKind: "configured_standby";
  agentId: string;
  visualState: VisualState;
  pose: StaticPose;
  animationKind: AgentAnimationKind;
  animationFrameIndex: number | null;
  locationIndex: number;
  resolvedAtlasFrameIndex: number | null;
  poseFallback: boolean;
  displayWidth: number;
  displayHeight: number;
  opacity: number;
  boardX: number;
  boardY: number;
  x: number;
  y: number;
}

export interface SceneInspection {
  ready: boolean;
  actorCount: number;
  characterCount: number;
  standbyCount: number;
  packetCount: number;
  semanticMotionCount: number;
  movementSpeedRatio: number;
  renderCount: number;
  planningRevision: number | null;
  zoomLevel: SemanticZoomLevel;
  cameraZoom: number;
  actors: SceneInspectionActor[];
  standbyCharacters: SceneInspectionStandby[];
}

export interface DioramaHandle {
  app: Application;
  destroy: () => void;
  applyDashboard: (dashboard: unknown) => boolean;
  rebuildAgentUnits: () => void;
  syncLayout: () => void;
  inspectScene: () => SceneInspection;
  resetCamera: () => void;
  zoomBy: (factor: number) => void;
}

export interface AkraAdminGameBridge {
  mountDiorama?: () => DioramaHandle | null;
  applyDashboard?: (dashboard: unknown) => boolean;
  inspectScene?: () => SceneInspection | null;
  resetCamera?: () => void;
  zoomBy?: (factor: number) => void;
  [key: string]: unknown;
}
