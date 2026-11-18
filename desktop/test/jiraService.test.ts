import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, expect, it, vi } from "vitest";

import { adfToText, issuePrompt, type JiraFetch, JiraService, normalizeSiteUrl, parseIssues, type SecretBox } from "../src/main/jiraService";

const SITE = "https://acme.atlassian.net";

const searchJson = JSON.stringify({
  issues: [
    {
      key: "APP-12",
      fields: {
        summary: "Login fails",
        updated: "2026-11-06T10:00:00.000+0900",
        status: { name: "In Progress", statusCategory: { key: "indeterminate" } },
        issuetype: { name: "Bug" },
        priority: { name: "High" },
      },
    },
    { key: "bad key", fields: { summary: "dropped" } },
    { fields: { summary: "no key" } },
  ],
});

const issueJson = JSON.stringify({
  key: "APP-12",
  fields: {
    summary: "Login fails",
    status: { name: "In Progress" },
    issuetype: { name: "Bug" },
    priority: null,
    description: {
      type: "doc",
      content: [
        { type: "paragraph", content: [{ type: "text", text: "Steps:" }, { type: "hardBreak" }, { type: "text", text: "open " }, { type: "inlineCard", attrs: { url: "https://app.example/login" } }] },
        { type: "bulletList", content: [{ type: "listItem", content: [{ type: "paragraph", content: [{ type: "mention", attrs: { text: "@kim" } }, { type: "text", text: " saw it" }] }] }] },
      ],
    },
    comment: { total: 12, comments: [{ author: { displayName: "Kim" }, body: { type: "doc", content: [{ type: "paragraph", content: [{ type: "text", text: "Still broken" }] }] } }] },
  },
});

/** 시험용 암호화이다. 실제 앱은 safeStorage 를 쓴다. */
const box: SecretBox = {
  isEncryptionAvailable: () => true,
  encryptString: (text) => Buffer.from(`enc:${text}`),
  decryptString: (data) => data.toString().replace(/^enc:/, ""),
};

function recorder(responses: Record<string, { status: number; body: string }>) {
  const calls: Array<{ url: string; authorization: string }> = [];
  const fetcher: JiraFetch = async (url, init) => {
    calls.push({ url, authorization: init.headers.Authorization ?? "" });
    const match = Object.entries(responses).find(([prefix]) => url.includes(prefix));
    const response = match?.[1] ?? { status: 404, body: "" };
    return { status: response.status, text: async () => response.body };
  };
  return { calls, fetcher };
}

