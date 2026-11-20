// 공급자와 모델 호출 없이, 응답 순서를 직접 바꾸어 연결·페이지·명령 경계를 검증한다.
import { describe, expect, it } from "vitest";
import { EMPTY_KERNEL_VIEW, KernelConnection, type KernelView } from "../src/renderer/kernel/kernelConnection";
import { JsonRpcClient } from "../src/renderer/rpc/jsonRpcClient";

interface Sent {
  id: number;
  method: string;
  params: {
    query?: { type: string; after?: string | null; limit?: number };
    command?: { type: string; mode?: string };
    idempotency_key?: string;
  };
}

function automation(revision = 7, mode: "off" | "observe" = "observe") {
  return {
    type: "issue_automation", status: "ready", mode, epoch: 1, revision,
    binding_id: null, binding: null, collecting: false, next_poll_after_ms: 300_000,
    last_attempt_at_ms: 100, last_success_at_ms: 90, last_failure: null, consecutive_failures: 0,
  };
}

function issuePage(ids = ["I_first"], revision = 7, nextAfter: string | null = "opaque-next") {
  return {
    type: "issues", status: "ready", revision, next_after: nextAfter,
    items: ids.map((id) => ({
      issue: {
        provider: "github", server_id: "github.com", repository_id: "R_repo", issue_id: id,
        project_scope: { workspace_dir: "/repo" },
      },
      binding_id: "origin", tracked: null, current: { status: "not_yet_observed" },
      presentation: {
        number: null, url: null, title: null,
        status: { code: "not_yet_observed", label: "서버 상태 " + id },
        reason: { code: "not_yet_observed", summary: "서버 이유 " + id },
      },
    })),
    groups: [{ id: "collection", label: "서버가 정한 묶음", item_indices: ids.map((_, index) => index) }],
  };
}

function harness() {
  const sent: Sent[] = [];
  const listeners = new Set<() => void>();
  const changed = () => { for (const listener of listeners) listener(); };
  const client = new JsonRpcClient((text) => { sent.push(JSON.parse(text) as Sent); changed(); });
  let view = EMPTY_KERNEL_VIEW;
  const connection = new KernelConnection(client, {
    getKernel: () => view,
    setKernel: (next: KernelView) => { view = next; changed(); },
  });
  const when = (condition: () => boolean): Promise<void> => {
    if (condition()) return Promise.resolve();
    return new Promise((resolve, reject) => {
      const check = () => {
        if (!condition()) return;
        clearTimeout(timeout);
        listeners.delete(check);
        resolve();
      };
      const timeout = setTimeout(() => {
        listeners.delete(check);
        reject(new Error("Expected issue request or state change did not arrive"));
      }, 2_000);
      listeners.add(check);
    });
  };
  const requests = (method: string) => sent.filter((request) => request.method === method);
  const queries = (type: string) => requests("akra/kernel/query").filter((request) => request.params.query?.type === type);
  const query = (type: string) => queries(type).at(-1);
  const reply = (request: Sent | undefined, result: unknown) => {
    if (!request) throw new Error("Request is missing");
    client.handleText(JSON.stringify({ jsonrpc: "2.0", id: request.id, result }));
  };
  const fail = (request: Sent | undefined, message = "read failed") => {
    if (!request) throw new Error("Request is missing");
    client.handleText(JSON.stringify({ jsonrpc: "2.0", id: request.id, error: { code: -32013, message } }));
  };
  const answerIssueReads = (page = issuePage(), state = automation(page.revision)) => {
    reply(query("issue_automation"), state);
    reply(query("issues"), page);
  };
  const answerAttach = async (kernel = "repo") => {
    const before = queries("issues").length;
    reply(requests("akra/kernel/attach").at(-1), { client_id: "1", protocol_version: 1, kernel });
    await when(() => queries("issues").length > before);
  };
  const openIssues = async (page = issuePage(), state = automation(page.revision)) => {
    connection.connectionChanged(true);
    const shown = connection.show("/repo", ["issues"]);
    await answerAttach();
    answerIssueReads(page, state);
    await shown;
  };
  return { connection, sent, requests, queries, query, reply, fail, when, answerIssueReads, answerAttach, openIssues, view: () => view };
}

