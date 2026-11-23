import { parseProviderState, type ProviderCredentialBridge } from "../../shared/providerCredentials";
import { isJiraIssueUrl } from "../../shared/providerLinks";
import type { GatewayRequest } from "../work/notionHub";

/** Jira links use the existing main/Gateway current-connection check before OS opening. */
export async function openIssueAutomationUrl(url: string, dependencies: {
  readonly request: GatewayRequest;
  readonly native: Pick<ProviderCredentialBridge, "openExternal">;
  readonly openGitHub: (url: string) => void;
}): Promise<void> {
  if (url.startsWith("https://github.com/")) {
    dependencies.openGitHub(url);
    return;
  }
  try {
    const state = parseProviderState(await dependencies.request<unknown>(
      "akra/providerCredentials/state", { provider: "jira" },
    ), "jira");
    const active = state.activeConnection;
    if (!active || url.trim() !== url || !isJiraIssueUrl(active.metadata.configuration.siteUrl, url)) throw new Error();
    await dependencies.native.openExternal({ connection: active.reference, url });
  } catch {
    throw new Error("현재 Jira 연결의 이슈 주소를 확인할 수 없습니다.");
  }
}