const directories: string[] = [];
afterEach(async () => {
  await Promise.all(directories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

async function service(fetcher: JiraFetch, secrets: SecretBox = box) {
  const directory = await mkdtemp(join(tmpdir(), "akra-jira-"));
  directories.push(directory);
  return { directory, jira: new JiraService(directory, secrets, fetcher) };
}

describe("Jira parsing", () => {
  it("accepts only https site addresses and keeps a server context path", () => {
    expect(normalizeSiteUrl(" https://acme.atlassian.net/ ")).toBe(SITE);
    expect(normalizeSiteUrl("https://jira.corp.example/jira/?x=1#y")).toBe("https://jira.corp.example/jira");
    expect(normalizeSiteUrl("http://jira.corp.example")).toBeNull();
    expect(normalizeSiteUrl("https://user:pass@acme.atlassian.net")).toBeNull();
    expect(normalizeSiteUrl("not a url")).toBeNull();
  });

  it("reads issues and drops entries without a valid key", () => {
    expect(parseIssues(searchJson, SITE)).toEqual([
      {
        key: "APP-12",
        summary: "Login fails",
        url: `${SITE}/browse/APP-12`,
        status: "In Progress",
        statusCategory: "indeterminate",
        issueType: "Bug",
        priority: "High",
        updatedAt: "2026-11-06T10:00:00.000+0900",
      },
    ]);
  });

  it("turns Cloud documents into text and keeps Server text as is", () => {
    expect(adfToText(JSON.parse(issueJson).fields.description)).toBe("Steps:\nopen https://app.example/login\n- @kim saw it\n");
    expect(adfToText("h1. Server wiki text")).toBe("h1. Server wiki text");
  });

  it("builds a first message with description, comments, and a note about missing comments", () => {
    const prompt = issuePrompt(issueJson, SITE);
    expect(prompt).toContain(`티켓: APP-12 Login fails (${SITE}/browse/APP-12)`);
    expect(prompt).toContain("유형: Bug · 상태: In Progress");
    expect(prompt).toContain("@kim saw it");
    expect(prompt).toContain("Kim: Still broken");
    expect(prompt).toContain("댓글 12개 중 최근 1개만");
  });
});

describe("Jira service", () => {
  it("bounds a stalled response body below the Gateway deadline", async () => {
    // Given a response body that does not complete.
    let started: (() => void) | undefined;
    const bodyStarted = new Promise<void>((resolve) => { started = resolve; });
    let body: ReadableStreamDefaultController<Uint8Array> | undefined;
    const { jira } = await service(async (url) => {
      if (url.endsWith("/myself")) return new Response("{}");
      return new Response(new ReadableStream<Uint8Array>({ start(controller) { body = controller; started?.(); } }));
    });
    await jira.connect({ siteUrl: SITE, deployment: "cloud", email: "me@acme.com", token: "t" });
    vi.useFakeTimers();
    try {
      const pending = jira.currentVersion({ key: "acme.atlassian.net/APP-12", url: `${SITE}/browse/APP-12` });
      const rejected = expect(pending).rejects.toThrow();
      await bodyStarted;
      // When the eight-second main deadline expires.
      await vi.advanceTimersByTimeAsync(8_000);
      // Then the caller finishes before the ten-second Gateway deadline.
      await rejected;
    } finally {
      body?.close();
      vi.useRealTimers();
    }
  });

  it.each([
    { siteUrl: SITE, deployment: "cloud", email: "me@acme.com", token: "cloud-token", api: "3", auth: `Basic ${Buffer.from("me@acme.com:cloud-token").toString("base64")}`, key: "acme.atlassian.net/APP-12" },
    { siteUrl: "https://jira.example:8443/jira", deployment: "server", email: "", token: "server-token", api: "2", auth: "Bearer server-token", key: "jira.example:8443/jira/APP-12" },
  ])("reads only updated with stored credentials for $deployment", async (connection) => {
    // Given an existing Desktop login.
    const calls: Array<{ url: string; init: Parameters<JiraFetch>[1] }> = [];
    const { jira } = await service(async (url, init) => {
      calls.push({ url, init });
      return new Response(JSON.stringify({ fields: { updated: "2026-11-06T10:00:00.000+0900" } }));
    });
    await jira.connect(connection);
    // When the provider resolves the matching resource.
    const result = await jira.currentVersion({ key: connection.key, url: `${connection.siteUrl}/browse/APP-12?focusedCommentId=123#comment-123` });
    // Then credentials stay in main and the request is narrow.
    expect(result).toEqual({ version: "2026-11-06T01:00:00.000Z" });
    expect(calls.at(-1)?.url).toBe(`${connection.siteUrl}/rest/api/${connection.api}/issue/APP-12?fields=updated`);
    expect(calls.at(-1)?.init.headers.Authorization).toBe(connection.auth);
    expect(calls.at(-1)?.init.redirect).toBe("error");
  });

  it.each([
    { key: "evil.example/APP-12", url: "https://evil.example/browse/APP-12" },
    { key: "acme.atlassian.net/APP-13", url: `${SITE}/browse/APP-12` },
    { key: "acme.atlassian.net/APP-12", url: `${SITE}/jira/browse/APP-12` },
    { key: "acme.atlassian.net/APP-12", url: `${SITE}:8443/browse/APP-12` },
    { key: "acme.atlassian.net/APP-12", url: "https://user@acme.atlassian.net/browse/APP-12" },
    { key: "acme.atlassian.net/APP-12/../APP-13", url: `${SITE}/browse/APP-13` },
  ])("rejects a mismatched resource before fetching: $url", async (request) => {
    // Given a login and a mismatched resource.
    let calls = 0;
    const { jira } = await service(async () => { calls++; return new Response("{}"); });
    await jira.connect({ siteUrl: SITE, deployment: "cloud", email: "me@acme.com", token: "t" });
    // When a version is requested.
    await expect(jira.currentVersion(request)).rejects.toThrow();
    // Then no additional authenticated HTTP request occurs.
    expect(calls).toBe(1);
  });

  it.each(["not-a-date", "2026-02-30T10:00:00.000Z", "2026-11-06", "2026-11-06T24:00:00Z"])("rejects invalid updated timestamp %s", async (updated) => {
    // Given an invalid response timestamp.
    const { jira } = await service(async () => new Response(JSON.stringify({ fields: { updated } })));
    await jira.connect({ siteUrl: SITE, deployment: "cloud", email: "me@acme.com", token: "t" });
    // When the matching resource is resolved, then the provider fails closed.
    await expect(jira.currentVersion({ key: "acme.atlassian.net/APP-12", url: `${SITE}/browse/APP-12` })).rejects.toThrow();
  });

  it.each([302, 401, 403, 404])("rejects HTTP %s without returning a version", async (status) => {
    // Given a successful login followed by an unsuccessful version response.
    const { jira } = await service(async (url) => new Response("{}", { status: url.endsWith("/myself") ? 200 : status }));
    await jira.connect({ siteUrl: SITE, deployment: "cloud", email: "me@acme.com", token: "t" });
    // When a version is requested, then redirects and authentication failures are rejected.
    await expect(jira.currentVersion({ key: "acme.atlassian.net/APP-12", url: `${SITE}/browse/APP-12` })).rejects.toThrow();
  });

  it("rejects oversized bodies while streaming", async () => {
    // Given a connected account and an oversized response.
    const { jira } = await service(async (url) => new Response(url.endsWith("/myself") ? "{}" : "x".repeat(65_537)));
    await jira.connect({ siteUrl: SITE, deployment: "cloud", email: "me@acme.com", token: "t" });
    // When a version is requested, then no version escapes the body limit.
    await expect(jira.currentVersion({ key: "acme.atlassian.net/APP-12", url: `${SITE}/browse/APP-12` })).rejects.toThrow();
  });

  it.each(["disconnect", "account change"])("cancels a pending body on %s", async (change) => {
    // Given a body that has started but has not completed.
    let started: (() => void) | undefined;
    const bodyStarted = new Promise<void>((resolve) => { started = resolve; });
    let body: ReadableStreamDefaultController<Uint8Array> | undefined;
    const { jira } = await service(async (url) => {
      if (url.endsWith("/myself")) return new Response("{}");
      return new Response(new ReadableStream<Uint8Array>({ start(controller) { body = controller; started?.(); } }));
    });
    await jira.connect({ siteUrl: SITE, deployment: "cloud", email: "me@acme.com", token: "t" });
    const pending = jira.currentVersion({ key: "acme.atlassian.net/APP-12", url: `${SITE}/browse/APP-12` });
    const rejected = expect(pending).rejects.toThrow();
    await bodyStarted;
    // When the login changes before the body arrives.
    if (change === "disconnect") await jira.disconnect();
    else await jira.connect({ siteUrl: SITE, deployment: "cloud", email: "other@acme.com", token: "new-token" });
    body?.enqueue(new TextEncoder().encode(JSON.stringify({ fields: { updated: "2026-11-06T10:00:00Z" } })));
    body?.close();
    // Then the old response cannot provide a version.
    await rejected;
  });

  it("verifies the token before saving it encrypted, then searches my open issues", async () => {
    const { calls, fetcher } = recorder({ "/myself": { status: 200, body: "{}" }, "/search/jql": { status: 200, body: searchJson } });
    const { directory, jira } = await service(fetcher);

    const status = await jira.connect({ siteUrl: `${SITE}/`, deployment: "cloud", email: "me@acme.com", token: "secret-token" });
    expect(status).toEqual({ connected: true, siteUrl: SITE, deployment: "cloud", email: "me@acme.com" });
    expect(JSON.stringify(status)).not.toContain("secret-token");
    expect(await readFile(join(directory, "jira-connection.json"), "utf8")).not.toContain("secret-token");
    expect((await readFile(join(directory, "jira-token.bin"))).toString()).toBe("enc:secret-token");

    const summary = await jira.summary();
    expect(summary.issues.map((issue) => issue.key)).toEqual(["APP-12"]);
    expect(calls[0]!.url).toBe(`${SITE}/rest/api/3/myself`);
    expect(calls[1]!.url).toContain(`${SITE}/rest/api/3/search/jql?jql=assignee%20%3D%20currentUser()`);
    expect(calls[1]!.authorization).toBe(`Basic ${Buffer.from("me@acme.com:secret-token").toString("base64")}`);
  });

  it("uses REST v2 and a bearer PAT for Server and ignores an email", async () => {
    const { calls, fetcher } = recorder({ "/myself": { status: 200, body: "{}" }, "/search?": { status: 200, body: searchJson } });
    const { jira } = await service(fetcher);
    await jira.connect({ siteUrl: "https://jira.corp.example/jira", deployment: "server", email: "ignored@corp", token: "pat" });
    await jira.summary();
    expect(calls[1]!.url).toContain("https://jira.corp.example/jira/rest/api/2/search?jql=");
    expect(calls[1]!.authorization).toBe("Bearer pat");
  });

  it("does not save anything when Jira rejects the token or encryption is unavailable", async () => {
    const rejected = await service(recorder({ "/myself": { status: 401, body: "" } }).fetcher);
    await expect(rejected.jira.connect({ siteUrl: SITE, deployment: "cloud", email: "me@acme.com", token: "bad" })).rejects.toThrow("인증에 실패");
    expect(await rejected.jira.status()).toEqual({ connected: false });

    const noCrypto = await service(recorder({ "/myself": { status: 200, body: "{}" } }).fetcher, { ...box, isEncryptionAvailable: () => false });
    await expect(noCrypto.jira.connect({ siteUrl: SITE, deployment: "cloud", email: "me@acme.com", token: "t" })).rejects.toThrow("OS 암호화");
    expect(await noCrypto.jira.status()).toEqual({ connected: false });
  });

  it("rejects a Cloud connection without an email and a bad issue key", async () => {
    const { jira } = await service(recorder({ "/myself": { status: 200, body: "{}" }, "/issue/": { status: 200, body: issueJson } }).fetcher);
    await expect(jira.connect({ siteUrl: SITE, deployment: "cloud", email: "", token: "t" })).rejects.toThrow("이메일");
    await jira.connect({ siteUrl: SITE, deployment: "cloud", email: "me@acme.com", token: "t" });
    await expect(jira.issuePrompt("APP-12/../../myself")).rejects.toThrow("키가 올바르지 않습니다");
    await expect(jira.issuePrompt("APP-12")).resolves.toContain("Login fails");
  });

  it("opens only ticket addresses of the connected site and forgets everything on disconnect", async () => {
    const { jira } = await service(recorder({ "/myself": { status: 200, body: "{}" } }).fetcher);
    await jira.connect({ siteUrl: SITE, deployment: "cloud", email: "me@acme.com", token: "t" });
    expect(await jira.isSiteUrl(`${SITE}/browse/APP-12`)).toBe(true);
    expect(await jira.isSiteUrl(`${SITE}/secure/admin`)).toBe(false);
    expect(await jira.isSiteUrl("https://evil.example/browse/APP-12")).toBe(false);
    await jira.disconnect();
    expect(await jira.status()).toEqual({ connected: false });
    expect(await jira.isSiteUrl(`${SITE}/browse/APP-12`)).toBe(false);
  });
});
