import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, expect, it, vi } from "vitest";

import type { SecretBox } from "../src/main/jiraService";
import { isLinearIssueUrl, LinearService, type LinearFetch } from "../src/main/linearService";
import type { LinearFilters, LinearIssue } from "../src/shared/linear";

const DEFAULT_FILTERS: LinearFilters = { teamId: null, projectId: null, currentCycle: false };
const VIEWER = { id: "viewer-1", name: "Test operator" };
const ISSUE: LinearIssue = {
  id: "issue-1", identifier: "APP-12", title: "Fix login",
  url: "https://linear.app/acme/issue/APP-12/fix-login",
  priority: 1, updatedAt: "2026-11-18T10:00:00.000Z", branchName: "operator/app-12-fix-login",
  state: { id: "state-1", name: "In Progress", type: "started" },
  team: { id: "team-1", key: "APP", name: "Application" },
  project: { id: "project-1", name: "Authentication" },
  cycle: { id: "cycle-1", number: 12, name: "November", isActive: true },
};
const box: SecretBox = {
  isEncryptionAvailable: () => true,
  encryptString: (value) => Buffer.from(`encrypted:${Buffer.from(value).toString("base64")}`),
  decryptString: (value) => {
    if (!value.toString().startsWith("encrypted:")) throw new Error("not encrypted");
    return Buffer.from(value.toString().slice("encrypted:".length), "base64").toString();
  },
};

function response(data: unknown): Response {
  return new Response(JSON.stringify({ data }), { status: 200 });
}