describe("issue kernel connection", () => {
  it("uses the existing attach and reads only issue state and one page", async () => {
    const h = harness();
    const page = issuePage(["I_b", "I_a"]);
    page.groups[0]!.item_indices = [1, 0];
    await h.openIssues(page);
    expect(h.requests("akra/kernel/attach")).toHaveLength(1);
    expect(h.requests("akra/kernel/query").map((request) => request.params.query?.type)).toEqual(["issue_automation", "issues"]);
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
    expect(h.view().issues).toMatchObject({ page, pageNumber: 1, canGoBack: false, loading: false, stale: false, error: null });
    expect(h.view().planning).toBeNull();
  });

  it("changes interests without reattaching and does not send a late overview refresh", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const overview = h.connection.show("/repo");
    h.reply(h.requests("akra/kernel/attach")[0], { client_id: "1", protocol_version: 1, kernel: "repo" });
    await h.when(() => h.requests("akra/kernel/query").length === 4);
    const oldReads = h.requests("akra/kernel/query");
    const issues = h.connection.show("/repo", ["issues"]);
    h.answerIssueReads();
    for (const request of oldReads) h.fail(request, "old overview failed");
    await Promise.all([overview, issues]);
    expect(h.requests("akra/kernel/attach")).toHaveLength(1);
    expect(h.requests("akra/kernel/detach")).toHaveLength(0);
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
    expect(h.view().message).toBeNull();
    expect(h.view().issues.error).toBeNull();

    const openedOverview = h.connection.show("/repo", ["overview"]);
    const overviewReads = h.requests("akra/kernel/query").slice(-4);
    for (const request of overviewReads) {
      const type = request.params.query?.type;
      h.reply(request, type === "thread_activity" ? { type, threads: [] }
        : type === "auto_follow" ? { type, max_turns: 1, paused: false }
        : type === "planning" ? { type, status: "not_ready" }
        : { type, enabled: false });
    }
    await h.when(() => h.requests("akra/kernel/command").length === 1);
    h.reply(h.requests("akra/kernel/command")[0], { processed: true });
    await openedOverview;
    expect(h.requests("akra/kernel/attach")).toHaveLength(1);
    expect(h.view().issues.page).toBeNull();
    expect(h.requests("akra/kernel/command")[0]?.params.command?.type).toBe("refresh_planning");
  });

  it("keeps an issues state event newer than an in-flight state query", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const shown = h.connection.show("/repo", ["issues"]);
    await h.answerAttach();
    h.connection.handleEvent({ event: { ...automation(8, "off"), type: "issues" } });
    h.reply(h.query("issue_automation"), automation(7));
    h.reply(h.query("issues"), issuePage(["I_current"], 8));
    await shown;
    expect(h.view().issues.automation).toMatchObject({ type: "issue_automation", mode: "off", revision: 8 });
    expect(h.view().issues.page).toMatchObject({ type: "issues", revision: 8, items: [{ issue: { issue_id: "I_current" } }] });
    expect(h.queries("issues")).toHaveLength(1);
  });

  it("replaces pages, keeps previous cursors and coalesces repeated navigation", async () => {
    const h = harness();
    await h.openIssues();
    const next = h.connection.nextIssuesPage();
    const duplicate = h.connection.nextIssuesPage();
    const previousWhileLoading = h.connection.previousIssuesPage();
    expect(h.queries("issues")).toHaveLength(2);
    expect(h.query("issues")?.params.query).toEqual({ type: "issues", after: "opaque-next", limit: 50 });
    h.reply(h.query("issues"), issuePage(["I_second"], 7, null));
    await Promise.all([next, duplicate, previousWhileLoading]);
    expect(h.view().issues).toMatchObject({ pageNumber: 2, canGoBack: true, page: { items: [{ issue: { issue_id: "I_second" } }] } });
    const secondPage = h.view().issues.page;
    expect(secondPage?.status === "ready" && secondPage.items.length).toBe(1);

    const previous = h.connection.previousIssuesPage();
    expect(h.query("issues")?.params.query?.after).toBeNull();
    h.reply(h.query("issues"), issuePage());
    await previous;
    expect(h.view().issues).toMatchObject({ pageNumber: 1, canGoBack: false });
    const again = h.connection.nextIssuesPage();
    expect(h.query("issues")?.params.query?.after).toBe("opaque-next");
    h.reply(h.query("issues"), issuePage(["I_second"], 7, null));
    await again;
    expect(h.view().issues.pageNumber).toBe(2);
  });

  it("returns to the first page when a navigation response has a different revision", async () => {
    const h = harness();
    await h.openIssues();
    const next = h.connection.nextIssuesPage();
    h.reply(h.query("issues"), issuePage(["I_page_from_new_revision"], 8, null));
    await h.when(() => h.queries("issues").length === 3);
    expect(h.query("issues")?.params.query?.after).toBeNull();
    expect(h.view().issues).toMatchObject({ stale: true, canGoBack: false });
    h.reply(h.query("issues"), issuePage(["I_new_first"], 8, null));
    await next;
    expect(h.view().issues).toMatchObject({ pageNumber: 1, canGoBack: false, stale: false, page: { revision: 8, items: [{ issue: { issue_id: "I_new_first" } }] } });
  });

  it("does not forget a newer revision observed on a discarded next page", async () => {
    const h = harness();
    await h.openIssues();
    const next = h.connection.nextIssuesPage();
    h.reply(h.query("issues"), issuePage(["I_new_page"], 8));
    await h.when(() => h.queries("issues").length === 3);
    h.reply(h.query("issues"), issuePage(["I_old_first"], 7));
    await next;
    expect(h.view().issues.error).toContain("조회 중 바뀌었습니다");
    expect(h.view().issues.stale).toBe(true);
    expect(h.queries("issues")).toHaveLength(3);
  });

  it("invalidates an old cursor on an event and ignores its late error and finally", async () => {
    const h = harness();
    await h.openIssues();
    const next = h.connection.nextIssuesPage();
    const oldPage = h.query("issues");
    h.connection.handleEvent({ event: { ...automation(8), type: "issues" } });
    expect(h.queries("issues")).toHaveLength(3);
    expect(h.query("issues")?.params.query?.after).toBeNull();
    h.fail(oldPage, "old page failed");
    await next;
    expect(h.view().issues).toMatchObject({ loading: true, error: null, stale: true });
    h.reply(h.query("issues"), issuePage(["I_fresh"], 8));
    await h.when(() => !h.view().issues.loading);
    expect(h.view().issues).toMatchObject({ pageNumber: 1, stale: false, error: null, page: { revision: 8 } });
  });

  it("discards the old success and finally when a refresh supersedes a next page", async () => {
    const h = harness();
    await h.openIssues();
    const next = h.connection.nextIssuesPage();
    const oldPage = h.query("issues");
    const refreshed = h.connection.refreshIssues();
    h.reply(oldPage, issuePage(["I_old_second"], 7, null));
    await next;
    expect(h.view().issues.loading).toBe(true);
    expect(h.view().issues.pageNumber).toBe(1);
    h.answerIssueReads(issuePage(["I_refreshed"], 7, null));
    await refreshed;
    expect(h.view().issues).toMatchObject({ loading: false, pageNumber: 1, page: { items: [{ issue: { issue_id: "I_refreshed" } }] } });
  });

  it("ignores older first-page and state responses when refreshes complete out of order", async () => {
    const h = harness();
    await h.openIssues();
    const older = h.connection.refreshIssues();
    const oldState = h.query("issue_automation");
    const oldPage = h.query("issues");
    const latest = h.connection.refreshIssues();
    h.reply(oldState, automation(90, "off"));
    h.reply(oldPage, issuePage(["I_old_response"], 90, null));
    await older;
    expect(h.view().issues).toMatchObject({ loading: true, automation: { mode: "observe", revision: 7 } });
    h.answerIssueReads(issuePage(["I_latest_request"], 8, null));
    await latest;
    expect(h.view().issues).toMatchObject({ loading: false, page: { revision: 8, items: [{ issue: { issue_id: "I_latest_request" } }] } });
  });

  it("clears a prior workspace and ignores both late success and late failure", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const first = h.connection.show("/first", ["issues"]);
    await h.answerAttach("first");
    const oldState = h.query("issue_automation");
    const oldPage = h.query("issues");
    const second = h.connection.show("/second", ["issues"]);
    expect(h.view().issues.page).toBeNull();
    await h.answerAttach("second");
    h.reply(oldPage, issuePage(["I_private_to_first"], 99, null));
    h.fail(oldState, "first repository failed");
    await first;
    expect(h.view()).toMatchObject({ workspace: "/second", kernel: "second", issues: { page: null, loading: true, error: null } });
    h.answerIssueReads(issuePage(["I_second_repository"], 1, null));
    await second;
    expect(h.view().issues).toMatchObject({ loading: false, error: null, page: { revision: 1 } });
  });

  it("reconnects from the first page and never replays a mutation", async () => {
    const h = harness();
    await h.openIssues();
    const next = h.connection.nextIssuesPage();
    h.reply(h.query("issues"), issuePage(["I_second"], 7, null));
    await next;
    const oldMode = h.connection.setIssueAutomationMode("off");
    const oldCommand = h.requests("akra/kernel/command").at(-1);
    h.connection.connectionChanged(false);
    expect(h.view().issues).toMatchObject({ page: null, pageNumber: 1, commandPending: null });
    h.connection.connectionChanged(true);
    await h.answerAttach();
    expect(h.query("issues")?.params.query?.after).toBeNull();
    h.answerIssueReads(issuePage(["I_after_reconnect"], 8, null));
    await h.when(() => !h.view().issues.loading);
    const sync = h.connection.syncIssues();
    h.reply(oldCommand, { processed: false });
    await oldMode;
    expect(h.view().issues.commandPending).toBe("sync");
    expect(h.requests("akra/kernel/command")).toHaveLength(2);
    h.reply(h.requests("akra/kernel/command").at(-1), { processed: true });
    await h.when(() => h.queries("issue_automation").length === 3);
    h.answerIssueReads(issuePage(["I_after_sync"], 8, null));
    await sync;
    expect(h.view().issues).toMatchObject({ commandPending: null, error: null, pageNumber: 1 });
    expect(h.requests("akra/kernel/attach")).toHaveLength(2);
  });

  it("serializes mode and sync actions and uses the common sender key contract", async () => {
    const h = harness();
    await h.openIssues(issuePage(), automation(7, "off"));
    const mode = h.connection.setIssueAutomationMode("observe");
    await h.connection.setIssueAutomationMode("observe");
    await h.connection.syncIssues();
    expect(h.requests("akra/kernel/command")).toHaveLength(1);
    const command = h.requests("akra/kernel/command")[0];
    expect(command?.params.command).toEqual({ type: "set_issue_automation_mode", mode: "observe" });
    expect(command?.params.idempotency_key).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    h.reply(command, { processed: true });
    await h.when(() => h.queries("issues").length === 2);
    expect(h.view().issues.commandPending).toBe("mode");
    h.answerIssueReads(issuePage(["I_current"], 8, null), automation(8));
    await mode;
    expect(h.view().issues).toMatchObject({ commandPending: null, automation: { mode: "observe" } });
    const sync = h.connection.syncIssues();
    const syncCommand = h.requests("akra/kernel/command").at(-1);
    expect(syncCommand?.params).toEqual({ command: { type: "sync_issues" } });
    h.reply(syncCommand, { processed: true, rejection: { reason: "issue_automation_off", message: "off", next: [] } });
    await sync;
    expect(h.view().issues.error).toContain("관찰로 바꾼 뒤");
    expect(h.view().issues.commandPending).toBeNull();
    expect(h.requests("akra/kernel/command")).toHaveLength(2);
    expect(h.sent.every((request) => request.method.startsWith("akra/kernel/"))).toBe(true);
  });

  it("does not clear a status read error when the page succeeds", async () => {
    const h = harness();
    await h.openIssues();
    const refreshed = h.connection.refreshIssues();
    h.fail(h.query("issue_automation"), "status could not be read");
    h.reply(h.query("issues"), issuePage(["I_readable"], 7, null));
    await refreshed;
    expect(h.view().issues.error).toContain("status could not be read");
    expect(h.view().issues.page).toMatchObject({ items: [{ issue: { issue_id: "I_readable" } }] });
  });

  it("does not replace a newer status event with a stale query error", async () => {
    const h = harness();
    await h.openIssues();
    const refreshed = h.connection.refreshIssues();
    h.connection.handleEvent({ event: { ...automation(7, "off"), type: "issues" } });
    h.fail(h.query("issue_automation"), "old status failed");
    h.reply(h.query("issues"), issuePage());
    await refreshed;
    expect(h.view().issues).toMatchObject({ error: null, automation: { mode: "off" } });
  });

  it("keeps unavailable distinct from an empty ready page", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const shown = h.connection.show("/repo", ["issues"]);
    await h.answerAttach();
    h.reply(h.query("issue_automation"), { type: "issue_automation", status: "unavailable" });
    h.reply(h.query("issues"), { type: "issues", status: "unavailable" });
    await shown;
    expect(h.view().issues).toMatchObject({
      automation: { status: "unavailable" }, page: { status: "unavailable" }, loading: false, error: null,
    });
    const refreshed = h.connection.refreshIssues();
    h.answerIssueReads(issuePage([], 1, null));
    await refreshed;
    expect(h.view().issues.page).toMatchObject({ status: "ready", items: [], revision: 1 });
  });

  it.each([
    ["the previous Gateway contract", (page: ReturnType<typeof issuePage>) => {
      const { groups: _groups, ...legacy } = page;
      return { ...legacy, items: legacy.items.map(({ presentation: _presentation, ...item }) => item) };
    }],
    ["missing presentation", (page: ReturnType<typeof issuePage>) => ({ ...page, items: page.items.map(({ presentation: _presentation, ...item }) => item) })],
    ["out-of-range indices", (page: ReturnType<typeof issuePage>) => ({ ...page, groups: [{ id: "collection", label: "group", item_indices: [3] }] })],
    ["duplicate indices", (page: ReturnType<typeof issuePage>) => ({ ...page, groups: [{ id: "collection", label: "group", item_indices: [0, 0] }] })],
    ["omitted item indices", (page: ReturnType<typeof issuePage>) => ({ ...page, groups: [] })],
    ["invalid title", (page: ReturnType<typeof issuePage>) => ({ ...page, items: page.items.map((item) => ({ ...item, presentation: { ...item.presentation, title: { private: "not text" } } })) })],
    ["invalid issue number", (page: ReturnType<typeof issuePage>) => ({ ...page, items: page.items.map((item) => ({ ...item, presentation: { ...item.presentation, number: "12" } })) })],
  ] as const)("reports an update requirement for %s instead of rendering an invalid page", async (_name, malformed) => {
    const h = harness();
    h.connection.connectionChanged(true);
    const shown = h.connection.show("/repo", ["issues"]);
    await h.answerAttach();
    h.reply(h.query("issue_automation"), automation());
    h.reply(h.query("issues"), malformed(issuePage()));
    await shown;
    expect(h.view().issues.error).toContain("Gateway를 업데이트하거나 다시 시작");
    expect(h.view().issues.page).toBeNull();
    expect(h.view().issues.loading).toBe(false);
    expect(h.queries("issues")).toHaveLength(1);
  });

  it("resyncs only the issue interests and detaches when no interests remain", async () => {
    const h = harness();
    await h.openIssues();
    h.connection.handleEvent({ event: { type: "resync" } });
    expect(h.requests("akra/kernel/query").map((request) => request.params.query?.type)).toEqual([
      "issue_automation", "issues", "issue_automation", "issues",
    ]);
    const oldState = h.query("issue_automation");
    const oldPage = h.query("issues");
    const closed = h.connection.show("/repo", []);
    h.reply(h.requests("akra/kernel/detach").at(-1), {});
    await closed;
    h.reply(oldState, automation(12));
    h.reply(oldPage, issuePage(["I_after_close"], 12, null));
    h.connection.handleEvent({ event: { ...automation(13), type: "issues" } });
    await Promise.resolve();
    expect(h.view()).toEqual(EMPTY_KERNEL_VIEW);
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
  });
});
