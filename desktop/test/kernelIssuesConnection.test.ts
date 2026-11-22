// 공급자와 모델 호출 없이, 응답 순서를 직접 바꾸어 연결·페이지·명령 경계를 검증한다.
import { describe, expect, it, vi } from "vitest";
import { EMPTY_KERNEL_VIEW, KernelConnection, type KernelView, type IssueConfirmationBridge } from "../src/renderer/kernel/kernelConnection";
import { JsonRpcClient } from "../src/renderer/rpc/jsonRpcClient";
import type { CommandResponse, IssueAutomationAnswer, IssueAutonomyLevel, IssueAutonomyState, IssueEventReference } from "../src/renderer/kernel/protocol";
import type { ConfirmIssueAutomationModeRequest, IssueConfirmationResponse } from "../src/shared/issueConfirmation";

interface Sent {
  id: number;
  method: string;
  params: {
    query?: { type: string; after?: string | null; limit?: number; issue?: IssueEventReference };
    command?: { type: string; mode?: string; [key: string]: unknown };
    idempotency_key?: string;
  };
}

function automation(revision = 7, mode: "off" | "observe" = "observe"): Extract<IssueAutomationAnswer, { status: "ready" }> {
  return {
    type: "issue_automation", status: "ready", mode, epoch: 1, revision,
    binding_id: null, binding: null, collecting: false, next_poll_after_ms: 300_000,
    last_attempt_at_ms: 100, last_success_at_ms: 90, last_failure: null, consecutive_failures: 0,
  };
}

function issueReference(id = "I_first", workspace = "/repo"): IssueEventReference {
  return {
    provider: "github", server_id: "github.com", repository_id: "R_repo", issue_id: id,
    project_scope: { workspace_dir: workspace },
  };
}

function policyAnswer(version = "policy-v1", workspace = "/repo") {
  return {
    type: "issue_automation_policy", status: "ready", workspace, version,
    settings: [{
      key: "authority.autonomy_cap", label: "서버 상한 이름", value: "서버 유효값",
      origins: [{ scope: "user", label: "서버 사용자 출처" }, { scope: "project", label: "서버 저장소 출처" }],
      resolution: "minimum", resolution_summary: "서버가 설명한 제한 병합",
    }],
    files: [
      { scope: "user", label: "사용자 정책", path: "/user/issue-automation.toml", status: "readable", open_target: "user" },
      { scope: "project", label: "저장소 정책", path: workspace + "/.akra/issue-automation.toml", status: "missing", open_target: "project" },
    ],
  };
}

function report(revision: number) {
  return {
    revision, binding_id: "origin", policy_version: "policy-v1", assumed_mode: "propose", policy_blocked: false,
    total: 20, automatic_start: 12, human_approval: 5, excluded: 3,
    deferred: 0, needs_info: 0, unknown: 0, observation_only: 0, stale: 0, untriaged: 0, blocked: 0,
  };
}

