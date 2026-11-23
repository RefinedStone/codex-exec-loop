import { useCallback, useEffect, useRef, useState } from "react";
import {
  operationPending, type LegacyAvailability, type ProviderConnectionBridge, type ProviderCredentialState,
} from "../../shared/providerCredentials";
import { ProviderGatewayError } from "./providerGateway";

export function providerConnectionKey(state: ProviderCredentialState | null): string {
  const reference = state?.activeConnection?.reference;
  return reference ? [reference.profileId, reference.provider, reference.connectionId, reference.revision].join(":") : "";
}
export function useProviderConnection(bridge: ProviderConnectionBridge) {
  const [state, setState] = useState<ProviderCredentialState | null>(null);
  const [legacy, setLegacy] = useState<LegacyAvailability>({ kind: "absent" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(false);
  const loadSequence = useRef(0);
  const actionSequence = useRef(0);
  const message = (cause: unknown) => cause instanceof ProviderGatewayError ? cause.message : "연결 상태를 확인할 수 없습니다. 다시 확인해 주세요.";

  const reload = useCallback(async () => {
    const sequence = ++loadSequence.current;
    try {
      const current = await bridge.status();
      if (!alive.current || sequence !== loadSequence.current) return;
      setState(current);
      const available = await bridge.legacyStatus();
      if (alive.current && sequence === loadSequence.current) setLegacy(available);
    } catch (cause) {
      if (alive.current && sequence === loadSequence.current) setError(message(cause));
    }
  }, [bridge]);

  useEffect(() => {
    alive.current = true;
    setState(null); setError(null); setBusy(false); setLegacy({ kind: "absent" });
    const unsubscribe = bridge.onChanged(changed => {
      if (changed) { setState(null); loadSequence.current++; }
      void reload();
    });
    void reload();
    return () => { alive.current = false; loadSequence.current++; actionSequence.current++; unsubscribe(); };
  }, [bridge, reload]);

  const run = useCallback(async (action: () => Promise<ProviderCredentialState>): Promise<boolean> => {
    const sequence = ++actionSequence.current;
    setBusy(true); setError(null);
    try {
      const current = await action();
      if (!alive.current || sequence !== actionSequence.current) return false;
      loadSequence.current++;
      setState(current);
      const available = await bridge.legacyStatus();
      if (!alive.current || sequence !== actionSequence.current) return false;
      setLegacy(available);
      return true;
    } catch (cause) {
      if (!alive.current || sequence !== actionSequence.current) return false;
      setError(message(cause));
      await reload();
      return false;
    } finally {
      if (alive.current && sequence === actionSequence.current) setBusy(false);
    }
  }, [bridge, reload]);

  return {
    state, legacy, busy, error, run, reload,
    pending: operationPending(state?.operation ?? null),
    canConnect: state?.capabilities.nativeInput === true && state.capabilities.keyring,
    canCleanLegacy: state?.capabilities.legacyImport === true && state.status === "disconnected" && state.revision > 0 &&
      !state.operation?.cleanupPending && (legacy.kind === "available" || legacy.kind === "unavailable"),
    canMigrate: state?.capabilities.legacyImport === true && state.capabilities.keyring &&
      (legacy.kind === "pending" || (legacy.kind === "available" && state.revision === 0)),
  };
}
