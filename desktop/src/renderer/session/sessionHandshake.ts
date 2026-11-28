import type { InitializeParams } from "../../shared/protocol/generated/InitializeParams";

// A native prediction does not attest isolated, redacted generation (#2914).
export const NATIVE_PREDICTION = "thread/prediction/updated";

export function initializeParams(clientVersion: string): InitializeParams {
  return {
    clientInfo: { name: "akra-desktop", title: "Akra Desktop", version: clientVersion },
    capabilities: { experimentalApi: false, requestAttestation: false, optOutNotificationMethods: [NATIVE_PREDICTION] },
  };
}
