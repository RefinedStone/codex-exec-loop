import { Rectangle, Texture } from "pixi.js";
import type {
  GamePrApproverProjection,
  PrApproverClip,
  PrApproverQualifier,
  PrApproverState,
} from "./game-types";

export const PR_APPROVER_FRAME_WIDTH = 128;
export const PR_APPROVER_FRAME_HEIGHT = 192;
export const PR_APPROVER_COLUMNS = 6;
export const PR_APPROVER_FRAME_COUNT = 24;

interface AnimationStep {
  frame: number;
  durationMs: number;
}

interface ClipDefinition {
  steps: readonly AnimationStep[];
  loop: boolean;
}

const clip = (
  frames: readonly number[],
  durations: readonly number[],
  loop: boolean
): ClipDefinition => ({
  steps: frames.map((frame, index) => ({
    frame,
    durationMs: durations[index] ?? durations[durations.length - 1] ?? 400,
  })),
  loop,
});

// Intake frame 1 intentionally stays reserved: ImageGen represented the incoming
// hand as an external courier. The existing packet animation carries that event,
// so the officer begins at the received-document pose instead.
const CLIPS: Record<PrApproverClip, ClipDefinition> = {
  idle: clip([0], [1_000], false),
  intake: clip([0, 2, 3, 4, 5], [380, 300, 340, 420, 560], false),
  review: clip([6, 7, 8, 9, 10, 11], [620, 560, 260, 230, 230, 440], true),
  failure: clip([12, 13, 14, 15, 16, 17], [420, 340, 320, 280, 360, 900], false),
  success: clip([18, 19, 20, 21, 22, 23], [420, 330, 340, 360, 440, 900], false),
};

const representativeFrame = (state: PrApproverState): number => {
  switch (state) {
    case "reviewing":
      return 6;
    case "failure":
      return 17;
    case "success":
      return 23;
    default:
      return 0;
  }
};

export const buildPrApproverFrames = (atlas: Texture): Texture[] =>
  Array.from({ length: PR_APPROVER_FRAME_COUNT }, (_, index) =>
    new Texture({
      source: atlas.source,
      frame: new Rectangle(
        (index % PR_APPROVER_COLUMNS) * PR_APPROVER_FRAME_WIDTH,
        Math.floor(index / PR_APPROVER_COLUMNS) * PR_APPROVER_FRAME_HEIGHT,
        PR_APPROVER_FRAME_WIDTH,
        PR_APPROVER_FRAME_HEIGHT
      ),
    })
  );

export interface PrApproverAnimationSnapshot {
  state: PrApproverState;
  qualifier: PrApproverQualifier;
  clip: PrApproverClip;
  frameIndex: number;
  sourceFrameIndex: number;
  settled: boolean;
  transitionKey: string;
}

export class PrApproverAnimator {
  private projection: GamePrApproverProjection;
  private activeClip: PrApproverClip = "idle";
  private elapsedMs = 0;
  private settled = true;

  constructor(projection: GamePrApproverProjection) {
    this.projection = projection;
    this.selectClip(true);
  }

  reconcile(projection: GamePrApproverProjection): void {
    const transitionChanged = projection.transitionKey !== this.projection.transitionKey;
    const qualifierChanged = projection.qualifier !== this.projection.qualifier;
    this.projection = projection;
    if (transitionChanged) {
      this.selectClip(true);
    } else if (qualifierChanged) {
      this.selectClip(false);
    }
  }

  update(deltaMilliseconds: number, reducedMotion: boolean): PrApproverAnimationSnapshot {
    if (reducedMotion || this.isFrozenQualifier()) {
      return this.staticSnapshot(representativeFrame(this.projection.state));
    }
    if (this.activeClip === "idle") {
      return this.staticSnapshot(0);
    }
    if (!this.settled || CLIPS[this.activeClip].loop) {
      this.elapsedMs += Math.min(250, Math.max(0, deltaMilliseconds));
    }
    const current = this.resolveCurrentFrame();
    if (current.completed && this.activeClip === "intake") {
      this.activeClip = "review";
      this.elapsedMs = 0;
      this.settled = false;
      return this.resolveCurrentFrame().snapshot;
    }
    this.settled = current.completed && !CLIPS[this.activeClip].loop;
    return current.snapshot;
  }

  snapshot(reducedMotion: boolean): PrApproverAnimationSnapshot {
    if (reducedMotion || this.isFrozenQualifier()) {
      return this.staticSnapshot(representativeFrame(this.projection.state));
    }
    if (this.activeClip === "idle") {
      return this.staticSnapshot(0);
    }
    return this.resolveCurrentFrame().snapshot;
  }

  private selectClip(resetElapsed: boolean): void {
    this.activeClip = this.projection.state === "reviewing"
      ? "intake"
      : this.projection.state;
    if (resetElapsed) this.elapsedMs = 0;
    this.settled = this.activeClip === "idle";
  }

  private isFrozenQualifier(): boolean {
    return this.projection.qualifier === "paused"
      || this.projection.qualifier === "stale"
      || this.projection.qualifier === "waiting";
  }

  private staticSnapshot(sourceFrameIndex: number): PrApproverAnimationSnapshot {
    return {
      state: this.projection.state,
      qualifier: this.projection.qualifier,
      clip: this.projection.state === "reviewing" ? "review" : this.projection.state,
      frameIndex: 0,
      sourceFrameIndex,
      settled: true,
      transitionKey: this.projection.transitionKey,
    };
  }

  private resolveCurrentFrame(): {
    completed: boolean;
    snapshot: PrApproverAnimationSnapshot;
  } {
    const definition = CLIPS[this.activeClip];
    const totalDuration = definition.steps.reduce((total, step) => total + step.durationMs, 0);
    const elapsed = definition.loop && totalDuration > 0
      ? this.elapsedMs % totalDuration
      : Math.min(this.elapsedMs, totalDuration);
    let cursor = 0;
    let stepIndex = definition.steps.length - 1;
    for (let index = 0; index < definition.steps.length; index += 1) {
      cursor += definition.steps[index]?.durationMs ?? 0;
      if (elapsed < cursor) {
        stepIndex = index;
        break;
      }
    }
    const step = definition.steps[stepIndex] ?? definition.steps[0] ?? { frame: 0 };
    const completed = !definition.loop && this.elapsedMs >= totalDuration;
    return {
      completed,
      snapshot: {
        state: this.projection.state,
        qualifier: this.projection.qualifier,
        clip: this.activeClip,
        frameIndex: stepIndex,
        sourceFrameIndex: step.frame,
        settled: completed,
        transitionKey: this.projection.transitionKey,
      },
    };
  }
}
