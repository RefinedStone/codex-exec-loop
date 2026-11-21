import { session } from "electron";

import type { ChatPageStatus } from "./delegatedChatDriver";
import { DELEGATED_CHAT_PARTITION, DELEGATED_CHAT_URL } from "./delegatedChatWindow";

export async function chatAccountStatus(expected?: string): Promise<ChatPageStatus> {
  if (!expected) return { readiness: "unknown", accountMatch: "unknown" };
  // Compare the selected web workspace account with the Gateway-supplied
  // account. The response may contain credentials. Keep it in main and
  // return only fixed values. A changed schema always closes the send gate.
  const browser = session.fromPartition(DELEGATED_CHAT_PARTITION);
  const response = await browser.fetch(new URL("/api/auth/session", DELEGATED_CHAT_URL).href, {
    credentials: "include",
    redirect: "error",
    cache: "no-store",
    signal: AbortSignal.timeout(1_500),
  });
  if (response.status === 401) return { readiness: "login_required", accountMatch: "unknown" };
  if (!response.ok || !response.headers.get("content-type")?.startsWith("application/json")) {
    return { readiness: "unknown", accountMatch: "unknown" };
  }
  const value = await response.json() as { account?: { id?: unknown } } | null;
  const accountId = value?.account?.id;
  if (typeof accountId !== "string" || !accountId) return { readiness: "unknown", accountMatch: "unknown" };
  return { readiness: "ready", accountMatch: accountId === expected ? "match" : "mismatch" };
}
