import { useEffect } from "react";
import type { AttentionItem } from "../../shared/kernel-protocol/generated";
import { useDesktopStore } from "../session/desktopStore";
import { toast } from "../session/uiStore";
import { newAttentionCount } from "./attentionNavigation";

/** A query failure withdraws this window's badge report. Its recovering snapshot is not a new event. */
export function useAttentionSignals(): void {
  const badgeKernel = useDesktopStore(state => state.connection?.phase === "connected" && state.kernel.phase === "attached"
    && state.kernel.attentionItems !== null ? state.kernel.kernel : null);
  const badgeCount = useDesktopStore(state => state.kernel.attentionItems?.length ?? 0);
  useEffect(() => window.akra.settings.reportAttention(badgeKernel, badgeCount), [badgeKernel, badgeCount]);

  useEffect(() => {
    let timer: number | undefined;
    let baseline: readonly AttentionItem[] | null = null;
    let generation = 0;
    const cancel = () => { generation++; window.clearTimeout(timer); timer = undefined; baseline = null; };
    const unsubscribe = useDesktopStore.subscribe(({kernel}, previous) => {
      if (kernel.kernel !== previous.kernel.kernel || kernel.phase !== "attached" || kernel.attentionItems === null) { cancel(); return; }
      if (previous.kernel.attentionItems === kernel.attentionItems || !newAttentionCount(previous.kernel.attentionItems, kernel.attentionItems)) return;
      if (timer !== undefined) return;
      baseline = previous.kernel.attentionItems;
      const scope = ++generation;
      // Count surviving revisions across a short burst, not one toast per network snapshot.
      timer = window.setTimeout(() => {
        const before = baseline;
        timer = undefined;
        void window.akra.isAppFocused("window").then(focused => {
          const current = useDesktopStore.getState().kernel;
          if (!focused || generation !== scope || current.kernel !== kernel.kernel || current.phase !== "attached") return;
          const count = newAttentionCount(before, current.attentionItems);
          if (count) toast(`새 응답 필요 항목 ${count}개가 있습니다.`);
        });
      }, 250);
    });
    return () => { unsubscribe(); cancel(); };
  }, []);
}
