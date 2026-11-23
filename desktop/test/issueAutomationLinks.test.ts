import { describe, expect, it, vi } from "vitest";
import { openIssueAutomationUrl } from "../src/renderer/kernel/issueAutomationLinks";
import type { GatewayRequest } from "../src/renderer/work/notionHub";
import { connected, disconnected, reference, SECRET_CANARY } from "./providerFixtures";

function harness(state = connected("jira")) {
  const request = vi.fn(async (_method: string, _params: unknown) => state);
  const openExternal = vi.fn(async (_request: unknown) => undefined);
  const openGitHub = vi.fn((_url: string) => undefined);
  return { request, openExternal, openGitHub,
    open: (url: string) => openIssueAutomationUrl(url, { request: request as GatewayRequest, native: { openExternal }, openGitHub }),
  };
}

describe("automation issue links", () => {
  it("keeps GitHub links on the existing route without reading provider credentials", async () => {
    const h = harness();
    const url = "https://github.com/sample/widgets/issues/17";
    await h.open(url);
    expect(h.openGitHub).toHaveBeenCalledWith(url);
    expect(h.request).not.toHaveBeenCalled();
    expect(h.openExternal).not.toHaveBeenCalled();
  });
  it("passes only the current public Jira reference and URL to the validated native bridge", async () => {
    const h = harness();
    const url = "https://jira.example/jira/browse/APP-1";
    await h.open(url);
    expect(h.request).toHaveBeenCalledWith("akra/providerCredentials/state", { provider: "jira" });
    expect(h.openExternal).toHaveBeenCalledWith({ connection: reference("jira"), url });
    expect(h.openGitHub).not.toHaveBeenCalled();
  });
  it.each(["https://evil.example/browse/APP-1", "https://jira.example/browse/APP-1", "https://jira.example/jira/browse/APP-1\n"])("rejects a URL outside the current exact Jira site: %s", async url => {
    const h = harness();
    await expect(h.open(url)).rejects.toThrow("현재 Jira 연결");
    expect(h.openExternal).not.toHaveBeenCalled();
    expect(h.openGitHub).not.toHaveBeenCalled();
  });
  it("does not fall back after disconnect or a changed native connection", async () => {
    const disconnectedHost = harness(disconnected("jira"));
    const url = "https://jira.example/jira/browse/APP-1";
    await expect(disconnectedHost.open(url)).rejects.toThrow("현재 Jira 연결");
    expect(disconnectedHost.openExternal).not.toHaveBeenCalled();
    const h = harness();
    h.openExternal.mockRejectedValue(new Error(SECRET_CANARY));
    await expect(h.open(url)).rejects.toThrow("현재 Jira 연결");
    expect(h.openGitHub).not.toHaveBeenCalled();
  });
});