function issuePage(ids = ["I_first"], revision = 7, nextAfter: string | null = "opaque-next") {
  return {
    type: "issues", status: "ready", revision, next_after: nextAfter,
    items: ids.map((id) => ({
      issue: issueReference(id),
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

function issueDetail(issue: IssueEventReference, revision = 7, body = "서버의 현재 공유 본문") {
  return {
    type: "issue", status: "ready", revision, report: report(revision),
    item: {
      ...issuePage([issue.issue_id], revision, null).items[0]!,
      issue,
      current: {
        status: "observed", remote_state: "open",
        labels: { completeness: "complete", items: [] }, assignees: { completeness: "complete", items: [] },
        author: null, label_actors: { completeness: "complete", items: [] }, linked_pull_request: "absent",
        content: {
          kind: "shared", basis: { field_hash: { version: 1, sha256: "0".repeat(64) }, provider_updated_at: "2026-11-20T00:00:00Z" },
          shared_copy: { title: "서버 공유 제목", body }, policy_version: "disclosure-v1",
        },
      },
      triage: {
        revision: 1, observation_revision: 1, tracked_revision: null, stale: false,
        eligibility: "eligible", readiness: "ready", risk_class: "normal", effective_mode: "observe",
        policy_version: "policy-v1", field_hash: { version: 1, sha256: "0".repeat(64) },
        basis: {
          version: 1, issue, source: "shared", disclosure_policy_version: "disclosure-v1",
          validation_policy_version: "disclosure-v1", evidence_sha256: "1".repeat(64), requirements_sha256: "2".repeat(64), pinned_source_commit: null,
        },
        gates: ["start", "spec", "plan", "delivery", "merge", "close"].map((gate) => ({ gate, action: "human", enabled: false, gate_revision: 1 })),
        reasons: [{ code: "observation_only", summary: "서버가 정한 판정 이유", next: [{ step: "ask_user" }], next_summary: "서버가 정한 다음 행동" }],
        observation: { observation_only: true, assumed_mode: "propose", disposition: "human_approval" },
      },
    },
  };
}

function overviewAnswer(type: string | undefined, maxTurns = 1): unknown {
  if (type === "thread_activity") return { type, threads: [] };
  if (type === "auto_follow") return { type, max_turns: maxTurns, paused: false };
  if (type === "planning") return { type, status: "not_ready" };
  return { type: "parallel", enabled: false };
}

function harness({ autoPolicy = true, confirmMode, confirmations, observeView }: { autoPolicy?: boolean; confirmMode?: (request: ConfirmIssueAutomationModeRequest) => Promise<IssueConfirmationResponse>; confirmations?: IssueConfirmationBridge; observeView?: (view: KernelView) => void } = {}) {
  let automaticPolicyReplies = autoPolicy;
  const sent: Sent[] = [];
  const listeners = new Set<() => void>();
  const changed = () => { for (const listener of listeners) listener(); };
  const client = new JsonRpcClient((text) => {
    const request = JSON.parse(text) as Sent;
    sent.push(request);
    changed();
    if (automaticPolicyReplies && request.params.query?.type === "issue_automation_policy") {
      queueMicrotask(() => client.handleText(JSON.stringify({ jsonrpc: "2.0", id: request.id, result: policyAnswer() })));
    }
  });
  let view = EMPTY_KERNEL_VIEW;
  const connection = new KernelConnection(client, {
    getKernel: () => view,
    setKernel: (next: KernelView) => { view = next; observeView?.(next); changed(); },
  }, confirmMode, confirmations);
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
  const answerPolicyBootstrap = async (policy = policyAnswer()) => {
    const beforeRevision = query("issue_automation_policy");
    reply(beforeRevision, policy);
    await when(() => query("issue_automation_policy") !== beforeRevision);
    reply(query("issue_automation_policy"), policy);
    await when(() => !view.issues.policyLoading);
  };
  const answerOverviewReads = (reads: Sent[], maxTurns = 1) => {
    for (const request of reads) reply(request, overviewAnswer(request.params.query?.type, maxTurns));
  };
  const openOverview = async () => {
    connection.connectionChanged(true);
    const shown = connection.show("/repo", ["overview"]);
    reply(requests("akra/kernel/attach").at(-1), { client_id: "1", protocol_version: 1, kernel: "repo" });
    await when(() => requests("akra/kernel/query").length === 4);
    answerOverviewReads(requests("akra/kernel/query"));
    await when(() => requests("akra/kernel/command").length === 1);
    reply(requests("akra/kernel/command")[0], { processed: true });
    await shown;
  };
  const openIssues = async (page = issuePage(), state = automation(page.revision), workspace = "/repo") => {
    // 최초 catalog 판을 알기 전 보낸 정책 요청의 보완 조회까지 끝낸 뒤 개별 지연 회귀를 시작한다.
    const previousAutomaticReplies = automaticPolicyReplies;
    automaticPolicyReplies = true;
    connection.connectionChanged(true);
    const shown = connection.show(workspace, ["issues"]);
    await answerAttach();
    answerIssueReads(page, state);
    await shown;
    await when(() => !view.issues.policyLoading);
    automaticPolicyReplies = previousAutomaticReplies;
  };
  return { connection, sent, requests, queries, query, reply, fail, when, answerIssueReads, answerAttach, answerPolicyBootstrap, answerOverviewReads, openOverview, openIssues, view: () => view };
}

describe("issue kernel connection", () => {
  it("uses the existing attach and reads issue state, one page and read-only policy", async () => {
    const h = harness();
    const page = issuePage(["I_b", "I_a"]);
    page.groups[0]!.item_indices = [1, 0];
    await h.openIssues(page);
    expect(h.requests("akra/kernel/attach")).toHaveLength(1);
    expect(h.requests("akra/kernel/query").map((request) => request.params.query?.type)).toEqual(["issue_automation", "issues", "issue_automation_policy", "issue_automation_policy"]);
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

  it.each(["refresh-first", "resync-first"] as const)("keeps one planning refresh through resync when four replies are delayed: %s", async (order) => {
    const h = harness();
    await h.openOverview();
    const refreshed = h.connection.refresh();
    const refreshReads = h.requests("akra/kernel/query").slice(-4);
    h.connection.handleEvent({ event: { type: "resync" } });
    const resyncReads = h.requests("akra/kernel/query").slice(-4);
    expect(h.requests("akra/kernel/query")).toHaveLength(12);

    if (order === "resync-first") {
      h.answerOverviewReads(resyncReads, 3);
      await h.when(() => h.view().autoFollow?.max_turns === 3);
      expect(h.requests("akra/kernel/command")).toHaveLength(1);
      h.answerOverviewReads(refreshReads, 2);
    } else {
      h.answerOverviewReads(refreshReads, 2);
      await h.when(() => h.requests("akra/kernel/command").length === 2);
      h.answerOverviewReads(resyncReads, 3);
    }
    await h.when(() => h.requests("akra/kernel/command").length === 2);
    h.reply(h.requests("akra/kernel/command").at(-1), { processed: true });
    await refreshed;
    expect(h.requests("akra/kernel/command").map((request) => request.params)).toEqual([
      { command: { type: "refresh_planning" } },
      { command: { type: "refresh_planning" } },
    ]);
    expect(h.view().autoFollow?.max_turns).toBe(3);
    expect(h.view().message).toBeNull();
  });

  it("coalesces superseded explicit refreshes without letting resync erase the latest intent", async () => {
    const h = harness();
    await h.openOverview();
    const first = h.connection.refresh();
    const firstReads = h.requests("akra/kernel/query").slice(-4);
    const latest = h.connection.refresh();
    const latestReads = h.requests("akra/kernel/query").slice(-4);
    h.connection.handleEvent({ event: { type: "resync" } });
    const resyncReads = h.requests("akra/kernel/query").slice(-4);
    h.answerOverviewReads(firstReads, 2);
    await first;
    expect(h.requests("akra/kernel/command")).toHaveLength(1);
    h.answerOverviewReads(resyncReads, 4);
    h.answerOverviewReads(latestReads, 3);
    await h.when(() => h.requests("akra/kernel/command").length === 2);
    h.reply(h.requests("akra/kernel/command").at(-1), { processed: true });
    await latest;
    expect(h.requests("akra/kernel/command")).toHaveLength(2);
    expect(h.view().autoFollow?.max_turns).toBe(4);
  });

  it.each(["interests", "workspace", "reconnect"] as const)("does not replay an old overview refresh after changing %s", async (change) => {
    const h = harness();
    await h.openOverview();
    const oldRefresh = h.connection.refresh();
    const oldReads = h.requests("akra/kernel/query").slice(-4);
    let fresh: Promise<void> | null = null;
    if (change === "interests") {
      const issues = h.connection.show("/repo", ["issues"]);
      h.answerIssueReads();
      await issues;
      fresh = h.connection.show("/repo", ["overview"]);
      expect(h.requests("akra/kernel/attach")).toHaveLength(1);
    } else {
      const before = h.requests("akra/kernel/query").length;
      if (change === "workspace") {
        fresh = h.connection.show("/other", ["overview"]);
      } else {
        h.connection.connectionChanged(false);
        h.connection.connectionChanged(true);
      }
      h.reply(h.requests("akra/kernel/attach").at(-1), {
        client_id: "2", protocol_version: 1, kernel: change === "workspace" ? "other" : "repo",
      });
      await h.when(() => h.requests("akra/kernel/query").length === before + 4);
    }
    const freshReads = h.requests("akra/kernel/query").slice(-4);
    // 관심 영역이 다시 overview여도 이전 관심/연결 세대의 성공 답은 명령을 만들지 못한다.
    h.answerOverviewReads(oldReads, 90);
    await oldRefresh;
    expect(h.requests("akra/kernel/command")).toHaveLength(1);
    h.answerOverviewReads(freshReads, 5);
    await h.when(() => h.requests("akra/kernel/command").length === 2);
    h.reply(h.requests("akra/kernel/command").at(-1), { processed: true });
    if (fresh) await fresh;
    expect(h.view().autoFollow?.max_turns).toBe(5);
    expect(h.requests("akra/kernel/command")).toHaveLength(2);
  });

  it("keeps a planning command failure visible after a newer resync query succeeds", async () => {
    const h = harness();
    await h.openOverview();
    const refreshed = h.connection.refresh();
    h.answerOverviewReads(h.requests("akra/kernel/query").slice(-4), 2);
    await h.when(() => h.requests("akra/kernel/command").length === 2);
    const command = h.requests("akra/kernel/command").at(-1);
    h.connection.handleEvent({ event: { type: "resync" } });
    h.answerOverviewReads(h.requests("akra/kernel/query").slice(-4), 3);
    await h.when(() => h.view().autoFollow?.max_turns === 3);
    h.fail(command, "planning refresh failed");
    await refreshed;
    expect(h.view().message).toContain("planning refresh failed");
    expect(h.view().autoFollow?.max_turns).toBe(3);
    expect(h.requests("akra/kernel/command")).toHaveLength(2);
  });

  it.each(["refresh", "interests"] as const)("ignores an old planning command failure after a newer %s", async (change) => {
    const h = harness();
    await h.openOverview();
    const oldRefresh = h.connection.refresh();
    h.answerOverviewReads(h.requests("akra/kernel/query").slice(-4), 2);
    await h.when(() => h.requests("akra/kernel/command").length === 2);
    const oldCommand = h.requests("akra/kernel/command").at(-1);
    let latest: Promise<void>;
    if (change === "interests") {
      const issues = h.connection.show("/repo", ["issues"]);
      h.answerIssueReads();
      await issues;
      latest = h.connection.show("/repo", ["overview"]);
    } else {
      latest = h.connection.refresh();
    }
    h.answerOverviewReads(h.requests("akra/kernel/query").slice(-4), 5);
    await h.when(() => h.requests("akra/kernel/command").length === 3);
    h.reply(h.requests("akra/kernel/command").at(-1), { processed: true });
    await latest;
    h.fail(oldCommand, "old planning command failed");
    await oldRefresh;
    expect(h.view().message).toBeNull();
    expect(h.view().autoFollow?.max_turns).toBe(5);
    expect(h.requests("akra/kernel/command")).toHaveLength(3);
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

  it("serializes native mode confirmation and sync actions and retains the sender key contract", async () => {
    const confirmations: ConfirmIssueAutomationModeRequest[] = [];
    let approve!: (response: IssueConfirmationResponse) => void;
    const h = harness({ confirmMode: (request) => {
      confirmations.push(request);
      return new Promise((resolve) => { approve = resolve; });
    } });
    await h.openIssues(issuePage(), automation(7, "off"));
    const mode = h.connection.setIssueAutomationMode("observe");
    await h.connection.setIssueAutomationMode("observe");
    await h.connection.syncIssues();
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
    expect(confirmations).toHaveLength(1);
    expect(confirmations[0]).toMatchObject({ workspace: "/repo", mode: "observe" });
    expect(confirmations[0]?.key).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    approve({ cancelled: false, response: { processed: true } });
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
    expect(h.requests("akra/kernel/command")).toHaveLength(1);
    expect(h.sent.every((request) => request.method.startsWith("akra/kernel/"))).toBe(true);
  });

  it("keeps Off unchanged when native confirmation is cancelled", async () => {
    const h = harness({ confirmMode: async () => ({ cancelled: true }) });
    await h.openIssues(issuePage(), automation(7, "off"));
    await h.connection.setIssueAutomationMode("observe");
    expect(h.view().issues).toMatchObject({ commandPending: null, error: null, automation: { mode: "off" } });
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
    expect(h.queries("issues")).toHaveLength(1);
  });

  it("requeries the committed mode and reports an unknown confirmation result without resubmitting", async () => {
    const confirmations: ConfirmIssueAutomationModeRequest[] = [];
    const h = harness({ confirmMode: async (request) => {
      confirmations.push(request);
      return { cancelled: false, outcome: "unknown" };
    } });
    await h.openIssues(issuePage(), automation(7, "off"));
    const mode = h.connection.setIssueAutomationMode("observe");
    await h.when(() => h.queries("issues").length === 2);
    expect(h.view().issues.error).toContain("처리 결과");
    expect(h.view().issues.commandPending).toBe("mode");
    h.answerIssueReads(issuePage(["I_committed"], 8, null), automation(8, "observe"));
    await mode;
    expect(h.view().issues).toMatchObject({ commandPending: null, automation: { mode: "observe", revision: 8 } });
    expect(h.view().issues.error).toContain("처리 결과를 확인하지 못했습니다");
    expect(confirmations).toHaveLength(1);
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
    expect(h.queries("issue_automation")).toHaveLength(2);
  });

  it.each(["unknown", "response"] as const)("ignores a late %s confirmation result after the workspace changes", async (outcome) => {
    let complete!: (response: IssueConfirmationResponse) => void;
    const h = harness({ confirmMode: () => new Promise((resolve) => { complete = resolve; }) });
    await h.openIssues(issuePage(), automation(7, "off"));
    const oldMode = h.connection.setIssueAutomationMode("observe");
    const replacement = h.connection.show("/second", ["issues"]);
    await h.answerAttach("second");
    h.answerIssueReads(issuePage(["I_second"], 9, null), automation(9, "off"));
    await replacement;
    const reads = h.queries("issues").length;
    complete(outcome === "unknown" ? { cancelled: false, outcome: "unknown" } : { cancelled: false, response: { processed: true } });
    await oldMode;
    expect(h.view()).toMatchObject({
      workspace: "/second", kernel: "second",
      issues: { error: null, commandPending: null, automation: { mode: "off", revision: 9 } },
    });
    expect(h.queries("issues")).toHaveLength(reads);
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
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
      "issue_automation", "issues", "issue_automation_policy", "issue_automation_policy", "issue_automation", "issues", "issue_automation_policy",
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

  it("uses server reference and policy workspace when attach starts from a worktree subfolder", async () => {
    const h = harness();
    const page = issuePage();
    await h.openIssues(page, automation(), "/repo-worktree/subfolder");
    expect(h.view().workspace).toBe("/repo-worktree/subfolder");
    expect(h.view().issues.policy).toEqual(policyAnswer());
    const selected = h.connection.selectIssue(page.items[0]!.issue);
    expect(h.query("issue")?.params.query).toEqual({ type: "issue", issue: issueReference() });
    const detail = issueDetail(issueReference());
    h.reply(h.query("issue"), detail);
    await selected;
    expect(h.view().issues.detail).toEqual(detail);
    expect(h.requests("akra/kernel/attach")).toHaveLength(1);
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
  });

  it.each(["success", "failure"] as const)("ignores old detail %s and finally while another row is loading", async (outcome) => {
    const h = harness();
    const page = issuePage(["I_A", "I_B"]);
    await h.openIssues(page);
    const first = h.connection.selectIssue(page.items[0]!.issue);
    const oldRequest = h.query("issue");
    const second = h.connection.selectIssue(page.items[1]!.issue);
    const currentRequest = h.query("issue");
    expect(h.view().issues).toMatchObject({ selectedIssue: issueReference("I_B"), detail: null, detailLoading: true, detailError: null });
    if (outcome === "success") h.reply(oldRequest, issueDetail(issueReference("I_A"), 99, "old-private-body"));
    else h.fail(oldRequest, "old detail failed");
    await first;
    expect(h.view().issues).toMatchObject({ detail: null, detailLoading: true, detailError: null });
    const detail = issueDetail(issueReference("I_B"));
    h.reply(currentRequest, detail);
    await second;
    expect(h.view().issues).toMatchObject({ detail, detailLoading: false, detailStale: false, detailError: null });
    expect(JSON.stringify(h.view().issues)).not.toContain("old-private-body");
  });

  it("clears a visible body immediately on selection and ignores a closed detail response", async () => {
    const h = harness();
    const page = issuePage(["I_A", "I_B"]);
    await h.openIssues(page);
    const first = h.connection.selectIssue(page.items[0]!.issue);
    h.reply(h.query("issue"), issueDetail(issueReference("I_A"), 7, "only-A-body"));
    await first;
    expect(JSON.stringify(h.view().issues.detail)).toContain("only-A-body");
    const second = h.connection.selectIssue(page.items[1]!.issue);
    const oldRequest = h.query("issue");
    expect(h.view().issues.detail).toBeNull();
    h.connection.clearIssueSelection();
    h.reply(oldRequest, issueDetail(issueReference("I_B"), 7, "closed-body"));
    await second;
    expect(h.view().issues).toMatchObject({ selectedIssue: null, detail: null, detailLoading: false, detailError: null, detailStale: false });
    expect(h.view().issues.page).toEqual(page);
    expect(JSON.stringify(h.view().issues)).not.toContain("closed-body");
  });

  it("does not select a reference that is absent from the current page", async () => {
    const h = harness();
    await h.openIssues();
    await h.connection.selectIssue(issueReference("I_foreign"));
    await h.connection.selectIssue(issueReference("I_first", "/another-workspace"));
    expect(h.queries("issue")).toHaveLength(0);
    expect(h.view().issues.selectedIssue).toBeNull();
  });

  it.each(["provider", "server_id", "repository_id", "issue_id", "workspace"] as const)(
    "rejects a detail response with a different %s without advancing the revision watermark",
    async (coordinate) => {
      const h = harness();
      await h.openIssues();
      const selected = h.connection.selectIssue(issueReference());
      const original = issueReference();
      const wrong: IssueEventReference = coordinate === "workspace"
        ? { ...original, project_scope: { ...original.project_scope, workspace_dir: "/foreign" } }
        : coordinate === "provider"
          ? { ...original, provider: "jira" }
          : { ...original, [coordinate]: "foreign" };
      h.reply(h.query("issue"), issueDetail(wrong, 99, "foreign-private-body"));
      await selected;
      expect(h.view().issues.detail).toBeNull();
      expect(h.view().issues.detailError).toContain("저장소 좌표");
      const refreshed = h.connection.refreshIssueDetail();
      h.reply(h.query("issue"), issueDetail(issueReference(), 7, "correct-body"));
      await refreshed;
      expect(h.view().issues).toMatchObject({ detailStale: false, detailLoading: false, detailError: null, detail: { revision: 7 } });
      expect(JSON.stringify(h.view().issues)).not.toContain("foreign-private-body");
    },
  );

  it("hides a detail on a newer event and retries an older pending response once", async () => {
    const h = harness();
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    const oldDetail = h.query("issue");
    h.connection.handleEvent({ event: { ...automation(8), type: "issues" } });
    expect(h.view().issues).toMatchObject({ detail: null, detailStale: true, detailLoading: true });
    expect(h.queries("issue")).toHaveLength(1);
    h.reply(oldDetail, issueDetail(issueReference(), 7, "old-private-body"));
    await h.when(() => h.queries("issue").length === 2);
    expect(h.view().issues.detail).toBeNull();
    h.reply(h.query("issues"), issuePage(["I_first"], 8, null));
    const fresh = issueDetail(issueReference(), 8, "fresh-current-body");
    h.reply(h.query("issue"), fresh);
    await selected;
    expect(h.view().issues).toMatchObject({ detail: fresh, detailStale: false, detailLoading: false });
    expect(JSON.stringify(h.view().issues)).not.toContain("old-private-body");
  });

  it("bounds retries when detail replies remain older than the latest catalog revision", async () => {
    const h = harness();
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    h.connection.handleEvent({ event: { ...automation(8), type: "issues" } });
    h.reply(h.query("issues"), issuePage(["I_first"], 8, null));
    h.reply(h.query("issue"), issueDetail(issueReference(), 7));
    await h.when(() => h.queries("issue").length === 2);
    h.reply(h.query("issue"), issueDetail(issueReference(), 7));
    await selected;
    expect(h.queries("issue")).toHaveLength(2);
    expect(h.view().issues).toMatchObject({ detail: null, detailLoading: false, detailStale: true });
    expect(h.view().issues.detailError).toContain("조회 중 바뀌었습니다");
  });

  it("invalidates an older page from a newer detail and preserves the server's whole report", async () => {
    const h = harness();
    const page = { ...issuePage(["I_first", "I_second"]), report: report(7) };
    await h.openIssues(page);
    const selected = h.connection.selectIssue(issueReference());
    const detail = issueDetail(issueReference(), 10);
    h.reply(h.query("issue"), detail);
    await selected;
    expect(h.view().issues).toMatchObject({ detail, stale: true, loading: true, canGoBack: false });
    expect(h.query("issues")?.params.query?.after).toBeNull();
    h.reply(h.query("issues"), { ...issuePage(["I_first"], 10, null), report: report(10) });
    await h.when(() => !h.view().issues.loading);
    expect(h.view().issues).toMatchObject({ stale: false, page: { report: report(10) }, detail: { report: report(10) } });
    expect(h.view().issues.detail).toEqual(detail);
    expect(h.queries("issue")).toHaveLength(1);
  });

  it("does not restore the old Shared body when the current detail becomes Unknown or Withheld", async () => {
    const h = harness();
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    h.reply(h.query("issue"), issueDetail(issueReference(), 7, "previous-shared-body"));
    await selected;
    for (const [revision, current] of [
      [8, { status: "unknown", reason: "provider_unavailable" }],
      [9, { status: "observed", content: { kind: "withheld", reason: "confirmation_required" } }],
      [10, { status: "not_yet_observed" }],
    ] as const) {
      h.connection.handleEvent({ event: { ...automation(revision), type: "issues" } });
      expect(h.view().issues.detail).toBeNull();
      const base = issueDetail(issueReference(), revision);
      const detail = { ...base, item: { ...base.item, current, triage: { ...base.item.triage, stale: true } } };
      h.reply(h.query("issue"), detail);
      h.reply(h.query("issues"), issuePage(["I_first"], revision, null));
      await h.when(() => !h.view().issues.detailLoading && !h.view().issues.loading);
      expect(h.view().issues.detail).toEqual(detail);
      expect(JSON.stringify(h.view().issues.detail)).not.toContain("previous-shared-body");
    }
  });


  it("keeps policy and detail failures independent when state and page reads succeed", async () => {
    const h = harness({ autoPolicy: false });
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    h.reply(h.query("issue"), issueDetail(issueReference()));
    await selected;
    const refreshed = h.connection.refreshIssues();
    h.fail(h.query("issue"), "detail unavailable");
    h.fail(h.query("issue_automation_policy"), "policy unavailable");
    await h.when(() => h.view().issues.detailError !== null && h.view().issues.policyError !== null);
    h.answerIssueReads();
    await refreshed;
    expect(h.view().issues).toMatchObject({ error: null, detail: null, policy: null, detailLoading: false, policyLoading: false });
    expect(h.view().issues.detailError).toContain("detail unavailable");
    expect(h.view().issues.policyError).toContain("policy unavailable");
    expect(h.view().issues.page).toEqual(issuePage());
    expect(h.queries("issue")).toHaveLength(2);
  });

  it.each(["success", "failure"] as const)("ignores superseded policy %s and finally without clearing a newer policy load", async (outcome) => {
    const h = harness({ autoPolicy: false });
    await h.openIssues();
    const older = h.connection.refreshIssueAutomationPolicy();
    const oldRequest = h.query("issue_automation_policy");
    const latest = h.connection.refreshIssueAutomationPolicy();
    const currentRequest = h.query("issue_automation_policy");
    if (outcome === "success") h.reply(oldRequest, policyAnswer("old-version", "/old-workspace"));
    else h.fail(oldRequest, "old policy error");
    await older;
    expect(h.view().issues).toMatchObject({ policy: null, policyLoading: true, policyError: null });
    const policy = policyAnswer("current-version");
    h.reply(currentRequest, policy);
    await latest;
    expect(h.view().issues).toMatchObject({ policy, policyLoading: false, policyError: null });
  });

  it("rereads policy on a newer issue event and preserves blocked causes, paths and server wording", async () => {
    const h = harness({ autoPolicy: false });
    await h.openIssues();
    const policyReads = h.queries("issue_automation_policy").length;
    h.connection.handleEvent({ event: { ...automation(8), type: "issues" } });
    expect(h.view().issues).toMatchObject({ policy: null, policyLoading: true });
    await h.when(() => h.queries("issue_automation_policy").length === policyReads + 1);
    const policy = {
      type: "issue_automation_policy", status: "policy_blocked", workspace: "/repo",
      blocked: { scope: "effective", field: "close.when", reason: "transition_disabled", summary: "서버가 보낸 차단 원인 그대로", next_summary: "서버가 보낸 해결 방법 그대로", file_scopes: ["user", "project"] },
      files: policyAnswer().files,
    };
    h.reply(h.query("issue_automation_policy"), policy);
    h.reply(h.query("issues"), { ...issuePage(["I_first"], 8, null), report: { ...report(8), policy_blocked: true, blocked: 20, automatic_start: 0, human_approval: 0, excluded: 0 } });
    await h.when(() => !h.view().issues.policyLoading && !h.view().issues.loading);
    expect(h.view().issues.policy).toEqual(policy);
    expect(h.view().issues.policyError).toBeNull();
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
    const unavailable = h.connection.refreshIssueAutomationPolicy();
    h.reply(h.query("issue_automation_policy"), { type: "issue_automation_policy", status: "unavailable" });
    await unavailable;
    expect(h.view().issues.policy).toEqual({ type: "issue_automation_policy", status: "unavailable" });
  });

  it("discards detail and policy responses from the prior workspace, including their finally blocks", async () => {
    const h = harness({ autoPolicy: false });
    await h.openIssues();
    const oldDetail = h.connection.selectIssue(issueReference());
    const oldDetailRequest = h.query("issue");
    const oldPolicy = h.connection.refreshIssueAutomationPolicy();
    const oldPolicyRequest = h.query("issue_automation_policy");
    const shown = h.connection.show("/second/worktree", ["issues"]);
    expect(h.view().issues).toMatchObject({ selectedIssue: null, detail: null, policy: null });
    await h.answerAttach("second-kernel");
    h.reply(oldDetailRequest, issueDetail(issueReference(), 99, "first-workspace-private-body"));
    h.fail(oldPolicyRequest, "first policy failed");
    await Promise.all([oldDetail, oldPolicy]);
    expect(h.view().issues).toMatchObject({ detail: null, detailError: null, policy: null, policyLoading: true, policyError: null });
    const page = issuePage(["I_second"], 1, null);
    page.items[0]!.issue = issueReference("I_second", "/second");
    h.answerIssueReads(page, automation(1));
    await h.answerPolicyBootstrap(policyAnswer("second", "/second"));
    await shown;
    expect(h.view().issues).toMatchObject({ page: { revision: 1 }, policy: { workspace: "/second" }, selectedIssue: null });
    expect(JSON.stringify(h.view().issues)).not.toContain("first-workspace-private-body");
  });

  it("does not revive selection or policy after issue interest is removed and restored", async () => {
    const h = harness({ autoPolicy: false });
    await h.openIssues();
    const oldDetail = h.connection.selectIssue(issueReference());
    const oldDetailRequest = h.query("issue");
    const oldPolicy = h.connection.refreshIssueAutomationPolicy();
    const oldPolicyRequest = h.query("issue_automation_policy");
    const closed = h.connection.show("/repo", []);
    h.reply(h.requests("akra/kernel/detach").at(-1), {});
    h.reply(oldDetailRequest, issueDetail(issueReference(), 9, "closed-private-body"));
    h.reply(oldPolicyRequest, policyAnswer("closed-policy"));
    await Promise.all([closed, oldDetail, oldPolicy]);
    expect(h.view()).toEqual(EMPTY_KERNEL_VIEW);
    const reopened = h.connection.show("/repo", ["issues"]);
    await h.answerAttach();
    h.answerIssueReads();
    await h.answerPolicyBootstrap(policyAnswer("reopened-policy"));
    await reopened;
    expect(h.view().issues).toMatchObject({ selectedIssue: null, detail: null, policy: { version: "reopened-policy" } });
    expect(h.queries("issue")).toHaveLength(1);
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
  });

  it("reconnects without replaying detail selection and rejects late detail and policy errors", async () => {
    const h = harness({ autoPolicy: false });
    await h.openIssues();
    const oldDetail = h.connection.selectIssue(issueReference());
    const oldDetailRequest = h.query("issue");
    const oldPolicy = h.connection.refreshIssueAutomationPolicy();
    const oldPolicyRequest = h.query("issue_automation_policy");
    h.connection.connectionChanged(false);
    h.connection.connectionChanged(true);
    await h.answerAttach();
    h.fail(oldDetailRequest, "disconnected detail failed");
    h.fail(oldPolicyRequest, "disconnected policy failed");
    await Promise.all([oldDetail, oldPolicy]);
    expect(h.view().issues).toMatchObject({ selectedIssue: null, detail: null, detailError: null, policy: null, policyLoading: true, policyError: null });
    h.answerIssueReads(issuePage(["I_reconnected"], 8, null), automation(8));
    await h.answerPolicyBootstrap(policyAnswer("reconnected-policy"));
    await h.when(() => !h.view().issues.loading && !h.view().issues.policyLoading);
    expect(h.view().issues.policy).toMatchObject({ version: "reconnected-policy" });
    expect(h.queries("issue")).toHaveLength(1);
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
  });

  it("keeps not-found and unavailable details distinct and clears selection when changing pages", async () => {
    const h = harness();
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    h.reply(h.query("issue"), { type: "issue", status: "not_found", revision: 7, report: report(7) });
    await selected;
    expect(h.view().issues.detail).toEqual({ type: "issue", status: "not_found", revision: 7, report: report(7) });
    const refreshed = h.connection.refreshIssueDetail();
    h.reply(h.query("issue"), { type: "issue", status: "unavailable" });
    await refreshed;
    expect(h.view().issues.detail).toEqual({ type: "issue", status: "unavailable" });
    const next = h.connection.nextIssuesPage();
    expect(h.view().issues).toMatchObject({ selectedIssue: null, detail: null });
    h.reply(h.query("issues"), issuePage(["I_next"], 7, null));
    await next;
    expect(h.view().issues.pageNumber).toBe(2);
  });

  it("rejects a report from another snapshot instead of mixing its counts with the detail", async () => {
    const h = harness();
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    h.reply(h.query("issue"), { ...issueDetail(issueReference()), report: report(8) });
    await selected;
    expect(h.view().issues.detail).toBeNull();
    expect(h.view().issues.detailError).toContain("표시 계약");
  });


  it("fences the old issue interest after an overview round trip on the same attachment", async () => {
    const h = harness({ autoPolicy: false });
    await h.openIssues();
    const oldDetail = h.connection.selectIssue(issueReference());
    const oldDetailRequest = h.query("issue");
    const oldPolicy = h.connection.refreshIssueAutomationPolicy();
    const oldPolicyRequest = h.query("issue_automation_policy");
    const overview = h.connection.show("/repo", ["overview"]);
    h.answerOverviewReads(h.requests("akra/kernel/query").slice(-4));
    await h.when(() => h.requests("akra/kernel/command").length === 1);
    h.reply(h.requests("akra/kernel/command")[0], { processed: true });
    await overview;
    const returned = h.connection.show("/repo", ["issues"]);
    h.reply(oldDetailRequest, issueDetail(issueReference(), 99, "old-interest-body"));
    h.fail(oldPolicyRequest, "old interest policy failed");
    await Promise.all([oldDetail, oldPolicy]);
    expect(h.view().issues).toMatchObject({ selectedIssue: null, detail: null, detailError: null, policy: null, policyLoading: true, policyError: null });
    h.answerIssueReads();
    await h.answerPolicyBootstrap(policyAnswer("current-interest-policy"));
    await returned;
    expect(h.requests("akra/kernel/attach")).toHaveLength(1);
    expect(h.requests("akra/kernel/detach")).toHaveLength(0);
    expect(h.view().issues.policy).toMatchObject({ version: "current-interest-policy" });
    expect(h.queries("issue")).toHaveLength(1);
    expect(JSON.stringify(h.view().issues)).not.toContain("old-interest-body");
  });


  it.each([
    { leader: "page", oldReply: "success" },
    { leader: "page", oldReply: "failure" },
    { leader: "detail", oldReply: "success" },
    { leader: "detail", oldReply: "failure" },
  ] as const)("refreshes policy when $leader leads the catalog and rejects old policy $oldReply", async ({ leader, oldReply }) => {
    const h = harness({ autoPolicy: false });
    await h.openIssues();
    const older = h.connection.refreshIssueAutomationPolicy();
    const oldPolicy = h.query("issue_automation_policy");
    const policyReads = h.queries("issue_automation_policy").length;
    if (leader === "page") {
      const next = h.connection.nextIssuesPage();
      h.reply(h.query("issues"), issuePage(["I_first"], 8, null));
      await h.when(() => h.queries("issues").length === 3);
      h.reply(h.query("issues"), issuePage(["I_first"], 8, null));
      await next;
    } else {
      const selected = h.connection.selectIssue(issueReference());
      h.reply(h.query("issue"), issueDetail(issueReference(), 8));
      await selected;
      h.reply(h.query("issues"), issuePage(["I_first"], 8, null));
      await h.when(() => !h.view().issues.loading);
    }
    await h.when(() => h.queries("issue_automation_policy").length === policyReads + 1);
    const newPolicy = h.query("issue_automation_policy");
    expect(h.view().issues).toMatchObject({ policy: null, policyLoading: true, policyError: null });
    h.connection.handleEvent({ event: { ...automation(8), type: "issues" } });
    await Promise.resolve();
    expect(h.queries("issue_automation_policy")).toHaveLength(policyReads + 1);
    if (oldReply === "success") h.reply(oldPolicy, policyAnswer("old-policy-P7"));
    else h.fail(oldPolicy, "old policy failed after catalog 8");
    await older;
    expect(h.view().issues).toMatchObject({ policy: null, policyLoading: true, policyError: null });
    h.reply(newPolicy, policyAnswer("current-policy-P8"));
    await h.when(() => !h.view().issues.policyLoading);
    expect(h.view().issues.policy).toEqual(policyAnswer("current-policy-P8"));
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
  });

  it("coalesces initial state and page revision advances into one corrective policy read", async () => {
    const h = harness({ autoPolicy: false });
    h.connection.connectionChanged(true);
    const shown = h.connection.show("/repo", ["issues"]);
    await h.answerAttach();
    const initialPolicy = h.query("issue_automation_policy");
    h.reply(h.query("issue_automation"), automation(7));
    h.reply(h.query("issues"), issuePage(["I_first"], 8, null));
    await h.when(() => h.queries("issue_automation_policy").length === 2);
    const currentPolicy = h.query("issue_automation_policy");
    h.reply(initialPolicy, policyAnswer("before-catalog"));
    await shown;
    expect(h.view().issues).toMatchObject({ policy: null, policyLoading: true, policyError: null });
    h.reply(currentPolicy, policyAnswer("catalog-8-policy"));
    await h.when(() => !h.view().issues.policyLoading);
    expect(h.view().issues.policy).toEqual(policyAnswer("catalog-8-policy"));
    expect(h.queries("issue_automation_policy")).toHaveLength(2);
  });

  it("starts and completes policy reads while a refreshed detail is still pending", async () => {
    const h = harness({ autoPolicy: false });
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    const oldDetail = h.query("issue");
    const policyReads = h.queries("issue_automation_policy").length;
    const refreshed = h.connection.refreshIssues();
    const pendingDetail = h.query("issue");
    expect(h.queries("issue_automation_policy")).toHaveLength(policyReads + 1);
    h.reply(h.query("issue_automation_policy"), policyAnswer("independent-policy"));
    await h.when(() => !h.view().issues.policyLoading);
    expect(h.view().issues).toMatchObject({ policy: policyAnswer("independent-policy"), detail: null, detailLoading: true });
    h.answerIssueReads();
    h.reply(pendingDetail, issueDetail(issueReference()));
    await refreshed;
    h.reply(oldDetail, issueDetail(issueReference(), 99, "old-late-detail"));
    await selected;
    expect(h.view().issues.policy).toEqual(policyAnswer("independent-policy"));
    expect(JSON.stringify(h.view().issues.detail)).not.toContain("old-late-detail");
  });

});


function autonomyState(level: IssueAutonomyLevel = "observe"): Extract<IssueAutonomyState, { status: "ready" }> {
  return {
    status: "ready" as const, requested_level: level, effective_level: level, cap: "propose" as const,
    revision: 1, policy_version: "runtime-policy-1", pending: false,
    choices: (["off", "observe", "propose"] as const).map((choice) => ({
      level: choice, enabled: true,
      requires_confirmation: ["off", "observe", "propose"].indexOf(choice) > ["off", "observe", "propose"].indexOf(level),
      reason: null,
    })),
  };
}

function decisionRequest(issue = issueReference(), id = "decision-1") {
  return {
    id, kind: "start_approval", subject: { kind: "issue", issue }, gate: "start", state: "open",
    revision: 1, basis: { policy_version: "policy-v1", gate_revision: 1, authorization_epoch: 0 },
    question: "공유된 질문만 표시합니다.", options: [
      { id: "approve", label: "시작 승인", effect: "작업을 시작할 수 있습니다." },
      { id: "hold", label: "보류", effect: "작업을 시작하지 않습니다." },
    ], recommended: "approve", due_at: null, on_timeout: "wait",
  };
}

function interactionDetail(issue = issueReference()) {
  return {
    ...issueDetail(issue), open_decisions: [decisionRequest(issue)], decision_history: [],
    control: { tracked: true, held: false, taken_over: false, cancelled: false, authorization_epoch: 0 },
    actions: ["hold_issue", "release_issue", "steer_issue", "take_over_issue", "retriage_issue", "cancel_issue_work", "track_issue", "untrack_issue"]
      .map((command) => ({ command, enabled: true, reason: null })),
    conversation: { kind: "draft", shared_copy: { title: "현재 공유 제목", body: "현재 공유 본문" } },
  };
}

async function selectInteraction(h: ReturnType<typeof harness>) {
  const selected = h.connection.selectIssue(issueReference());
  h.reply(h.query("issue"), interactionDetail());
  await selected;
}

async function completeInteractionRefresh(h: ReturnType<typeof harness>, before: number, state: Extract<IssueAutomationAnswer, { status: "ready" }> = { ...automation(), autonomy: autonomyState() }) {
  await h.when(() => h.queries("issues").length > before);
  h.answerIssueReads(issuePage(), state);
  if (h.view().issues.selectedIssue) h.reply(h.query("issue"), interactionDetail());
}

function deferredValue<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { resolve, promise };
}

function interventionResponse(request: Sent | undefined, delivery: "queued" | "sent" | "unknown" | null, changed = true): CommandResponse {
  const command = request?.params.command;
  if (!command || typeof command.command_id !== "string") throw new Error("Intervention request is missing");
  return { processed: true, result: { type: "issue_intervened", receipt: {
    command_id: command.command_id, issue: issueReference(), action: command.type,
    authorization_epoch: 0, delivery, changed,
  } } };
}

describe("issue decisions and interventions", () => {
  it("accepts policy-inherited autonomy when the wire response omits requested_level", async () => {
    const h = harness();
    const { requested_level: _requested, ...inherited } = autonomyState();
    const state = { ...automation(), autonomy: inherited };
    await h.openIssues(issuePage(), state);
    expect(Object.hasOwn(inherited, "requested_level")).toBe(false);
    expect(h.view().issues.automation).toEqual(state);
    expect(h.view().issues.error).toBeNull();
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
  });

  it.each(["unavailable", "policy_blocked"] as const)("accepts the generated %s autonomy state without a reason field", async (status) => {
    const h = harness();
    const autonomy: IssueAutonomyState = { status };
    const state = { ...automation(), autonomy };
    await h.openIssues(issuePage(), state);
    expect(h.view().issues.automation).toEqual(state);
    expect(h.view().issues.error).toBeNull();
    await h.connection.setIssueAutonomy("propose");
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
  });

  it("cancels a native decision without any kernel write", async () => {
    const confirmation = vi.fn<NonNullable<IssueConfirmationBridge["confirmIssueDecision"]>>(async () => ({ cancelled: true as const }));
    const h = harness({ confirmations: { confirmIssueDecision: confirmation } });
    await h.openIssues();
    await selectInteraction(h);
    await h.connection.answerIssueDecision("decision-1", "approve", "공유 메모");
    expect(confirmation).toHaveBeenCalledTimes(1);
    expect(confirmation.mock.calls[0]?.[0]).toMatchObject({ workspace: "/repo", decisionId: "decision-1", option: "approve", note: "공유 메모" });
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
    expect(h.view().issues.commandPending).toBeNull();
  });

  it("allows only one native decision submission while a click is pending", async () => {
    const native = deferredValue<IssueConfirmationResponse>();
    const confirmation = vi.fn<NonNullable<IssueConfirmationBridge["confirmIssueDecision"]>>(() => native.promise);
    const h = harness({ confirmations: { confirmIssueDecision: confirmation } });
    await h.openIssues();
    await selectInteraction(h);
    const before = h.queries("issues").length;
    const first = h.connection.answerIssueDecision("decision-1", "approve");
    await h.connection.answerIssueDecision("decision-1", "hold");
    expect(confirmation).toHaveBeenCalledTimes(1);
    expect(h.view().issues.commandPending).toBe("decision");
    native.resolve({ cancelled: false, response: { processed: true } });
    await completeInteractionRefresh(h, before);
    await first;
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
    expect(h.view().issues.notice).toBe("요청을 처리했습니다.");
  });

  it("re-reads after an unknown confirmation outcome without replaying the answer", async () => {
    const confirmation = vi.fn<NonNullable<IssueConfirmationBridge["confirmIssueDecision"]>>(async () => ({ cancelled: false as const, outcome: "unknown" as const }));
    const h = harness({ confirmations: { confirmIssueDecision: confirmation } });
    await h.openIssues();
    await selectInteraction(h);
    const before = h.queries("issues").length;
    const pending = h.connection.answerIssueDecision("decision-1", "approve");
    await completeInteractionRefresh(h, before);
    await pending;
    expect(confirmation).toHaveBeenCalledTimes(1);
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
    expect(h.view().issues.error).toContain("명령을 다시 보내기 전에");
  });

  it("cannot answer a removed, foreign, or stale decision", async () => {
    const confirmation = vi.fn<NonNullable<IssueConfirmationBridge["confirmIssueDecision"]>>(async () => ({ cancelled: true as const }));
    const h = harness({ confirmations: { confirmIssueDecision: confirmation } });
    await h.openIssues();
    await selectInteraction(h);
    await h.connection.answerIssueDecision("foreign-decision", "approve");
    const refresh = h.connection.refreshIssueDetail();
    await h.connection.answerIssueDecision("decision-1", "approve");
    h.reply(h.query("issue"), { ...interactionDetail(), open_decisions: [decisionRequest(issueReference("other-issue"))] });
    await refresh;
    expect(h.view().issues.detail).toBeNull();
    expect(h.view().issues.detailError).toContain("표시 계약");
    expect(confirmation).not.toHaveBeenCalled();
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
  });

  it("uses a native confirmation for an enabled rise and refuses a disabled cap choice", async () => {
    const confirmation = vi.fn<NonNullable<IssueConfirmationBridge["confirmIssueAutonomy"]>>(async () => ({ cancelled: true as const }));
    const h = harness({ confirmations: { confirmIssueAutonomy: confirmation } });
    const autonomy = autonomyState();
    await h.openIssues(issuePage(), { ...automation(), autonomy });
    await h.connection.setIssueAutonomy("propose");
    expect(confirmation).toHaveBeenCalledTimes(1);
    expect(confirmation.mock.calls[0]?.[0]).toMatchObject({ workspace: "/repo", level: "propose" });
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
    h.connection.handleEvent({ event: { ...automation(), type: "issues", autonomy: {
      ...autonomy, choices: autonomy.choices.map((choice) => ({ ...choice, enabled: choice.level !== "propose", reason: choice.level === "propose" ? "상한 제한" : null })),
    } } });
    await h.connection.setIssueAutonomy("propose");
    expect(confirmation).toHaveBeenCalledTimes(1);
  });

  it("sends a lower autonomy once with distinct command and transport identifiers", async () => {
    const h = harness();
    await h.openIssues(issuePage(), { ...automation(), autonomy: autonomyState("propose") });
    const before = h.queries("issues").length;
    const pending = h.connection.setIssueAutonomy("observe");
    const request = h.requests("akra/kernel/command")[0];
    expect(request?.params.command).toMatchObject({ type: "set_issue_autonomy", level: "observe" });
    expect(request?.params.command?.command_id).toMatch(/^[0-9a-f-]{36}$/);
    expect(request?.params.idempotency_key).not.toBe(request?.params.command?.command_id);
    h.reply(request, { processed: true });
    await completeInteractionRefresh(h, before);
    await pending;
    expect(h.requests("akra/kernel/command")).toHaveLength(1);
  });

  it("requires explicit cancel confirmation and keeps Unknown steer terminal in the UI", async () => {
    const h = harness();
    await h.openIssues();
    await selectInteraction(h);
    expect(await h.connection.interveneIssue("cancel_issue_work")).toBe("cancelled");
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
    const before = h.queries("issues").length;
    const pending = h.connection.interveneIssue("steer_issue", "새 지시");
    const request = h.requests("akra/kernel/command")[0];
    expect(request?.params.command).toMatchObject({ type: "steer_issue", issue: issueReference(), message: "새 지시" });
    h.reply(request, { processed: true, result: { type: "issue_intervened", receipt: {
      command_id: request?.params.command?.command_id, issue: issueReference(), action: "steer_issue", changed: true,
      authorization_epoch: 0, delivery: "unknown",
    } } });
    await completeInteractionRefresh(h, before);
    expect(await pending).toBe("unknown");
    expect(h.view().issues.notice).toContain("자동으로 다시 보내지 않습니다");
    expect(h.requests("akra/kernel/command")).toHaveLength(1);
    const cancelBefore = h.queries("issues").length;
    const cancelled = h.connection.interveneIssue("cancel_issue_work", undefined, true);
    const cancelRequest = h.requests("akra/kernel/command").at(-1);
    expect(cancelRequest?.params.command).toMatchObject({ type: "cancel_issue_work", issue: issueReference(), confirm: true });
    h.reply(cancelRequest, interventionResponse(cancelRequest, null, false));
    await completeInteractionRefresh(h, cancelBefore);
    expect(await cancelled).toBe("succeeded");
  });


  it.each([
    { delivery: "queued", changed: true }, { delivery: "queued", changed: false },
    { delivery: "sent", changed: true }, { delivery: "sent", changed: false },
  ] as const)("confirms the matching $delivery steer receipt even when changed=$changed", async ({ delivery, changed }) => {
    const h = harness();
    await h.openIssues();
    await selectInteraction(h);
    const before = h.queries("issues").length;
    const pending = h.connection.interveneIssue("steer_issue", "성공이 확인되면 비울 지시");
    const request = h.requests("akra/kernel/command").at(-1);
    h.reply(request, interventionResponse(request, delivery, changed));
    await completeInteractionRefresh(h, before);
    expect(await pending).toBe("succeeded");
    expect(h.requests("akra/kernel/command")).toHaveLength(1);
  });

  it.each(["rejected", "failure", "connection"] as const)("returns a non-success outcome for %s without replaying the intervention", async (kind) => {
    const h = harness();
    await h.openIssues();
    await selectInteraction(h);
    const before = h.queries("issues").length;
    const pending = h.connection.interveneIssue("steer_issue", "실패해도 보존할 지시");
    const request = h.requests("akra/kernel/command").at(-1);
    if (kind === "connection") h.fail(request, "Gateway 연결 결과를 확인하지 못했습니다.");
    else h.reply(request, kind === "rejected"
      ? { processed: false, rejection: { reason: "issue_automation_off", message: "수집이 꺼졌습니다.", next: [] } }
      : { processed: false, failure: "작업자 응답을 확인하지 못했습니다." });
    await completeInteractionRefresh(h, before);
    expect(await pending).toBe(kind === "rejected" ? "rejected" : "failed");
    expect(h.view().issues.error).toContain("이슈 요청을 완료하지 못했습니다");
    expect(h.view().issues.commandPending).toBeNull();
    expect(h.requests("akra/kernel/command")).toHaveLength(1);
  });

  it.each(["absent", "command", "action", "issue", "delivery"] as const)("keeps an unconfirmed %s receipt distinct from successful delivery", async (kind) => {
    const h = harness();
    await h.openIssues();
    await selectInteraction(h);
    const before = h.queries("issues").length;
    const pending = h.connection.interveneIssue("steer_issue", "영수증을 확인할 때까지 보존할 지시");
    const request = h.requests("akra/kernel/command").at(-1);
    const response = interventionResponse(request, "queued");
    if (response.result?.type !== "issue_intervened") throw new Error("Intervention receipt is missing");
    const receipt = response.result.receipt;
    h.reply(request, kind === "absent" ? { processed: true } : {
      ...response, result: { type: "issue_intervened", receipt: {
        ...receipt,
        ...(kind === "command" ? { command_id: "another-command" } : {}),
        ...(kind === "action" ? { action: "cancel_issue_work" } : {}),
        ...(kind === "issue" ? { issue: issueReference("another-issue") } : {}),
        ...(kind === "delivery" ? { delivery: null } : {}),
      } },
    });
    await completeInteractionRefresh(h, before);
    expect(await pending).toBe("unknown");
    expect(h.view().issues.notice).toContain("자동으로 다시 보내지 않습니다");
    expect(h.requests("akra/kernel/command")).toHaveLength(1);
  });

  it("does not report a successful old submission after the selected detail lifetime is replaced", async () => {
    const h = harness();
    await h.openIssues();
    await selectInteraction(h);
    const pending = h.connection.interveneIssue("steer_issue", "이전 선택 수명의 지시");
    const request = h.requests("akra/kernel/command").at(-1);
    h.connection.connectionChanged(false);
    h.connection.connectionChanged(true);
    await h.answerAttach();
    h.answerIssueReads();
    await h.when(() => !h.view().issues.loading);
    await selectInteraction(h);
    const current = h.view().issues.detail;
    h.reply(request, interventionResponse(request, "sent"));
    expect(await pending).toBe("unavailable");
    expect(h.view().issues.detail).toEqual(current);
    expect(h.requests("akra/kernel/command")).toHaveLength(1);
  });

  it("does not send an intervention without an available selected detail", async () => {
    const h = harness();
    expect(await h.connection.interveneIssue("steer_issue", "아직 연결되지 않은 지시")).toBe("unavailable");
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
  });

  it("re-reads a changed autonomy cap after confirmation failure without another submission", async () => {
    const confirmation = vi.fn<NonNullable<IssueConfirmationBridge["confirmIssueAutonomy"]>>(async () => ({
      cancelled: false as const,
      response: { processed: false, failure: "자율 수준 상한이 확인 중 바뀌었습니다." },
    }));
    const h = harness({ confirmations: { confirmIssueAutonomy: confirmation } });
    await h.openIssues(issuePage(), { ...automation(), autonomy: autonomyState() });
    const before = h.queries("issues").length;
    const changed = h.connection.setIssueAutonomy("propose");
    const fresh: Extract<IssueAutonomyState, { status: "ready" }> = {
      ...autonomyState(), cap: "observe" as const, revision: 2, policy_version: "runtime-policy-2",
      choices: autonomyState().choices.map((choice) => ({
        ...choice, enabled: choice.level !== "propose", reason: choice.level === "propose" ? "현재 정책 상한입니다." : null,
      })),
    };
    await completeInteractionRefresh(h, before, { ...automation(), autonomy: fresh });
    await changed;
    expect(h.view().issues.automation).toMatchObject({ autonomy: fresh });
    expect(h.view().issues.error).toContain("상한이 확인 중 바뀌었습니다");
    expect(h.view().issues.commandPending).toBeNull();
    await h.connection.setIssueAutonomy("propose");
    expect(confirmation).toHaveBeenCalledTimes(1);
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
  });

  it.each(["steer", "confirmation"] as const)("retains an Unknown %s notice across same-kernel reconnect without replay", async (kind) => {
    const confirmation = vi.fn<NonNullable<IssueConfirmationBridge["confirmIssueDecision"]>>(async () => ({ cancelled: false as const, outcome: "unknown" as const }));
    const h = harness({ confirmations: { confirmIssueDecision: confirmation } });
    await h.openIssues();
    await selectInteraction(h);
    const before = h.queries("issues").length;
    const pending = kind === "steer"
      ? h.connection.interveneIssue("steer_issue", "공유된 추가 지시")
      : h.connection.answerIssueDecision("decision-1", "approve");
    if (kind === "steer") {
      h.reply(h.requests("akra/kernel/command").at(-1), {
        processed: true, result: { type: "issue_intervened", receipt: { delivery: "unknown" } },
      });
    }
    await completeInteractionRefresh(h, before);
    await pending;
    const sentBefore = h.requests("akra/kernel/command").length;
    const confirmedBefore = confirmation.mock.calls.length;
    h.connection.connectionChanged(false);
    h.connection.connectionChanged(true);
    await h.answerAttach();
    h.answerIssueReads();
    await h.when(() => !h.view().issues.loading);
    expect(h.view().issues.selectedIssue).toBeNull();
    expect(h.view().issues.notice).toContain("확인하지 못했습니다");
    expect(h.requests("akra/kernel/command")).toHaveLength(sentBefore);
    expect(confirmation).toHaveBeenCalledTimes(confirmedBefore);

    h.connection.connectionChanged(false);
    h.connection.connectionChanged(true);
    await h.answerAttach("another-kernel");
    h.answerIssueReads();
    await h.when(() => !h.view().issues.loading);
    expect(h.view().issues.notice).toBeNull();
    expect(h.requests("akra/kernel/command")).toHaveLength(sentBefore);
    expect(confirmation).toHaveBeenCalledTimes(confirmedBefore);
  });

  it("opens only a fresh conversation target through a read and never takes over", async () => {
    const h = harness();
    await h.openIssues();
    await selectInteraction(h);
    const pending = h.connection.readIssueConversation();
    h.reply(h.query("issue"), { ...interactionDetail(), conversation: { kind: "thread", thread_id: "verified-worker-thread" } });
    expect(await pending).toEqual({ kind: "thread", thread_id: "verified-worker-thread" });
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
    expect(h.requests("turn/start")).toHaveLength(0);
    expect(h.requests("turn/steer")).toHaveLength(0);
  });

  it("refreshes the same notification target without clearing its selection", async () => {
    const views: KernelView[] = [];
    const h = harness({ observeView: (view) => views.push(view) });
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    h.reply(h.query("issue"), issueDetail(issueReference()));
    await selected;
    const before = views.length;
    const readCount = h.queries("issue").length;
    const reopened = h.connection.selectIssueFromAttention({
      workspace: "/repo", kernel: "repo", decisionId: "decision-next", issue: issueReference(),
    });
    expect(h.queries("issue")).toHaveLength(readCount + 1);
    expect(views.slice(before).every((view) => view.issues.selectedIssue !== null)).toBe(true);
    expect(h.view().issues).toMatchObject({ selectedIssue: issueReference(), detail: null, detailStale: true, detailLoading: true });
    h.reply(h.query("issue"), issueDetail(issueReference(), 7, "fresh notification detail"));
    await reopened;
    expect(h.view().issues.detail).toEqual(issueDetail(issueReference(), 7, "fresh notification detail"));
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
  });

  it.each(["provider", "server", "repository", "issue", "workspace"] as const)(
    "clears the old notification selection when its %s coordinate changes",
    async (coordinate) => {
      const views: KernelView[] = [];
      const h = harness({ observeView: (view) => views.push(view) });
      await h.openIssues();
      const selected = h.connection.selectIssue(issueReference());
      h.reply(h.query("issue"), issueDetail(issueReference()));
      await selected;
      const oldRead = h.connection.refreshIssueDetail();
      const oldRequest = h.query("issue");
      const other: IssueEventReference = {
        ...issueReference(),
        ...(coordinate === "provider" ? { provider: "jira" as const } : {}),
        ...(coordinate === "server" ? { server_id: "github.example" } : {}),
        ...(coordinate === "repository" ? { repository_id: "R_other" } : {}),
        ...(coordinate === "issue" ? { issue_id: "I_other" } : {}),
        ...(coordinate === "workspace" ? { project_scope: { workspace_dir: "/other" } } : {}),
      };
      const before = views.length;
      const opened = h.connection.selectIssueFromAttention({
        workspace: "/repo", kernel: "repo", decisionId: "other-decision", issue: other,
      });
      const currentRequest = h.query("issue");
      expect(views.slice(before).some((view) => view.issues.selectedIssue === null)).toBe(true);
      expect(h.view().issues).toMatchObject({ selectedIssue: other, detail: null, detailLoading: true });
      expect(currentRequest?.params.query?.issue).toEqual(other);
      h.reply(oldRequest, issueDetail(issueReference(), 99, "old notification body"));
      await oldRead;
      expect(h.view().issues.detail).toBeNull();
      h.reply(currentRequest, issueDetail(other));
      await opened;
      expect(h.view().issues.detail).toEqual(issueDetail(other));
      expect(JSON.stringify(h.view().issues)).not.toContain("old notification body");
      expect(h.requests("akra/kernel/command")).toHaveLength(0);
    },
  );

  it.each(["workspace", "kernel"] as const)("ignores notification navigation from another %s", async (field) => {
    const h = harness();
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    h.reply(h.query("issue"), issueDetail(issueReference()));
    await selected;
    const reads = h.queries("issue").length;
    await h.connection.selectIssueFromAttention({
      workspace: field === "workspace" ? "/other" : "/repo",
      kernel: field === "kernel" ? "other" : "repo",
      decisionId: "foreign-decision", issue: issueReference(),
    });
    expect(h.queries("issue")).toHaveLength(reads);
    expect(h.view().issues.selectedIssue).toEqual(issueReference());
    expect(h.view().issues.detail).toEqual(issueDetail(issueReference()));
  });

  it("keeps only metadata for background decisions and ignores duplicate recorded/direct events", async () => {
    const h = harness();
    h.connection.connectionChanged(true);
    const shown = h.connection.show("/repo", ["issue_attention"]);
    h.reply(h.requests("akra/kernel/attach")[0], { client_id: "1", protocol_version: 1, kernel: "repo" });
    await shown;
    expect(h.requests("akra/kernel/query")).toHaveLength(0);
    const request = decisionRequest();
    const fact = { decision_id: request.id, kind: request.kind, subject: request.subject, gate: request.gate, basis: request.basis, state: "open" };
    h.connection.handleEvent({ event: { type: "issue_decision", decision: fact, revision: 8 } });
    const attention = h.view().issueAttention;
    h.connection.handleEvent({ event: { type: "recorded", sequence: 1, recorded_at: "now", event: { kind: "issue_decision_requested", decision: fact, revision: 8 } } });
    expect(h.view().issueAttention).toEqual(attention);
    expect(attention).toEqual([{ workspace: "/repo", kernel: "repo", decisionId: "decision-1", issue: issueReference() }]);
    expect(JSON.stringify(attention)).not.toContain(request.question);
    expect(h.requests("akra/kernel/query")).toHaveLength(0);
    h.connection.handleEvent({ event: { type: "issue_decision", decision: { ...fact, state: "answered" }, revision: 9 } });
    h.connection.handleEvent({ event: { type: "issue_decision", decision: fact, revision: 8 } });
    expect(h.view().issueAttention).toEqual([]);
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
  });

  it("accepts a derived policy display value emitted by the server", async () => {
    const h = harness({ autoPolicy: false });
    await h.openIssues();
    const pending = h.connection.refreshIssueAutomationPolicy();
    const answer = policyAnswer();
    answer.settings[0]!.resolution = "derived";
    h.reply(h.query("issue_automation_policy"), answer);
    await pending;
    expect(h.view().issues.policy).toEqual(answer);
    expect(h.view().issues.policyError).toBeNull();
  });
});

function executionTrackingDetail(issue = issueReference(), revision = 7, label = "서버 실행 중") {
  return {
    ...issueDetail(issue, revision),
    execution: {
      status: "ready", work_order_id: "work-order-fixture", revision: 41,
      state: "running", state_label: label, execution_attempts: 2, execution_attempt_limit: 3,
      verification_attempts: 4, review_pull_request_url: null, terminal_class: null, recovery: null,
    },
    feedback: { revision: 2, publication_attempts: 5, external_state: "open", reason: "feedback_unconfirmed" },
    timeline: { status: "ready", entries: [{ sequence: 101, recorded_at: "2026-11-22T09:00:00Z", kind: "issue_execution_progressed", summary: "저장된 서버 기록" }], has_older: false },
  };
}

function executionTrackingEvent(kind: string, revision = 2, issue = issueReference()) {
  const common = { kind, revision, issue, event_id: "fixture-" + kind };
  const integration = { record_id: "integration-1", source_sha: "a".repeat(40), integration_sha: "b".repeat(40), pull_request_url: "https://github.com/sample/widgets/pull/9" };
  const event = kind === "issue_execution_progressed"
    ? { ...common, work_order_id: "work-order-fixture", progress: { revision, state: "running", attempts: { execution: 2, verification: 4, publication: 0 }, attempts_exhausted: false } }
    : kind === "issue_integrated" ? { ...common, work_order_id: "work-order-fixture", integration }
      : kind === "issue_feedback_posted" ? { ...common, intent_id: "intent-1", actor_id: "actor-1", action: "comment_created", comment_id: "comment-1" }
        : { ...common, origin: "confirmed_feedback", intent_id: "close-1", actor_id: "actor-1", closure_event_id: "closure-1" };
  return { event: { type: "recorded", sequence: 101, recorded_at: "2026-11-22T09:00:00Z", event } };
}

describe("issue execution recorded-event refresh", () => {
  it.each(["issue_execution_progressed", "issue_integrated", "issue_closed", "issue_feedback_posted"])(
    "reads a fresh detail after %s even when the journal revision is below the catalog",
    async (kind) => {
      const h = harness();
      await h.openIssues();
      const selected = h.connection.selectIssue(issueReference());
      h.reply(h.query("issue"), executionTrackingDetail());
      await selected;
      const before = h.queries("issue").length;
      h.connection.handleEvent(executionTrackingEvent(kind, 2));
      expect(h.view().issues).toMatchObject({ detail: null, detailStale: true });
      await h.when(() => h.queries("issue").length === before + 1);
      h.answerIssueReads();
      const fresh = executionTrackingDetail(issueReference(), 7, "서버가 재조회한 상태");
      h.reply(h.query("issue"), fresh);
      await h.when(() => !h.view().issues.detailLoading);
      expect(h.view().issues.detail).toEqual(fresh);
      expect(h.requests("akra/kernel/command")).toHaveLength(0);
      expect(h.requests("turn/start")).toHaveLength(0);
      expect(h.requests("turn/steer")).toHaveLength(0);
    },
  );

  it("coalesces recorded progress and never treats a large journal revision as a catalog watermark", async () => {
    const h = harness();
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    h.reply(h.query("issue"), executionTrackingDetail());
    await selected;
    const before = h.queries("issue").length;
    h.connection.handleEvent(executionTrackingEvent("issue_execution_progressed", 900));
    h.connection.handleEvent(executionTrackingEvent("issue_feedback_posted", 901));
    h.connection.handleEvent(executionTrackingEvent("issue_closed", 902));
    await h.when(() => h.queries("issue").length === before + 1);
    h.answerIssueReads();
    const fresh = executionTrackingDetail(issueReference(), 7, "별도 revision 공간");
    h.reply(h.query("issue"), fresh);
    await h.when(() => !h.view().issues.detailLoading);
    expect(h.queries("issue")).toHaveLength(before + 1);
    expect(h.view().issues).toMatchObject({ detail: fresh, detailStale: false, detailError: null });
  });

  it("discards an in-flight pre-event execution snapshot with the same catalog revision", async () => {
    const h = harness();
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    h.reply(h.query("issue"), executionTrackingDetail());
    await selected;
    const oldRefresh = h.connection.refreshIssueDetail();
    const oldRequest = h.query("issue");
    const before = h.queries("issue").length;
    h.connection.handleEvent(executionTrackingEvent("issue_integrated", 4));
    await h.when(() => h.queries("issue").length === before + 1);
    const freshRequest = h.query("issue");
    h.reply(oldRequest, executionTrackingDetail(issueReference(), 7, "이벤트 전의 오래된 실행"));
    await oldRefresh;
    expect(h.view().issues.detail).toBeNull();
    h.answerIssueReads();
    const fresh = executionTrackingDetail(issueReference(), 7, "통합 후 서버 상태");
    h.reply(freshRequest, fresh);
    await h.when(() => !h.view().issues.detailLoading);
    expect(h.view().issues.detail).toEqual(fresh);
    expect(JSON.stringify(h.view().issues)).not.toContain("이벤트 전의 오래된 실행");
  });

  it("ignores foreign workspaces and repositories without invalidating the current execution", async () => {
    const h = harness();
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    const shown = executionTrackingDetail();
    h.reply(h.query("issue"), shown);
    await selected;
    const count = h.requests("akra/kernel/query").length;
    for (const issue of [
      issueReference("I_first", "/foreign"),
      { ...issueReference(), repository_id: "R_foreign" },
      { ...issueReference(), server_id: "github.example" },
      { ...issueReference(), provider: "jira" as const },
    ]) h.connection.handleEvent(executionTrackingEvent("issue_feedback_posted", 999, issue));
    await Promise.resolve();
    expect(h.requests("akra/kernel/query")).toHaveLength(count);
    expect(h.view().issues.detail).toEqual(shown);
    expect(h.view().issues.detailStale).toBe(false);
  });

  it("rechecks an unconfirmed result using only the selected issue query", async () => {
    const h = harness();
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    h.reply(h.query("issue"), executionTrackingDetail());
    await selected;
    const start = h.sent.length;
    const refresh = h.connection.refreshIssueDetail();
    h.reply(h.query("issue"), { ...executionTrackingDetail(), feedback: { revision: 3, publication_attempts: 5, external_state: "open" } });
    await refresh;
    expect(h.sent.slice(start)).toHaveLength(1);
    expect(h.sent[start]).toMatchObject({ method: "akra/kernel/query", params: { query: { type: "issue", issue: issueReference() } } });
    expect(h.requests("akra/kernel/command")).toHaveLength(0);
    expect(h.requests("turn/start")).toHaveLength(0);
    expect(h.requests("turn/steer")).toHaveLength(0);
  });

  it("rejects a malformed tracking response and clears previously visible execution data", async () => {
    const h = harness();
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    const fresh = executionTrackingDetail();
    h.reply(h.query("issue"), fresh);
    await selected;
    const refresh = h.connection.refreshIssueDetail();
    h.reply(h.query("issue"), { ...fresh, execution: { ...fresh.execution, execution_attempts: -1 } });
    await refresh;
    expect(h.view().issues).toMatchObject({ detail: null, detailStale: true, detailLoading: false });
    expect(h.view().issues.detailError).toContain("표시 계약");
  });
});

describe("issue execution snapshot fencing", () => {
  it("retries an execution response older than a freshly queried catalog without using event journal revisions", async () => {
    const h = harness();
    await h.openIssues();
    const selected = h.connection.selectIssue(issueReference());
    h.reply(h.query("issue"), executionTrackingDetail());
    await selected;
    const before = h.queries("issue").length;
    h.connection.handleEvent(executionTrackingEvent("issue_feedback_posted", 1));
    await h.when(() => h.queries("issue").length === before + 1);
    h.answerIssueReads(issuePage(["I_first"], 8, null), automation(8));
    h.reply(h.query("issue"), executionTrackingDetail(issueReference(), 7, "이전 catalog의 실행"));
    await h.when(() => h.queries("issue").length === before + 2);
    expect(h.view().issues.detail).toBeNull();
    const fresh = executionTrackingDetail(issueReference(), 8, "현재 catalog의 실행");
    h.reply(h.query("issue"), fresh);
    await h.when(() => !h.view().issues.detailLoading);
    expect(h.view().issues.detail).toEqual(fresh);
    expect(JSON.stringify(h.view().issues)).not.toContain("이전 catalog의 실행");
  });

  it("never applies late execution data from the previous workspace", async () => {
    const h = harness({ autoPolicy: false });
    await h.openIssues();
    const oldDetail = h.connection.selectIssue(issueReference());
    const oldRequest = h.query("issue");
    const shown = h.connection.show("/second/worktree", ["issues"]);
    await h.answerAttach("second-kernel");
    h.reply(oldRequest, executionTrackingDetail(issueReference(), 999, "이전 저장소 작업"));
    await oldDetail;
    expect(h.view().issues).toMatchObject({ selectedIssue: null, detail: null, detailError: null });
    const page = issuePage(["I_second"], 1, null);
    page.items[0]!.issue = issueReference("I_second", "/second");
    h.answerIssueReads(page, automation(1));
    await h.answerPolicyBootstrap(policyAnswer("second-policy", "/second"));
    await shown;
    expect(JSON.stringify(h.view().issues)).not.toContain("이전 저장소 작업");
    const count = h.requests("akra/kernel/query").length;
    h.connection.handleEvent(executionTrackingEvent("issue_closed", 1000, issueReference()));
    await Promise.resolve();
    expect(h.requests("akra/kernel/query")).toHaveLength(count);
    expect(h.view().issues.page).toMatchObject({ revision: 1 });
  });
});
