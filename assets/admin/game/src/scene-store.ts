import type {
  DashboardSceneSnapshot,
  GameActorProjection,
  GameSceneProjection,
  GameStandbyProjection,
  GameValidationProjection,
  StaticPose,
  StatusSeverity,
  VisualState,
} from "./game-types";

type SceneListener = (snapshot: DashboardSceneSnapshot) => void;

const asRecord = (value: unknown): Record<string, unknown> | null =>
  typeof value === "object" && value !== null ? (value as Record<string, unknown>) : null;

const asString = (value: unknown, fallback = ""): string =>
  typeof value === "string" ? value : fallback;

const asNumber = (value: unknown, fallback: number): number =>
  typeof value === "number" && Number.isFinite(value) ? value : fallback;

const asVisualState = (value: unknown): VisualState | null => {
  if (
    value === "idle" ||
    value === "starting" ||
    value === "working" ||
    value === "awaiting_review" ||
    value === "blocked" ||
    value === "delivering" ||
    value === "cleanup"
  ) {
    return value;
  }
  return null;
};

const asStaticPose = (value: unknown): StaticPose =>
  value === "neutral" ||
  value === "laptop" ||
  value === "callout" ||
  value === "alert" ||
  value === "sit"
    ? value
    : "neutral";

const asSeverity = (value: unknown): StatusSeverity =>
  value === "normal" ||
  value === "success" ||
  value === "warning" ||
  value === "danger" ||
  value === "info" ||
  value === "muted"
    ? value
    : "normal";

const parseActor = (value: unknown): GameActorProjection | null => {
  const actor = asRecord(value);
  if (!actor) return null;
  const actorId = asString(actor.actorId);
  const agentId = asString(actor.agentId);
  const slotId = asString(actor.slotId);
  const visualState = asVisualState(actor.visualState);
  if (!actorId || !agentId || !slotId || !visualState) return null;
  return {
    actorId,
    agentId,
    taskId: asString(actor.taskId),
    slotId,
    seatIndex: Math.max(1, Math.trunc(asNumber(actor.seatIndex, 1))),
    displayName: asString(actor.displayName, agentId),
    archetypeKey: asString(actor.archetypeKey),
    roleLabel: asString(actor.roleLabel, "Agent"),
    visualState,
    staticPose: asStaticPose(actor.staticPose),
    severity: asSeverity(actor.severity),
    statusLabel: asString(actor.statusLabel),
    taskTitle: asString(actor.taskTitle),
    bubbleLabel: asString(actor.bubbleLabel),
  };
};

const parseStandby = (value: unknown): GameStandbyProjection | null => {
  const character = asRecord(value);
  if (!character) return null;
  const characterId = asString(character.characterId);
  const agentId = asString(character.agentId);
  const visualState = asVisualState(character.visualState);
  if (!characterId || !agentId || !visualState) return null;
  return {
    characterId,
    agentId,
    locationIndex: Math.max(1, Math.trunc(asNumber(character.locationIndex, 1))),
    displayName: asString(character.displayName, agentId),
    archetypeKey: asString(character.archetypeKey),
    roleLabel: asString(character.roleLabel, "Agent"),
    presenceKind: "configured_standby",
    visualState,
    staticPose: asStaticPose(character.staticPose),
    severity: asSeverity(character.severity),
    statusLabel: asString(character.statusLabel),
    bubbleLabel: asString(character.bubbleLabel),
  };
};

const emptyValidation = (): GameValidationProjection => ({
  stationState: "idle",
  severity: "muted",
  label: "QA/CI · 관찰 없음",
  recordKey: null,
  phase: null,
  packetKind: null,
  workerLeaseActive: false,
});

const parseValidation = (value: unknown): GameValidationProjection => {
  const validation = asRecord(value);
  if (!validation) return emptyValidation();
  const nullableString = (candidate: unknown): string | null => {
    const parsed = asString(candidate).trim();
    return parsed === "" ? null : parsed;
  };
  return {
    stationState: asString(validation.stationState, "idle"),
    severity: asSeverity(validation.severity),
    label: asString(validation.label, "QA/CI · 관찰 없음"),
    recordKey: nullableString(validation.recordKey),
    phase: nullableString(validation.phase),
    packetKind: nullableString(validation.packetKind),
    workerLeaseActive: validation.workerLeaseActive === true,
  };
};

const parseScene = (value: unknown): GameSceneProjection | null => {
  const scene = asRecord(value);
  if (!scene) return null;
  const actors = Array.isArray(scene.actors)
    ? scene.actors.map(parseActor).filter((actor): actor is GameActorProjection => actor !== null)
    : [];
  const standbyCharacters = Array.isArray(scene.standbyCharacters)
    ? scene.standbyCharacters
        .map(parseStandby)
        .filter((character): character is GameStandbyProjection => character !== null)
    : [];
  return { actors, standbyCharacters, validation: parseValidation(scene.validation) };
};

export class DashboardSceneStore {
  private snapshot: DashboardSceneSnapshot = {
    planningRevision: null,
    scene: { actors: [], standbyCharacters: [], validation: emptyValidation() },
  };

  private signature = "";
  private readonly listeners = new Set<SceneListener>();

  current(): DashboardSceneSnapshot {
    return this.snapshot;
  }

  applyDashboard(value: unknown): boolean {
    const dashboard = asRecord(value);
    if (!dashboard) return false;
    const scene = parseScene(dashboard.scene);
    if (!scene) return false;
    const planningRevision =
      typeof dashboard.planningRevision === "number" && Number.isFinite(dashboard.planningRevision)
        ? dashboard.planningRevision
        : null;
    const next: DashboardSceneSnapshot = { planningRevision, scene };
    const nextSignature = JSON.stringify(next);
    if (nextSignature === this.signature) return false;
    this.signature = nextSignature;
    this.snapshot = next;
    for (const listener of this.listeners) listener(next);
    return true;
  }

  subscribe(listener: SceneListener): () => void {
    this.listeners.add(listener);
    listener(this.snapshot);
    return () => this.listeners.delete(listener);
  }
}
