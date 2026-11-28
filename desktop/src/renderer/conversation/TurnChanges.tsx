import { useMemo } from "react";
import { Icon } from "../design/icons";
import { useConversationStore } from "../session/desktopStore";
import { openPanel } from "../session/uiStore";
import { totalCounts, turnDiffSources } from "./diff";
import { DiffCounts, DiffFileView } from "./DiffView";
import type { TurnView } from "./transcript";
import { TurnDiffContext } from "./TurnDiffContext";

/** Historical source stays in the conversation's host store, including remote conversations. */
export function TurnChanges({ turn }: { readonly turn: TurnView }) {
  const hostId = useConversationStore(state => state.connection?.endpoint ?? null);
  const threadId = useConversationStore(state => state.transcript?.threadId ?? null);
  const cwd = useConversationStore(state => state.transcript?.cwd ?? null);
  const sources = useMemo(() => turnDiffSources(turn.diff, turn.items), [turn.diff, turn.items]);
  const scope = useMemo(() => threadId === null || cwd === null ? null
    : { hostId, threadId, cwd, turnId: turn.id }, [hostId, threadId, cwd, turn.id]);
  if (sources.length === 0) return null;
  const counts = totalCounts(sources.map(source => source.file));
  return <div className="changes-card" data-testid="turn-changes">
    <header>
      <Icon name="diff" /><span>파일 {sources.length}개 변경</span>
      <DiffCounts added={counts.added} removed={counts.removed} />
      <button type="button" className="button" onClick={() => openPanel("review")}>변경 검토</button>
    </header>
    <div className="diff-file-list">
      {sources.map(source => <DiffFileView key={source.origin + ":" + source.file.path} file={source.file} defaultOpen={false}
        context={scope && <TurnDiffContext source={source} scope={scope} />} />)}
    </div>
  </div>;
}