function detail(overrides: Record<string, unknown> = {}) {
  return {
    ...ISSUE, description: "## Steps\nOpen the login page.",
    comments: {
      nodes: [{ body: "Still broken", createdAt: ISSUE.updatedAt, user: { name: "Kim" } }],
      pageInfo: { hasNextPage: false },
    },
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

type Call = { url: string; init: Parameters<LinearFetch>[1]; query: string; variables: Record<string, unknown> };
function recorder(handler?: (call: Call) => ReturnType<LinearFetch> | Response) {
  const calls: Call[] = [];
  const fetcher: LinearFetch = async (url, init) => {
    const parsed = JSON.parse(init.body) as { query: string; variables: Record<string, unknown> };
    const call = { url, init, ...parsed };
    calls.push(call);
    if (handler) return handler(call);
    if (parsed.query.includes("AkraLinearViewer")) return response({ viewer: VIEWER });
    if (parsed.query.includes("AkraLinearAssignedIssues")) return response({ viewer: { assignedIssues: { nodes: [ISSUE] } } });
    return response({ issue: detail() });
  };
  return { calls, fetcher };
}

const directories: string[] = [];
async function service(fetcher: LinearFetch = recorder().fetcher, secrets = box) {
  const directory = await mkdtemp(join(tmpdir(), "akra-linear-"));
  directories.push(directory);
  return { directory, linear: new LinearService(directory, secrets, fetcher) };
}

afterEach(async () => {
  vi.useRealTimers();
  await Promise.all(directories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

describe("Linear issue URL boundary", () => {
  it.each([
    ISSUE.url,
    "https://linear.app/acme/issue/APP-12",
    "https://linear.app/acme/issue/APP-12/changed-slug?x=1#comment",
  ])("accepts issue URL %s", (url) => {
    expect(isLinearIssueUrl(url)).toBe(true);
  });

  it.each([
    null, {}, 3, "", "http://linear.app/acme/issue/APP-12",
    "https://linear.app.evil.test/acme/issue/APP-12",
    "https://evil.test/linear.app/acme/issue/APP-12",
    "https://user:secret@linear.app/acme/issue/APP-12",
    "https://linear.app:444/acme/issue/APP-12",
    "https://linear.app/acme/project/APP-12",
    "https://linear.app/acme/issue/APP-12/slug/extra",
    "https://linear.app/acme/issue/APP-12/%2Fadmin",
    "https://linear.app/acme/issue/APP-12\\admin",
    "https://linear.app/acme/issue/APP-12\n",
    "https://linear.app/bad/../acme/issue/APP-12",
    "javascript:alert(1)",
  ])("rejects unsafe or non-issue URL %s", (url) => {
    expect(isLinearIssueUrl(url)).toBe(false);
  });
});

describe("Linear credentials and read-only transport", () => {
  it("verifies viewer and saves only encrypted credentials with viewer metadata", async () => {
    const { calls, fetcher } = recorder();
    const { directory, linear } = await service(fetcher);
    expect(await linear.status()).toEqual({ connected: false });
    expect(await linear.connect(" personal-key ")).toEqual({ connected: true, viewerName: VIEWER.name });
    expect(await readdir(directory)).toEqual(["linear-token.bin"]);
    const saved = await readFile(join(directory, "linear-token.bin"));
    expect(saved.toString()).not.toContain("personal-key");
    expect(saved.toString()).not.toContain(VIEWER.name);
    expect(JSON.parse(box.decryptString(saved))).toEqual({ token: "personal-key", viewer: VIEWER });
    expect(await new LinearService(directory, box, fetcher).status()).toEqual({ connected: true, viewerName: VIEWER.name });
    expect(calls[0]?.url).toBe("https://api.linear.app/graphql");
    expect(calls[0]?.init).toMatchObject({
      method: "POST", redirect: "error", credentials: "omit",
      headers: { Authorization: "personal-key", "Content-Type": "application/json" },
    });
    expect(calls[0]?.query.trim().startsWith("query ")).toBe(true);
    expect(calls[0]?.init.signal).toBeInstanceOf(AbortSignal);
  });

  it.each([null, {}, "", " ", "key\ninjected", "x".repeat(4_097)])("rejects invalid key input without a request: %s", async (input) => {
    const { fetcher, calls } = recorder();
    const { linear } = await service(fetcher);
    await expect(linear.connect(input)).rejects.toThrow();
    expect(calls).toHaveLength(0);
    expect(await linear.status()).toEqual({ connected: false });
  });

  it("does not request or save when encryption is unavailable", async () => {
    const { fetcher, calls } = recorder();
    const { directory, linear } = await service(fetcher, { ...box, isEncryptionAvailable: () => false });
    await expect(linear.connect("key")).rejects.toThrow("encryption");
    expect(calls).toHaveLength(0);
    expect(await readdir(directory)).toEqual([]);
  });

  it.each([
    {},
    { viewer: null },
    { viewer: { id: "viewer-1" } },
    { viewer: { id: 12, name: "Operator" } },
    { viewer: { id: "viewer-1", name: "" } },
  ])("requires a valid viewer before saving %j", async (data) => {
    const { directory, linear } = await service(async () => response(data));
    await expect(linear.connect("key")).rejects.toThrow();
    expect(await readdir(directory)).toEqual([]);
    expect(await linear.status()).toEqual({ connected: false });
  });

  it.each([401, 403, 429, 500, 302])("rejects HTTP %s and does not echo bodies", async (status) => {
    const { directory, linear } = await service(async () => new Response("secret-raw-server-error", { status }));
    const error = await linear.connect("secret-key").catch((value: Error) => value);
    expect(error).toBeInstanceOf(Error);
    expect(String(error)).not.toMatch(/secret-key|secret-raw-server-error/);
    expect(await readdir(directory)).toEqual([]);
  });

  it.each([
    '{"data":{"viewer":{"id":"v","name":"name"}},"errors":[{"message":"secret-raw-error"}]}',
    '{"data":{"viewer":{"id":"v","name":"name"}},"errors":null}',
    '{"errors":[{"message":"secret-raw-error"}]}',
    "not-json-secret-raw-error",
    "[]",
  ])("rejects malformed or partial GraphQL data", async (body) => {
    const { linear } = await service(async () => new Response(body));
    const error = await linear.connect("key").catch((value: Error) => value);
    expect(error).toBeInstanceOf(Error);
    expect(String(error)).not.toContain("secret-raw-error");
    expect(await linear.status()).toEqual({ connected: false });
  });

  it("rejects redirected responses even from an injected fetcher", async () => {
    const valid = response({ viewer: VIEWER });
    const { linear } = await service(async () => ({
      status: 200, redirected: true, headers: valid.headers, body: valid.body,
    }));
    await expect(linear.connect("key")).rejects.toThrow();
  });

  it("does not echo fetch errors that contain credentials", async () => {
    const { linear } = await service(async () => { throw new Error("Authorization: secret-key"); });
    const error = await linear.connect("secret-key").catch((value: Error) => value);
    expect(error).toBeInstanceOf(Error);
    expect(String(error)).not.toContain("secret-key");
  });

  it("bounds streamed bytes without trusting content-length and cancels overflow", async () => {
    let canceled = false;
    const { linear } = await service(async () => new Response(new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new Uint8Array((8 << 20) + 1));
      },
      cancel() { canceled = true; },
    })));
    await expect(linear.connect("key")).rejects.toThrow();
    expect(canceled).toBe(true);
  });

  it("rejects an advertised oversized body before reading it", async () => {
    const { linear } = await service(async () => new Response("{}", { headers: { "content-length": String((8 << 20) + 1) } }));
    await expect(linear.connect("key")).rejects.toThrow();
  });

  it("bounds the complete request even if fetch ignores abort", async () => {
    vi.useFakeTimers();
    const entered = deferred<AbortSignal>();
    const { linear } = await service(async (_url, init) => {
      entered.resolve(init.signal);
      return new Promise<never>(() => {});
    });
    const result = linear.connect("key");
    const rejected = expect(result).rejects.toThrow();
    const signal = await entered.promise;
    await vi.advanceTimersByTimeAsync(20_000);
    await rejected;
    expect(signal.aborted).toBe(true);
  });

  it("bounds body reads even if the stream stalls", async () => {
    const entered = deferred<void>();
    let canceled = false;
    const { linear } = await service(async () => new Response(new ReadableStream<Uint8Array>({
      pull() { entered.resolve(); },
      cancel() { canceled = true; },
    }, { highWaterMark: 0 })));
    vi.useFakeTimers();
    const result = linear.connect("key");
    const rejected = expect(result).rejects.toThrow();
    await entered.promise;
    await vi.advanceTimersByTimeAsync(20_000);
    await rejected;
    expect(canceled).toBe(true);
  });

  it("preserves the old key after a failed reconnect", async () => {
    const { fetcher } = recorder((call) => call.init.headers.Authorization === "good"
      ? response({ viewer: VIEWER }) : new Response("", { status: 401 }));
    const { linear, directory } = await service(fetcher);
    await linear.connect("good");
    const before = await readFile(join(directory, "linear-token.bin"));
    await expect(linear.connect("bad")).rejects.toThrow();
    expect(await readFile(join(directory, "linear-token.bin"))).toEqual(before);
    expect(await linear.status()).toEqual({ connected: true, viewerName: VIEWER.name });
  });

  it("treats corrupt encrypted storage as disconnected", async () => {
    const { linear, directory } = await service();
    await writeFile(join(directory, "linear-token.bin"), "invalid plaintext");
    expect(await linear.status()).toEqual({ connected: false });
    await expect(linear.summary(DEFAULT_FILTERS, false)).rejects.toThrow("not connected");
  });
});

describe("Linear cache, validation, and prompts", () => {
  it("requests only the latest thirty open assigned issues and filters cached IDs", async () => {
    const other: LinearIssue = {
      ...ISSUE, id: "issue-2", identifier: "APP-13",
      team: { ...ISSUE.team, id: "team-2" }, project: null,
      cycle: { ...ISSUE.cycle!, id: "cycle-2", isActive: false },
    };
    const { calls, fetcher } = recorder((call) => call.query.includes("AkraLinearViewer")
      ? response({ viewer: VIEWER }) : response({ viewer: { assignedIssues: { nodes: [ISSUE, other] } } }));
    const { linear } = await service(fetcher);
    await linear.connect("key");
    expect(await linear.summary(DEFAULT_FILTERS, false)).toEqual({
      issues: [ISSUE, other], totalCached: 2, limit: 30,
      teams: [{ id: "team-1", name: "Application" }, { id: "team-2", name: "Application" }],
      projects: [{ id: "project-1", name: "Authentication" }],
    });
    expect((await linear.summary({ ...DEFAULT_FILTERS, teamId: "team-1" }, false)).issues).toEqual([ISSUE]);
    expect((await linear.summary({ ...DEFAULT_FILTERS, projectId: "project-1" }, false)).issues).toEqual([ISSUE]);
    expect((await linear.summary({ ...DEFAULT_FILTERS, currentCycle: true }, false)).issues).toEqual([ISSUE]);
    expect((await linear.summary({ ...DEFAULT_FILTERS, teamId: "absent" }, false)).issues).toEqual([]);
    expect(calls).toHaveLength(2);
    expect(calls[1]!.query).toContain("first:30, orderBy:updatedAt, includeArchived:false");
    expect(calls[1]!.query).toContain('nin:["completed","canceled","duplicate"]');
    await linear.summary(DEFAULT_FILTERS, true);
    expect(calls).toHaveLength(3);
    expect(calls.every((call) => call.query.trim().startsWith("query "))).toBe(true);
  });

  it("deduplicates concurrent initial list fetches", async () => {
    const started = deferred<void>();
    const list = deferred<Response>();
    const { calls, fetcher } = recorder((call) => {
      if (call.query.includes("AkraLinearViewer")) return response({ viewer: VIEWER });
      started.resolve();
      return list.promise;
    });
    const { linear } = await service(fetcher);
    await linear.connect("key");
    const first = linear.summary(DEFAULT_FILTERS, false);
    const second = linear.summary(DEFAULT_FILTERS, false);
    await started.promise;
    list.resolve(response({ viewer: { assignedIssues: { nodes: [ISSUE] } } }));
    expect(await first).toEqual(await second);
    expect(calls).toHaveLength(2);
  });

  it("keeps open issues when the API also returns a duplicate state", async () => {
    const duplicate = { ...ISSUE, id: "duplicate-issue", state: { ...ISSUE.state, type: "duplicate" } };
    const { fetcher } = recorder(call => call.query.includes("AkraLinearViewer")
      ? response({ viewer: VIEWER }) : response({ viewer: { assignedIssues: { nodes: [ISSUE, duplicate] } } }));
    const { linear } = await service(fetcher);
    await linear.connect("key");
    expect((await linear.summary(DEFAULT_FILTERS, true)).issues).toEqual([ISSUE]);
  });

  it.each([
    { title: 42 }, { project: undefined }, { cycle: { ...ISSUE.cycle, isActive: "true" } },
    { state: null }, { updatedAt: "yesterday" }, { priority: 5 },
    { url: "https://evil.example/issue/APP-12" }, { branchName: undefined },
  ])("rejects invalid issue fields rather than caching partial data: %j", async (bad) => {
    const { fetcher } = recorder((call) => call.query.includes("AkraLinearViewer")
      ? response({ viewer: VIEWER }) : response({ viewer: { assignedIssues: { nodes: [{ ...ISSUE, ...bad }] } } }));
    const { linear } = await service(fetcher);
    await linear.connect("key");
    await expect(linear.summary(DEFAULT_FILTERS, false)).rejects.toThrow();
  });

  it.each([
    {}, null, { ...DEFAULT_FILTERS, currentCycle: "yes" }, { ...DEFAULT_FILTERS, teamId: {} },
  ])("rejects invalid filters before issuing a request: %j", async (input) => {
    const { calls, fetcher } = recorder();
    const { linear } = await service(fetcher);
    await linear.connect("key");
    await expect(linear.summary(input, false)).rejects.toThrow();
    expect(calls).toHaveLength(1);
  });

  it("builds handoff detail on demand with metadata and newest-first comments", async () => {
    const { fetcher, calls } = recorder();
    const { linear } = await service(fetcher);
    await linear.connect("key");
    await linear.summary(DEFAULT_FILTERS, false);
    expect(calls).toHaveLength(2);
    const result = await linear.issuePrompt(ISSUE.id);
    expect(result).toContain(ISSUE.identifier);
    expect(result).toContain(ISSUE.title);
    expect(result).toContain(ISSUE.url);
    expect(result).toContain(ISSUE.branchName);
    expect(result).toContain("## Steps");
    expect(result).toContain("Still broken");
    expect(calls[2]!.query).toContain("comments(first:10, orderBy:createdAt)");
    expect(calls[2]!.variables).toEqual({ id: ISSUE.id });
  });

  it("caps each comment at 2,000 characters including omission markers", async () => {
    const { fetcher } = recorder((call) => call.query.includes("AkraLinearViewer")
      ? response({ viewer: VIEWER }) : response({ issue: detail({
        description: null,
        comments: { nodes: [{ body: "c".repeat(3_000), createdAt: ISSUE.updatedAt, user: null }], pageInfo: { hasNextPage: true } },
      }) }));
    const { linear } = await service(fetcher);
    await linear.connect("key");
    const result = await linear.issuePrompt(ISSUE.id);
    const comment = result.split("Recent comments (newest first):\n")[1]!.split("\n[Older comments omitted]")[0]!;
    expect(comment).toHaveLength(2_000);
    expect(comment.endsWith("[Content omitted]")).toBe(true);
    expect(result).toContain("[Older comments omitted]");
  });

  it("caps the complete prompt at 20,000 inclusive and preserves header metadata", async () => {
    const { fetcher } = recorder((call) => call.query.includes("AkraLinearViewer")
      ? response({ viewer: VIEWER }) : response({ issue: detail({
        description: "d".repeat(40_000),
        comments: {
          nodes: Array.from({ length: 10 }, () => ({ body: "c".repeat(4_000), createdAt: ISSUE.updatedAt, user: { name: "Kim" } })),
          pageInfo: { hasNextPage: true },
        },
      }) }));
    const { linear } = await service(fetcher);
    await linear.connect("key");
    const result = await linear.issuePrompt(ISSUE.id);
    expect(result).toHaveLength(20_000);
    expect(result.endsWith("[Content omitted]")).toBe(true);
    for (const metadata of [ISSUE.identifier, ISSUE.title, ISSUE.url, ISSUE.branchName!]) expect(result).toContain(metadata);
  });

  it.each([null, "", "../issue", { id: ISSUE.id }, "x".repeat(129)])("rejects invalid detail IDs: %s", async (input) => {
    const { fetcher, calls } = recorder();
    const { linear } = await service(fetcher);
    await linear.connect("key");
    await expect(linear.issuePrompt(input)).rejects.toThrow();
    expect(calls).toHaveLength(1);
  });

  it.each([
    null,
    detail({ id: "different-issue" }),
    detail({ description: {} }),
    detail({ comments: { nodes: [{ body: 12, createdAt: ISSUE.updatedAt, user: null }], pageInfo: { hasNextPage: false } } }),
    detail({ comments: { nodes: [], pageInfo: {} } }),
  ])("rejects missing or malformed detail data", async (value) => {
    const { fetcher } = recorder((call) => call.query.includes("AkraLinearViewer")
      ? response({ viewer: VIEWER }) : response({ issue: value }));
    const { linear } = await service(fetcher);
    await linear.connect("key");
    await expect(linear.issuePrompt(ISSUE.id)).rejects.toThrow();
  });
});

describe("Linear lifecycle races", () => {
  it("invalidates old-account reads started while a replacement key is verified", async () => {
    const verifying = deferred<void>();
    const verification = deferred<Response>();
    const listing = deferred<void>();
    const oldList = deferred<Response>();
    const replacement = { ...ISSUE, id: "replacement-issue" };
    const { calls, fetcher } = recorder(call => {
      if (call.query.includes("AkraLinearViewer")) {
        if (call.init.headers.Authorization === "new-key") {
          verifying.resolve();
          return verification.promise;
        }
        return response({ viewer: VIEWER });
      }
      if (call.init.headers.Authorization === "old-key") {
        listing.resolve();
        return oldList.promise;
      }
      return response({ viewer: { assignedIssues: { nodes: [replacement] } } });
    });
    const { linear } = await service(fetcher);
    await linear.connect("old-key");
    const connection = linear.connect("new-key");
    await verifying.promise;
    const oldResult = linear.summary(DEFAULT_FILTERS, true).then(
      value => ({ value }), () => ({ canceled: true }),
    );
    await listing.promise;
    verification.resolve(response({ viewer: { ...VIEWER, name: "New account" } }));
    await connection;
    const next = linear.summary(DEFAULT_FILTERS, true);
    oldList.resolve(response({ viewer: { assignedIssues: { nodes: [ISSUE] } } }));
    expect((await next).issues).toEqual([replacement]);
    expect(await oldResult).toEqual({ canceled: true });
    expect(calls.filter(call => call.query.includes("AkraLinearAssignedIssues") && call.init.headers.Authorization === "new-key")).toHaveLength(1);
  });
  it("does not resurrect a pending connection after disconnect", async () => {
    const entered = deferred<void>();
    const pending = deferred<Response>();
    let signal: AbortSignal | undefined;
    const { linear, directory } = await service(async (_url, init) => {
      signal = init.signal;
      entered.resolve();
      return pending.promise;
    });
    const connect = linear.connect("late-key");
    const rejected = expect(connect).rejects.toThrow();
    await entered.promise;
    await linear.disconnect();
    expect(signal?.aborted).toBe(true);
    pending.resolve(response({ viewer: VIEWER }));
    await rejected;
    expect(await linear.status()).toEqual({ connected: false });
    expect(await readdir(directory)).toEqual([]);
    expect(await new LinearService(directory, box).status()).toEqual({ connected: false });
  });

  it("allows only the newest overlapping connect to save credentials", async () => {
    const firstEntered = deferred<void>();
    const firstResponse = deferred<Response>();
    const { fetcher } = recorder((call) => {
      if (call.init.headers.Authorization === "first") {
        firstEntered.resolve();
        return firstResponse.promise;
      }
      return response({ viewer: { ...VIEWER, name: "Second viewer" } });
    });
    const { linear, directory } = await service(fetcher);
    const first = linear.connect("first");
    const firstRejected = expect(first).rejects.toThrow();
    await firstEntered.promise;
    expect(await linear.connect("second")).toEqual({ connected: true, viewerName: "Second viewer" });
    firstResponse.resolve(response({ viewer: VIEWER }));
    await firstRejected;
    expect(JSON.parse(box.decryptString(await readFile(join(directory, "linear-token.bin")))).token).toBe("second");
    expect(await linear.status()).toEqual({ connected: true, viewerName: "Second viewer" });
  });

  it("clears cached issues and rejects stale list results after disconnect", async () => {
    const listEntered = deferred<void>();
    const listResponse = deferred<Response>();
    let listCalls = 0;
    const { fetcher } = recorder((call) => {
      if (call.query.includes("AkraLinearViewer")) return response({ viewer: VIEWER });
      listCalls++;
      if (listCalls === 1) return response({ viewer: { assignedIssues: { nodes: [ISSUE] } } });
      if (listCalls > 2) return response({ viewer: { assignedIssues: { nodes: [] } } });
      listEntered.resolve();
      return listResponse.promise;
    });
    const { linear, directory } = await service(fetcher);
    await linear.connect("key");
    await linear.summary(DEFAULT_FILTERS, false);
    const refresh = linear.summary(DEFAULT_FILTERS, true);
    const rejected = expect(refresh).rejects.toThrow();
    await listEntered.promise;
    await linear.disconnect();
    listResponse.resolve(response({ viewer: { assignedIssues: { nodes: [ISSUE] } } }));
    await rejected;
    await expect(linear.summary(DEFAULT_FILTERS, false)).rejects.toThrow("not connected");
    expect(await readdir(directory)).toEqual([]);
    await linear.connect("new-key");
    expect((await linear.summary(DEFAULT_FILTERS, false)).issues).toEqual([]);
    expect(listCalls).toBe(3);
  });

  it("rejects pending handoff detail after disconnect", async () => {
    const entered = deferred<void>();
    const pending = deferred<Response>();
    const { fetcher } = recorder((call) => {
      if (call.query.includes("AkraLinearViewer")) return response({ viewer: VIEWER });
      entered.resolve();
      return pending.promise;
    });
    const { linear } = await service(fetcher);
    await linear.connect("key");
    const result = linear.issuePrompt(ISSUE.id);
    const rejected = expect(result).rejects.toThrow();
    await entered.promise;
    await linear.disconnect();
    pending.resolve(response({ issue: detail() }));
    await rejected;
  });
});
