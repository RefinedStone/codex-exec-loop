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

export interface AgentFrameAlignment {
  x: number;
  y: number;
}

export interface ResolvedAgentTexture {
  texture: Texture | null;
  resolvedAtlasFrameIndex: number | null;
  poseFallback: boolean;
  frameScale: number;
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

const DEFAULT_WALK_FRAME_SOURCE_ORDER = [0, 1, 2, 3] as const;
const GUARDIAN_WALK_FRAME_SOURCE_ORDER = [0, 1, 0, 3] as const;

// The source atlas's numbered cells are not always a playback sequence. The
// Guardian walk rows mirror the pack's RPG Maker mapping: `01` is neutral,
// `02` and `04` are the alternating steps, and `03` is not part of its stable
// walk. Keep the presentation phases explicit so a source-order regression
// cannot make the character appear to walk backwards.
export const WALK_FRAME_SOURCE_ORDER: Record<
  ArchetypeKey,
  Record<Facing, readonly number[]>
> = {
  planner: {
    down: DEFAULT_WALK_FRAME_SOURCE_ORDER,
    side: DEFAULT_WALK_FRAME_SOURCE_ORDER,
    up: DEFAULT_WALK_FRAME_SOURCE_ORDER,
  },
  coffee_addict: {
    down: DEFAULT_WALK_FRAME_SOURCE_ORDER,
    side: DEFAULT_WALK_FRAME_SOURCE_ORDER,
    up: DEFAULT_WALK_FRAME_SOURCE_ORDER,
  },
  ai_researcher: {
    down: GUARDIAN_WALK_FRAME_SOURCE_ORDER,
    side: GUARDIAN_WALK_FRAME_SOURCE_ORDER,
    // The source atlas has no rear-facing Guardian row, so its calibrated
    // side fallback must retain the same verified stride order.
    up: GUARDIAN_WALK_FRAME_SOURCE_ORDER,
  },
  designer: {
    down: DEFAULT_WALK_FRAME_SOURCE_ORDER,
    side: DEFAULT_WALK_FRAME_SOURCE_ORDER,
    up: DEFAULT_WALK_FRAME_SOURCE_ORDER,
  },
};

const aligned = (
  x: readonly number[],
  y: readonly number[] = [0, 0, 0, 0]
): readonly AgentFrameAlignment[] =>
  x.map((offsetX, index) => ({ x: offsetX, y: y[index] ?? 0 }));

// Source-pixel offsets measured from each frame's alpha bounds. They keep the
// torso center and lowest visible foot aligned while the four poses cycle.
export const WALK_FRAME_ALIGNMENT: Record<
  ArchetypeKey,
  Record<Facing, readonly AgentFrameAlignment[]>
> = {
  planner: {
    down: aligned([0, -0.5, -0.5, 0]),
    side: aligned([0, 0.5, 0.5, 0.5]),
    up: aligned([0, 0, 0, 0.5]),
  },
  coffee_addict: {
    down: aligned([0, 0.5, 0.5, 0.5]),
    side: aligned([0, 0.5, 0.5, 0.5]),
    up: aligned([0, 0, 0, 0], [0, 0, -2, 0]),
  },
  ai_researcher: {
    down: aligned([0, 0.5, 0, 0], [0, -2, 0, 0]),
    side: aligned([0, 0.5, 0.5, 0.5]),
    up: aligned([0, 0.5, 0.5, 0.5]),
  },
  designer: {
    down: aligned([0, -0.5, 0, 0]),
    side: aligned([0, 0, 0, 0]),
    up: aligned([0, 0, 0, 0]),
  },
};

// The source atlas was composed from perspective-specific illustrations. Their
// opaque bounds are not a uniform height: notably, Guardian's 173px front row
// becomes a 153-158px side row. Keeping one raw scale makes an agent visibly
// shrink as it turns. These factors normalize each frame to that archetype's
// down-facing reference height while retaining the intended side silhouette.
// They are applied uniformly about the bottom anchor, so feet stay on the same
// map plane and the pixel art does not stretch.
export const WALK_FRAME_VISUAL_SCALE: Record<
  ArchetypeKey,
  Record<Facing, readonly number[]>
> = {
  planner: {
    down: [1, 1, 0.994, 0.994],
    side: [1.092, 1.099, 1.121, 1.099],
    up: [1.031, 1.037, 1.037, 1.031],
  },
  coffee_addict: {
    down: [1, 1, 1, 1],
    side: [1.149, 1.149, 1.149, 1.157],
    up: [1.029, 1.035, 1.035, 1.029],
  },
  ai_researcher: {
    down: [1, 1, 1, 1],
    side: [1.095, 1.131, 1.095, 1.131],
    // The source atlas has no rear-facing Guardian row; `up` deliberately
    // falls back to this calibrated side row.
    up: [1.095, 1.131, 1.095, 1.131],
  },
  designer: {
    down: [1, 1, 1, 1],
    side: [1.068, 1.068, 1.055, 1.062],
    // The designer source also has no separate rear-facing walk row.
    up: [1.068, 1.068, 1.055, 1.062],
  },
};

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

export const sourceFrameIndexForFacing = (
  archetype: ArchetypeKey,
  facing: Facing,
  frameIndex: number
): number => {
  const sourceOrder = WALK_FRAME_SOURCE_ORDER[archetype][facing];
  const normalizedIndex =
    ((frameIndex % sourceOrder.length) + sourceOrder.length) % sourceOrder.length;
  return sourceOrder[normalizedIndex] ?? 0;
};

export const frameForFacing = (
  frames: Record<ArchetypeKey, AgentFrameSet>,
  archetype: ArchetypeKey,
  facing: Facing,
  frameIndex: number
): Texture => {
  const row = frames[archetype][facing];
  const sourceFrameIndex = sourceFrameIndexForFacing(
    archetype,
    facing,
    frameIndex
  );
  return row[sourceFrameIndex % row.length] ?? row[0];
};

export const alignmentForFacing = (
  archetype: ArchetypeKey,
  facing: Facing,
  frameIndex: number
): AgentFrameAlignment => {
  const sourceFrameIndex = sourceFrameIndexForFacing(
    archetype,
    facing,
    frameIndex
  );
  return WALK_FRAME_ALIGNMENT[archetype][facing][sourceFrameIndex] ?? { x: 0, y: 0 };
};

export const scaleForFacing = (
  archetype: ArchetypeKey,
  facing: Facing,
  frameIndex: number
): number => {
  const sourceFrameIndex = sourceFrameIndexForFacing(
    archetype,
    facing,
    frameIndex
  );
  return WALK_FRAME_VISUAL_SCALE[archetype][facing][sourceFrameIndex] ?? 1;
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
      frameScale: scaleForFacing(archetype, "up", 0),
    };
  }
  const atlasFrameIndex = STATIC_POSE_MANIFEST[archetype][pose];
  if (atlasFrameIndex !== null) {
    return {
      texture: makeAtlasFrameByIndex(atlas, atlasFrameIndex),
      resolvedAtlasFrameIndex: atlasFrameIndex,
      poseFallback: false,
      frameScale: 1,
    };
  }
  const facing: Facing =
    visualState === "awaiting_review" || visualState === "delivering" ? "side" : "down";
  return {
    texture: frameForFacing(frames, archetype, facing, 0),
    resolvedAtlasFrameIndex: null,
    poseFallback: pose !== "neutral",
    frameScale: scaleForFacing(archetype, facing, 0),
  };
};

export { STATIC_POSE_MANIFEST };
