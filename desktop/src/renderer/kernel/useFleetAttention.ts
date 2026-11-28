import { useEffect, useState } from "react";
import { KERNEL_METHOD, type FleetAttention } from "./protocol";
import type { GatewayRequest } from "../work/notionHub";

type Reads = ReadonlyMap<string, FleetAttention | null>;

// A serial sweep bounds work on the shared Gateway connection. Its results never change the
// foreground attachment. A new folder set, refresh or disconnect invalidates the old sweep.
export function useFleetAttention(paths: readonly string[], request: GatewayRequest, connected: boolean, refresh: number): Reads {
  const key = JSON.stringify(paths);
  const [state, setState] = useState<{key: string; refresh: number; reads: Reads} | null>(null);
  useEffect(() => {
    let current = true;
    setState(null);
    if (!connected) return;
    const read = async () => {
      const reads = new Map<string, FleetAttention | null>();
      for (const workspace of paths) {
        let result: FleetAttention | null = null;
        try {
          const snapshot = await request<FleetAttention>(KERNEL_METHOD.fleet_attention, {workspace});
          if (snapshot && typeof snapshot.kernel === "string" && snapshot.kernel && Array.isArray(snapshot.items)) result = snapshot;
        } catch { /* A stopped kernel or an old Gateway cannot confirm an empty list. */ }
        if (!current) return;
        reads.set(workspace, result);
        setState({key, refresh, reads:new Map(reads)});
      }
    };
    void read();
    return () => { current = false; };
  }, [key, request, connected, refresh]);
  return connected && state?.key === key && state.refresh === refresh ? state.reads : new Map();
}
