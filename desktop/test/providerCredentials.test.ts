import { describe, expect, it } from "vitest";
import { isJiraIssueUrl, isLinearIssueUrl, normalizeJiraSite } from "../src/shared/providerLinks";
import { parseConnectionRef, parseLegacyTarget, parseOperationReceipt, parseProviderState } from "../src/shared/providerCredentials";
import { connected, PROFILE, receipt, reference, SECRET_CANARY, SECOND_PROFILE } from "./providerFixtures";

describe("public provider DTOs and URL hints", () => {
  it("keeps Jira HTTPS site, port and context identity when suggesting an issue URL", () => {
    expect(normalizeJiraSite(" https://JIRA.example:8443/jira/?ignored=1#fragment ")).toBe("https://jira.example:8443/jira");
    expect(isJiraIssueUrl("https://jira.example:8443/jira", "https://jira.example:8443/jira/browse/APP-1")).toBe(true);
    for (const url of ["http://jira.example:8443/jira/browse/APP-1", "https://jira.example/jira/browse/APP-1",
      "https://jira.example:8443/browse/APP-1", "https://jira.example:8443/jira/browse/APP-1?token=1",
      "https://jira.example:8443/jira/browse/APP-1#fragment", "https://evil.example/browse/APP-1"]) {
      expect(isJiraIssueUrl("https://jira.example:8443/jira", url)).toBe(false);
    }
    expect(normalizeJiraSite("http://jira.corp.example")).toBeNull();
  });
  it("restricts Linear hints to a normal HTTPS issue path", () => {
    expect(isLinearIssueUrl("https://linear.app/team/issue/APP-1/example")).toBe(true);
    for (const url of ["http://linear.app/team/issue/APP-1", "https://linear.app.evil/team/issue/APP-1",
      "https://user@linear.app/team/issue/APP-1", "https://linear.app/team/settings", "https://linear.app:444/team/issue/APP-1"]) {
      expect(isLinearIssueUrl(url)).toBe(false);
    }
  });
  it("rejects unknown secret fields at every public credential boundary", () => {
    expect(() => parseConnectionRef({ ...reference(), token: SECRET_CANARY })).toThrow();
    expect(() => parseLegacyTarget({ profileId: PROFILE, provider: "linear", expectedRevision: 0, token: SECRET_CANARY })).toThrow();
    expect(() => parseOperationReceipt({ ...receipt(), token: SECRET_CANARY })).toThrow();
    expect(() => parseProviderState({ ...connected(), token: SECRET_CANARY }, "linear")).toThrow();
    const wrong = connected();
    expect(() => parseProviderState({ ...wrong, activeConnection: { ...wrong.activeConnection,
      reference: { ...wrong.activeConnection!.reference, profileId: SECOND_PROFILE } } }, "linear")).toThrow();
  });
});
