// 커밋 그래프의 한 줄을 SVG 선으로 바꾼다 (plan 27 G1). 레인 배치는 데몬이 정한다. 여기서는 그리기 좌표만 계산한다.
// 한 줄은 위쪽 경계에서 선을 받고 아래쪽 경계로 선을 보낸다. 점은 줄의 가운데에 있다.

/** 레인 하나의 너비이다. CSS 픽셀이다. */
export const LANE_WIDTH = 14;
/** 한 줄의 높이이다. 가상 목록의 줄 높이와 같아야 선이 이어진다. */
export const ROW_HEIGHT = 28;
/** 점의 반지름이다. */
export const NODE_RADIUS = 4;
/** 그래프 열이 그리는 레인의 최댓값이다. 넘는 레인은 열의 오른쪽 끝에서 잘린다. */
export const MAX_VISIBLE_LANES = 16;
/** 레인 색의 개수이다. 색은 CSS 의 `--git-lane-<n>` 이다. */
export const LANE_COLOR_COUNT = 8;

/** 한 줄의 레인 배치이다. 데몬의 `GitGraphRow` 에서 그리기에 쓰는 값만 뽑은 것이다. */
export interface RowLanes {
  lane: number;
  lanesIn: readonly number[];
  lanesOut: readonly number[];
  lanesThrough: readonly number[];
}

export interface GraphPath {
  /** SVG `d` 값이다. */
  d: string;
  /** 선의 색을 정하는 레인이다. */
  colorLane: number;
}

export function laneX(lane: number): number {
  return lane * LANE_WIDTH + LANE_WIDTH / 2;
}

export function laneColorIndex(lane: number): number {
  return lane % LANE_COLOR_COUNT;
}

/** 줄이 쓰는 가장 오른쪽 레인이다. */
export function maxLane(row: RowLanes): number {
  return Math.max(row.lane, ...row.lanesIn, ...row.lanesOut, ...row.lanesThrough);
}

/** 줄들을 모두 그리는 데 필요한 그래프 열의 너비이다. */
export function graphWidth(rows: readonly RowLanes[]): number {
  let lanes = 1;
  for (const row of rows) lanes = Math.max(lanes, maxLane(row) + 1);
  return Math.min(lanes, MAX_VISIBLE_LANES) * LANE_WIDTH;
}

/** 경계의 레인과 점을 잇는 선이다. 같은 레인이면 곧은 선이고, 다르면 굽은 선이다. */
function nodeLink(nodeLane: number, edgeLane: number, edgeY: number): string {
  const nodeX = laneX(nodeLane);
  const edgeX = laneX(edgeLane);
  const nodeY = ROW_HEIGHT / 2;
  if (nodeLane === edgeLane) return `M${edgeX} ${edgeY}L${nodeX} ${nodeY}`;
  const bendY = (edgeY + nodeY) / 2;
  return `M${edgeX} ${edgeY}C${edgeX} ${bendY} ${nodeX} ${bendY} ${nodeX} ${nodeY}`;
}

/** 한 줄의 선들이다. 지나가는 선, 들어오는 선, 나가는 선의 순서이다. 점은 부르는 쪽이 그 위에 그린다. */
export function rowPaths(row: RowLanes): GraphPath[] {
  const through = row.lanesThrough.map((lane) => ({ d: `M${laneX(lane)} 0L${laneX(lane)} ${ROW_HEIGHT}`, colorLane: lane }));
  const incoming = row.lanesIn.map((lane) => ({ d: nodeLink(row.lane, lane, 0), colorLane: lane }));
  const outgoing = row.lanesOut.map((lane) => ({ d: nodeLink(row.lane, lane, ROW_HEIGHT), colorLane: lane }));
  return [...through, ...incoming, ...outgoing];
}
