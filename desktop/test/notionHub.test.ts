import { describe, expect, it } from "vitest";

import {
  MCP_METHOD,
  NOTION_SERVER,
  NotionHub,
  fetchedPageFrom,
  hubState,
  pagePrompt,
  pagesFrom,
  toolPayload,
} from "../src/renderer/work/notionHub";
import { isNotionPageUrl } from "../src/shared/notionLinks";

const textResult = (value: unknown) => ({ content: [{ type: "text", text: JSON.stringify(value) }] });
const status = (overrides: Record<string, unknown>) => ({ statuses: [{ name: NOTION_SERVER, auth: "oAuth", connection: "connected", tools: [], error: null, ...overrides }] });

describe("hubState", () => {
  it("reports an unregistered server", () => {
    expect(hubState({ statuses: [] })).toEqual({ kind: "unregistered" });
    expect(hubState({ statuses: [{ name: "jira", tools: [] }] })).toEqual({ kind: "unregistered" });
  });

  it("asks for a login when the hub has no token", () => {
    expect(hubState(status({ auth: "notLoggedIn", error: "Auth required" }))).toEqual({ kind: "needsLogin", detail: "Auth required" });
    expect(hubState(status({ connection: "authenticationRequired" }))).toEqual({ kind: "needsLogin" });
  });

  it("picks the official tool names and the connector names", () => {
    const official = status({ tools: [{ name: "notion-search" }, { name: "notion-fetch" }, { name: "notion-list-recent-pages" }] });
    expect(hubState(official)).toEqual({ kind: "ready", tools: { search: "notion-search", fetch: "notion-fetch", recent: "notion-list-recent-pages" } });
    const connector = status({ tools: [{ name: "search" }, { name: "fetch" }] });
    expect(hubState(connector)).toEqual({ kind: "ready", tools: { search: "search", fetch: "fetch" } });
  });

  it("fails when the server has no search tool", () => {
    expect(hubState(status({ tools: [{ name: "notion-fetch" }], error: "handshake failed" }))).toEqual({ kind: "failed", detail: "handshake failed" });
  });
});

describe("tool results", () => {
  it("prefers structured content and reads JSON text otherwise", () => {
    expect(toolPayload({ structuredContent: { a: 1 }, content: [{ type: "text", text: "{}" }] })).toEqual({ a: 1 });
    expect(toolPayload(textResult({ b: 2 }))).toEqual({ b: 2 });
  });

  it("turns tool errors and unknown shapes into errors", () => {
    expect(() => toolPayload({ isError: true, content: [{ type: "text", text: "rate limited" }] })).toThrow("rate limited");
    expect(() => toolPayload({ content: [{ type: "text", text: "not json" }] })).toThrow("형식");
    expect(() => toolPayload({ content: [] })).toThrow("내용");
  });

  it("reads search and recent pages and drops entries without a URL", () => {
    const pages = pagesFrom(
      textResult({
        results: [
          { type: "page", title: "Roadmap", url: "https://www.notion.so/Roadmap-1" },
          { title: "", url: "https://www.notion.so/2" },
          { title: "No link" },
        ],
        nextCursor: "abc",
      }),
    );
    expect(pages).toEqual([
      { title: "Roadmap", url: "https://www.notion.so/Roadmap-1", type: "page" },
      { title: "(제목 없음)", url: "https://www.notion.so/2" },
    ]);
    expect(() => pagesFrom(textResult({ items: [] }))).toThrow("목록");
  });

  it("reads a fetched page", () => {
    const page = fetchedPageFrom(
      textResult({ id: "x", title: "Spec", url: "https://www.notion.so/Spec-3", text: "# Goals", path: "Team / Spec", page_last_edited_at: "2026-11-10T00:00:00Z" }),
    );
    expect(page).toEqual({ title: "Spec", url: "https://www.notion.so/Spec-3", text: "# Goals", path: "Team / Spec", lastEditedAt: "2026-11-10T00:00:00Z" });
  });
});

describe("pagePrompt", () => {
  it("names the page and tells the agent to use the hub for edits", () => {
    const prompt = pagePrompt({ title: "Spec", url: "https://www.notion.so/Spec-3", text: "# Goals\n- ship", path: "Team / Spec" });
    expect(prompt).toContain("페이지: Spec (https://www.notion.so/Spec-3)");
    expect(prompt).toContain("위치: Team / Spec");
    expect(prompt).toContain("# Goals\n- ship");
    expect(prompt).toContain("`akra mcp`");
  });

  it("clips long pages", () => {
    const prompt = pagePrompt({ title: "Big", url: "https://www.notion.so/big", text: "x".repeat(50_000) });
    expect(prompt.length).toBeLessThanOrEqual(20_000 + 10);
    expect(prompt).toContain("…(생략)");
  });
});

describe("NotionHub", () => {
  it("sends hub requests for the notion server without a hub home", async () => {
    const sent: { method: string; params: unknown }[] = [];
    const hub = new NotionHub(<T,>(method: string, params: unknown): Promise<T> => {
      sent.push({ method, params });
      const reply = method === MCP_METHOD.statuses ? status({ tools: [{ name: "notion-search" }, { name: "notion-fetch" }] }) : textResult({ results: [] });
      return Promise.resolve(reply as T);
    });
    const state = await hub.state();
    expect(state.kind).toBe("ready");
    if (state.kind !== "ready") return;
    expect(await hub.recent(state.tools)).toEqual([]);
    await hub.search(state.tools, "roadmap");
    expect(sent).toEqual([
      { method: MCP_METHOD.statuses, params: { server: NOTION_SERVER } },
      { method: MCP_METHOD.call, params: { server: NOTION_SERVER, tool: "notion-search", arguments: { query: "roadmap" } } },
    ]);
  });
});

describe("isNotionPageUrl", () => {
  it("allows only https Notion hosts", () => {
    expect(isNotionPageUrl("https://www.notion.so/Roadmap-1")).toBe(true);
    expect(isNotionPageUrl("https://app.notion.com/p/1")).toBe(true);
    expect(isNotionPageUrl("https://team.notion.site/page")).toBe(true);
    expect(isNotionPageUrl("http://www.notion.so/x")).toBe(false);
    expect(isNotionPageUrl("https://notion.so.evil.com/x")).toBe(false);
    expect(isNotionPageUrl("https://user:pw@www.notion.so/x")).toBe(false);
    expect(isNotionPageUrl("javascript:alert(1)")).toBe(false);
    expect(isNotionPageUrl(42)).toBe(false);
  });
});
