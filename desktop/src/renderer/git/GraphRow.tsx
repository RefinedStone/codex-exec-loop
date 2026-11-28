import { DRAG_BRANCH } from "./GroupPlanControls";
import { memo } from "react";
import { type GitGraphRow, type GitRefLabel, type GitThreadBadge, relativeTime, shortSha } from "./gitGraph";
import { laneColorIndex, laneX, NODE_RADIUS, ROW_HEIGHT, rowPaths } from "./graphGeometry";

const REF_KIND_TITLE: Partial<Record<GitRefLabel["kind"], string>> = { backup: "백업", tag: "태그" };

function RefChip({ label }: { label: GitRefLabel }) {
  const kind = REF_KIND_TITLE[label.kind];
  const title = label.syncedRemotes.length > 0 ? `${label.name} (${label.syncedRemotes.join(", ")} 와 같은 커밋)` : kind ? `${kind} ${label.name}` : label.name;
  return (
    <span draggable={label.head && label.kind === "local_branch"} onDragStart={e=>{if(label.head && label.kind==="local_branch"){e.dataTransfer.setData(DRAG_BRANCH,label.name);e.dataTransfer.effectAllowed="move";}}} className={`git-ref git-ref-${label.kind}${label.head ? " git-ref-current" : ""}`} title={title} data-testid="git-graph-ref">
      {label.name}
      {label.syncedRemotes.length > 0 && <span className="git-ref-synced">{label.syncedRemotes.join(" ")}</span>}
    </span>
  );
}

function ThreadBadge({ badge, onOpenThread }: { badge: GitThreadBadge; onOpenThread(threadId: string): void }) {
  const others = badge.threadCount > 1 ? ` 외 ${badge.threadCount - 1}개` : "";
  return (
    <button
      type="button"
      className="git-thread-badge"
      title={`스레드 열기: ${badge.title}${others} (${badge.worktreePath})`}
      data-testid="git-graph-thread"
      onClick={(event) => {
        // 줄을 고르지 않는다. 배지는 스레드로 간다.
        event.stopPropagation();
        onOpenThread(badge.threadId);
      }}
    >
      {badge.title}
      {badge.threadCount > 1 && <span className="git-ref-synced">+{badge.threadCount - 1}</span>}
    </button>
  );
}

const GraphCell = memo(function GraphCell({ row, width, head, planned }: { row: GitGraphRow; width: number; head: boolean; planned: boolean }) {
  return (
    <svg className="git-graph-cell" width={width} height={ROW_HEIGHT} aria-hidden="true">
      {rowPaths(row).map((path, index) => (
        <path key={index} d={path.d} className={`git-lane-${laneColorIndex(path.colorLane)}`} />
      ))}
      <circle cx={laneX(row.lane)} cy={ROW_HEIGHT / 2} r={NODE_RADIUS} className={`git-node git-lane-${laneColorIndex(row.lane)}${head ? " git-node-head" : ""}`} />
      {planned && <circle cx={laneX(row.lane)} cy={ROW_HEIGHT / 2} r={NODE_RADIUS + 3} className="git-node-planned" />}
    </svg>
  );
});

interface GraphRowProps {
  row: GitGraphRow;
  width: number;
  labels: GitRefLabel[] | undefined;
  badges: GitThreadBadge[] | undefined;
  head: boolean;
  /** 열린 계획이 이 커밋으로 옮기는 브랜치(또는 HEAD)이다. 점선으로 그린다. */
  planned: { name: string; title: string } | undefined;
  now: number;
  onOpenThread(threadId: string): void;
}

export const GraphRow = memo(function GraphRow({ row, width, labels, badges, head, planned, now, onOpenThread }: GraphRowProps) {
  return (
    <>
      <GraphCell row={row} width={width} head={head} planned={planned !== undefined} />
      <span className="git-row-main">
        {planned && (
          <span className="git-ref git-ref-planned" title={`계획: ${planned.title}`} data-testid="git-graph-planned">
            {planned.name}
          </span>
        )}
        {labels?.map((label) => <RefChip key={`${label.kind}:${label.name}`} label={label} />)}
        {badges?.map((badge) => <ThreadBadge key={badge.worktreePath} badge={badge} onOpenThread={onOpenThread} />)}
        <span className="git-row-subject" title={row.subject}>
          {row.subject}
        </span>
      </span>
      <span className="git-row-author muted" title={row.authorName}>
        {row.authorName}
      </span>
      <span className="git-row-time muted">{relativeTime(row.authorTime, now)}</span>
      <code className="git-row-sha muted" title={row.id}>
        {shortSha(row.id)}
      </code>
    </>
  );
});
