import { describe, expect, it } from "vitest";

import { CONTEXT_THREAD_METHOD, documentsFrom, fetchThreadDocuments, groupByKind, kindLabel, sourceLabel } from "../src/renderer/work/threadDocuments";

const resource = (overrides: Record<string, unknown>) => ({
  kind: "jira",
  key: "acme.atlassian.net/ABC-1",
  url: "https://acme.atlassian.net/browse/ABC-1",
  title: "ABC-1",
  source: "user",
  firstSeenAt: "2026-11-11T00:00:00Z",
  ...overrides,
});

describe("documentsFrom", () => {
  it("reads documents and drops entries without an https URL or a key", () => {
    const documents = documentsFrom({
      resources: [resource({}), resource({ url: "http://acme.atlassian.net/browse/ABC-2", key: "x" }), resource({ key: "" }), null, resource({ kind: "linear", key: "L-1", title: "" })],
    });
    expect(documents.map((document) => document.key)).toEqual(["acme.atlassian.net/ABC-1", "L-1"]);
    expect(documents[1]?.title).toBe("https://acme.atlassian.net/browse/ABC-1");
  });

  it("rejects an unknown response shape", () => {
    expect(() => documentsFrom({ items: [] })).toThrow("형식");
    expect(() => documentsFrom(null)).toThrow("형식");
  });
});

describe("labels and groups", () => {
  it("labels known kinds and shows unknown kinds as they are", () => {
    expect(kindLabel("notion")).toBe("Notion");
    expect(kindLabel("linear")).toBe("linear");
    expect(sourceLabel("agent")).toBe("AI");
    expect(sourceLabel("hook")).toBe("hook");
  });

  it("groups by kind in first-seen order", () => {
    const groups = groupByKind(documentsFrom({ resources: [resource({}), resource({ kind: "github", key: "a/b#1" }), resource({ key: "acme.atlassian.net/ABC-3" })] }));
    expect(groups.map((group) => [group.kind, group.documents.length])).toEqual([
      ["jira", 2],
      ["github", 1],
    ]);
  });
});

describe("fetchThreadDocuments", () => {
  it("asks the gateway for the thread", async () => {
    const sent: unknown[] = [];
    const documents = await fetchThreadDocuments(<T,>(method: string, params: unknown): Promise<T> => {
      sent.push([method, params]);
      return Promise.resolve({ resources: [resource({})] } as T);
    }, "thread-1");
    expect(sent).toEqual([[CONTEXT_THREAD_METHOD, { threadId: "thread-1" }]]);
    expect(documents).toHaveLength(1);
  });
});
