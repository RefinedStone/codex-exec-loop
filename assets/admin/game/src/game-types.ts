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
export type PrApproverState = "idle" | "reviewing" | "failure" | "success";
export type PrApproverQualifier =
  | "none"
  | "waiting"
  | "paused"
  | "stale"
  | "recovering";
export type PrApproverClip = "idle" | "intake" | "review" | "failure" | "success";

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

export interface GameValidationProjection {
  stationState: string;
  severity: StatusSeverity;
  label: string;
  recordKey: string | null;
  phase: string | null;
  packetKind: string | null;
  workerLeaseActive: boolean;
  approver: GamePrApproverProjection;
}

export interface GamePrApproverProjection {
  state: PrApproverState;
  qualifier: PrApproverQualifier;
  recordKey: string | null;
  pullRequestNumber: number | null;
  evidenceShortSha: string | null;
  integrationMethod: string | null;
  requiredChecksSucceeded: number;
  requiredChecksTotal: number;
  findingCount: number;
  remediationCount: number;
  statusLabel: string;
  transitionKey: string;
}

export interface GameSceneProjection {
  actors: GameActorProjection[];
  standbyCharacters: GameStandbyProjection[];
  validation: GameValidationProjection;
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
  sourceFrameIndex: number | null;
  animationBlend: number;
  gaitOffsetX: number;
  gaitOffsetY: number;
  frameScale: number;
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
  ambientActivity: boolean;
  agentId: string;
  visualState: VisualState;
  pose: StaticPose;
  animationKind: AgentAnimationKind;
  animationFrameIndex: number | null;
  sourceFrameIndex: number | null;
  animationBlend: number;
  gaitOffsetX: number;
  gaitOffsetY: number;
  frameScale: number;
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
  ambientActivityCount: number;
  packetCount: number;
  semanticMotionCount: number;
  movementSpeedRatio: number;
  renderCount: number;
  planningRevision: number | null;
  zoomLevel: SemanticZoomLevel;
  cameraZoom: number;
  validation: {
    stationState: string;
    severity: StatusSeverity;
    label: string;
    recordKey: string | null;
    phase: string | null;
    packetKind: string | null;
    packetVisible: boolean;
    workerLeaseActive: boolean;
    approver: {
      state: PrApproverState;
      qualifier: PrApproverQualifier;
      clip: PrApproverClip;
      frameIndex: number;
      sourceFrameIndex: number;
      settled: boolean;
      recordKey: string | null;
      pullRequestNumber: number | null;
      transitionKey: string;
      visible: boolean;
      reducedMotion: boolean;
      displayWidth: number;
      displayHeight: number;
      boardX: number;
      boardY: number;
    };
  };
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
