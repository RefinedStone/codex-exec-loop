import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

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
