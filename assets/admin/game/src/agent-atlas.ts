import { Rectangle, Texture } from "pixi.js";
import type {
  ArchetypeKey,
  Facing,
  StaticPose,
  VisualState,
} from "./game-types";
import {
  AGENT_FRAME_HEIGHT,
  AGENT_FRAME_WIDTH,
  ARCHETYPE_BY_PROFILE,
} from "./scene-config";

export interface AgentFrameSet {
  down: Texture[];
  side: Texture[];
  up: Texture[];
}

export interface ResolvedAgentTexture {
  texture: Texture | null;
  resolvedAtlasFrameIndex: number | null;
  poseFallback: boolean;
}

const STATIC_POSE_MANIFEST: Record<ArchetypeKey, Record<StaticPose, number | null>> = {
  planner: { neutral: null, laptop: 40, callout: null, alert: 41, sit: null },
  coffee_addict: { neutral: null, laptop: 44, callout: null, alert: 45, sit: 47 },
  ai_researcher: { neutral: null, laptop: 48, callout: null, alert: null, sit: null },
  designer: { neutral: null, laptop: null, callout: 50, alert: 49, sit: null },
};

const makeAtlasFrame = (texture: Texture, col: number, row: number): Texture =>
  new Texture({
    source: texture.source,
    frame: new Rectangle(
      col * AGENT_FRAME_WIDTH,
      row * AGENT_FRAME_HEIGHT,
      AGENT_FRAME_WIDTH,
      AGENT_FRAME_HEIGHT
    ),
  });

export const makeAtlasFrameByIndex = (texture: Texture, atlasFrameIndex: number): Texture =>
  makeAtlasFrame(texture, atlasFrameIndex % 8, Math.floor(atlasFrameIndex / 8));

const makeFrameRow = (texture: Texture, row: number, startCol: number): Texture[] =>
  Array.from({ length: 4 }, (_, index) => makeAtlasFrame(texture, startCol + index, row));

export const buildAgentFrameSets = (texture: Texture): Record<ArchetypeKey, AgentFrameSet> => ({
  planner: {
    down: makeFrameRow(texture, 0, 0),
    side: makeFrameRow(texture, 1, 0),
    up: makeFrameRow(texture, 2, 0),
  },
  coffee_addict: {
    down: makeFrameRow(texture, 0, 4),
    side: makeFrameRow(texture, 1, 4),
    up: makeFrameRow(texture, 2, 4),
  },
  ai_researcher: {
    down: makeFrameRow(texture, 3, 0),
    side: makeFrameRow(texture, 4, 0),
    up: makeFrameRow(texture, 4, 0),
  },
  designer: {
    down: makeFrameRow(texture, 3, 4),
    side: makeFrameRow(texture, 4, 4),
    up: makeFrameRow(texture, 4, 4),
  },
});

export const archetypeForProfile = (profile: string): ArchetypeKey =>
  ARCHETYPE_BY_PROFILE[profile] ?? "coffee_addict";

export const frameForFacing = (
  frames: Record<ArchetypeKey, AgentFrameSet>,
  archetype: ArchetypeKey,
  facing: Facing,
  frameIndex: number
): Texture => {
  const row = frames[archetype][facing];
  return row[frameIndex % row.length] ?? row[0];
};

export const resolveRestTexture = (
  atlas: Texture,
  frames: Record<ArchetypeKey, AgentFrameSet>,
  archetype: ArchetypeKey,
  visualState: VisualState,
  pose: StaticPose
): ResolvedAgentTexture => {
  // At a workstation, the generated map expects an upright rear-facing sprite behind the desk.
  // The legacy laptop emotes include their own laptop and would visually duplicate the furniture.
  if (visualState === "working" || visualState === "starting") {
    return {
      texture: frameForFacing(frames, archetype, "up", 0),
      resolvedAtlasFrameIndex: null,
      poseFallback: pose !== "neutral",
    };
  }
  const atlasFrameIndex = STATIC_POSE_MANIFEST[archetype][pose];
  if (atlasFrameIndex !== null) {
    return {
      texture: makeAtlasFrameByIndex(atlas, atlasFrameIndex),
      resolvedAtlasFrameIndex: atlasFrameIndex,
      poseFallback: false,
    };
  }
  const facing: Facing =
    visualState === "awaiting_review" || visualState === "delivering" ? "side" : "down";
  return {
    texture: frameForFacing(frames, archetype, facing, 0),
    resolvedAtlasFrameIndex: null,
    poseFallback: pose !== "neutral",
  };
};

export { STATIC_POSE_MANIFEST };
