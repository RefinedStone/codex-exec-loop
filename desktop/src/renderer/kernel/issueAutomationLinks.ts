import { parseProviderState, type ProviderCredentialBridge } from "../../shared/providerCredentials";
import { isJiraIssueUrl, isLinearIssueUrl } from "../../shared/providerLinks";
import type { GatewayRequest } from "../work/notionHub";

/** Jira and Linear links use the existing main/Gateway current-connection check before OS opening. */
export async function openIssueAutomationUrl(url: string, dependencies: {
  readonly request: GatewayRequest;
  readonly native: Pick<ProviderCredentialBridge, "openExternal">;
  readonly openGitHub: (url: string) => void;
}): Promise<void> {
  if (url.startsWith("https://github.com/")) {
    dependencies.openGitHub(url);
    return;
  }
  const provider = url.startsWith("https://linear.app/") ? "linear" : "jira";
  try {
    const state = parseProviderState(await dependencies.request<unknown>(
      "akra/providerCredentials/state", { provider },
    ), provider);
    const active = state.activeConnection;
    const valid = provider === "linear"
      ? isLinearIssueUrl(url)
      : isJiraIssueUrl(active?.metadata.configuration.siteUrl, url);
    if (!active || url.trim() !== url || !valid) throw new Error();
    await dependencies.native.openExternal({ connection: active.reference, url });
  } catch {
    throw new Error(provider === "linear"
      ? "현재 Linear 연결의 이슈 주소를 확인할 수 없습니다."
      : "현재 Jira 연결의 이슈 주소를 확인할 수 없습니다.");
  }
}
