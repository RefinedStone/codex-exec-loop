import type {
  ArchetypeKey,
  Point,
  StatusSeverity,
  VisualState,
} from "./game-types";

export const MAP_WIDTH = 1672;
export const MAP_HEIGHT = 941;
export const AGENT_FRAME_WIDTH = 128;
export const AGENT_FRAME_HEIGHT = 192;
export const AGENT_SPRITE_SCALE = 0.72;

export const SLOT_SEATS: Point[] = [
  { x: 675, y: 478 },
  { x: 962, y: 478 },
  { x: 675, y: 637 },
  { x: 962, y: 637 },
  { x: 817, y: 793 },
];

export const STANDBY_LOUNGE_POINTS: Point[] = [
  { x: 105, y: 756 },
  { x: 245, y: 745 },
  { x: 380, y: 720 },
];

export const STANDBY_LOUNGE_PATROL_ROUTES: Point[][] = [
  [
    { x: 105, y: 756 },
    { x: 90, y: 763 },
    { x: 92, y: 716 },
    { x: 120, y: 726 },
  ],
  [
    { x: 245, y: 745 },
    { x: 235, y: 764 },
    { x: 236, y: 710 },
    { x: 255, y: 700 },
    { x: 255, y: 744 },
  ],
  [
    { x: 380, y: 720 },
    { x: 370, y: 744 },
    { x: 372, y: 688 },
    { x: 392, y: 680 },
    { x: 395, y: 718 },
  ],
];

export const REVIEW_STATION_POINTS: Point[] = [
  { x: 210, y: 382 },
  { x: 302, y: 382 },
  { x: 350, y: 305 },
];

export const DELIVERY_STATION_POINTS: Point[] = [
  { x: 1340, y: 565 },
  { x: 1450, y: 620 },
  { x: 1375, y: 708 },
];

export const QA_CI_STATION_POINT: Point = { x: 1190, y: 326 };
export const QA_CI_SIGNAL_POINTS: Record<string, { from: Point; to: Point }> = {
  ci_observation: {
    from: { x: 942, y: 245 },
    to: QA_CI_STATION_POINT,
  },
  failure_to_queue: {
    from: QA_CI_STATION_POINT,
    to: { x: 470, y: 470 },
  },
  queue_to_worker: {
    from: QA_CI_STATION_POINT,
    to: SLOT_SEATS[1] ?? { x: 962, y: 478 },
  },
  verified_return: {
    from: QA_CI_STATION_POINT,
    to: { x: 1395, y: 525 },
  },
  blocked_alert: {
    from: { x: QA_CI_STATION_POINT.x - 54, y: QA_CI_STATION_POINT.y },
    to: { x: QA_CI_STATION_POINT.x + 54, y: QA_CI_STATION_POINT.y },
  },
};

export const CLEANUP_STATION_POINTS: Point[] = [
  { x: 160, y: 750 },
  { x: 275, y: 748 },
  { x: 365, y: 708 },
];

export const OCCLUSION_POLYGONS: Point[][] = [
  [
    { x: 575, y: 414 },
    { x: 770, y: 414 },
    { x: 770, y: 482 },
    { x: 575, y: 482 },
  ],
  [
    { x: 864, y: 414 },
    { x: 1060, y: 414 },
    { x: 1060, y: 482 },
    { x: 864, y: 482 },
  ],
  [
    { x: 575, y: 572 },
    { x: 770, y: 572 },
    { x: 770, y: 642 },
    { x: 575, y: 642 },
  ],
  [
    { x: 864, y: 572 },
    { x: 1060, y: 572 },
    { x: 1060, y: 642 },
    { x: 864, y: 642 },
  ],
  [
    { x: 715, y: 727 },
    { x: 920, y: 727 },
    { x: 920, y: 798 },
    { x: 715, y: 798 },
  ],
];

export interface PointOfInterestSpec {
  id: "director" | "review" | "delivery" | "events" | "standby";
  detailTarget: "director" | "review" | "pipeline" | "events" | "standby";
  label: string;
  x: number;
  y: number;
  width: number;
  height: number;
  color: number;
}

export const POINTS_OF_INTEREST: PointOfInterestSpec[] = [
  {
    id: "director",
    detailTarget: "director",
    label: "COMMAND",
    x: 815,
    y: 205,
    width: 330,
    height: 150,
    color: 0xf5c84b,
  },
  {
    id: "review",
    detailTarget: "review",
    label: "REVIEW",
    x: 240,
    y: 315,
    width: 420,
    height: 250,
    color: 0x9d7cff,
  },
  {
    id: "delivery",
    detailTarget: "pipeline",
    label: "DELIVERY",
    x: 1410,
    y: 525,
    width: 380,
    height: 520,
    color: 0x35d07f,
  },
  {
    id: "events",
    detailTarget: "events",
    label: "SIGNALS",
    x: 1405,
    y: 210,
    width: 360,
    height: 180,
    color: 0x5da9ff,
  },
  {
    id: "standby",
    detailTarget: "standby",
    label: "STANDBY",
    x: 235,
    y: 715,
    width: 420,
    height: 225,
    color: 0x5da9ff,
  },
];

export const ARCHETYPE_BY_PROFILE: Record<string, ArchetypeKey> = {
  Artificer: "planner",
  Seer: "planner",
  Scribe: "coffee_addict",
  Runner: "coffee_addict",
  Guardian: "ai_researcher",
  Ranger: "designer",
};

export const STATUS_PALETTE: Record<StatusSeverity, number> = {
  normal: 0x35d07f,
  success: 0x35d07f,
  warning: 0xf5c84b,
  danger: 0xff6b6b,
  info: 0x5da9ff,
  muted: 0x98abc4,
};

export const STATE_LABELS: Record<VisualState, string> = {
  idle: "대기",
  starting: "배치 중",
  working: "작업 중",
  awaiting_review: "검토 대기",
  blocked: "차단됨",
  delivering: "배포 중",
  cleanup: "정리 중",
};

export const pointAt = (points: Point[], index: number): Point =>
  points[index % points.length] ?? points[0] ?? { x: MAP_WIDTH / 2, y: MAP_HEIGHT / 2 };

export const seatPoint = (seatIndex: number): Point =>
  pointAt(SLOT_SEATS, Math.max(0, seatIndex - 1));

export const stateTargetPoint = (
  visualState: VisualState,
  seatIndex: number,
  stableIndex: number
): Point => {
  if (visualState === "awaiting_review") return pointAt(REVIEW_STATION_POINTS, stableIndex);
  if (visualState === "delivering") return pointAt(DELIVERY_STATION_POINTS, stableIndex);
  if (visualState === "cleanup") return pointAt(CLEANUP_STATION_POINTS, stableIndex);
  return seatPoint(seatIndex);
};
