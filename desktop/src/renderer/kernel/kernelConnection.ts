// 같은 Gateway 접속에서 저장소 커널에 한 번 붙고, 열린 패널의 관심 영역만 읽는다.
// 개요와 이슈 목록은 서로 다른 조회 계약을 쓰며, 이슈를 읽는 동안 모델 턴을 만들지 않는다.
import { JsonRpcError, type JsonRpcClient } from "../rpc/jsonRpcClient";
import {
  type AttachResponse,
  type AutoFollowSettings,
  type IssueAutomationAnswer,
  type IssueAutomationModeRecord,
  type IssueAutomationPolicyAnswer,
  type IssueDetailAnswer,
  type IssueEventReference,
  type IssuesPageAnswer,
  KERNEL_METHOD,
  KERNEL_UNAVAILABLE,
  KERNEL_VIEW_QUERIES,
  type KernelCommand,
  type KernelEvent,
  type KernelViewAnswer,
  type ParallelState,
  type PlanningState,
  type ThreadActivity,
} from "./protocol";
import { sendKernelCommand } from "./sendKernelCommand";

export type KernelPhase = "idle" | "attaching" | "attached" | "failed";
export type KernelInterest = "overview" | "issues";

export interface KernelIssuesView {
  automation: IssueAutomationAnswer | null;
  /** 서버가 정한 순서와 묶음을 가진 현재 한 페이지다. 이전 페이지를 합치지 않는다. */
  page: IssuesPageAnswer | null;
  selectedIssue: IssueEventReference | null;
  detail: IssueDetailAnswer | null;
  detailLoading: boolean;
  detailError: string | null;
  /** 현재 선택 또는 catalog 판에 맞는 상세를 다시 읽고 있다. 저장된 triage.stale과는 별개다. */
  detailStale: boolean;
  policy: IssueAutomationPolicyAnswer | null;
  policyLoading: boolean;
  policyError: string | null;
  loading: boolean;
  pageNumber: number;
  canGoBack: boolean;
  commandPending: "mode" | "sync" | null;
  error: string | null;
  /** 보이는 페이지가 새 revision보다 오래되었거나 다시 읽기에 실패했다. */
  stale: boolean;
}

export const EMPTY_KERNEL_ISSUES: KernelIssuesView = {
  automation: null,
  page: null,
  selectedIssue: null,
  detail: null,
  detailLoading: false,
  detailError: null,
  detailStale: false,
  policy: null,
  policyLoading: false,
  policyError: null,
  loading: false,
  pageNumber: 1,
  canGoBack: false,
  commandPending: null,
  error: null,
  stale: false,
};

export interface KernelView {
  phase: KernelPhase;
  workspace: string | null;
  /** 붙은 커널의 저장소 신원이다. */
  kernel: string | null;
  message: string | null;
  threads: ThreadActivity[];
  autoFollow: AutoFollowSettings | null;
  planning: PlanningState | null;
  parallel: ParallelState | null;
  issues: KernelIssuesView;
}

export const EMPTY_KERNEL_VIEW: KernelView = {
  phase: "idle",
  workspace: null,
  kernel: null,
  message: null,
  threads: [],
  autoFollow: null,
  planning: null,
  parallel: null,
  issues: EMPTY_KERNEL_ISSUES,
};

export interface KernelStore {
  getKernel(): KernelView;
  setKernel(view: KernelView): void;
}

interface IssueRequestToken {
  connection: number;
  interest: number;
  request: number;
}

interface IssuePageRequest {
  token: IssueRequestToken;
  after: string | null;
}

interface IssueDetailRequest {
  token: IssueRequestToken;
  issue: IssueEventReference;
}

const CLIENT_NAME = "akra-desktop";
const ISSUE_PAGE_SIZE = 50;

function failureMessage(error: unknown): string {
  if (error instanceof JsonRpcError && error.code === KERNEL_UNAVAILABLE) {
    return `커널에 붙을 수 없습니다. 같은 저장소에서 TUI 가 실행 중이면 TUI 를 닫으세요. (${error.message})`;
  }
  return error instanceof Error ? error.message : String(error);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 표시 계약의 모양만 검사한다. 상태·이유·묶음의 의미는 서버가 정한다. */
function isIssuesPageAnswer(value: unknown): value is IssuesPageAnswer {
  if (!isRecord(value) || value.type !== "issues") return false;
  if (value.status === "unavailable") return true;
  if (value.status !== "ready" || !Number.isSafeInteger(value.revision) || Number(value.revision) < 0) return false;
  if (!Array.isArray(value.items) || !Array.isArray(value.groups)) return false;
  if (value.next_after !== undefined && value.next_after !== null && typeof value.next_after !== "string") return false;
  if (!value.items.every((item: unknown) => {
    if (!isRecord(item) || !isIssueReference(item.issue) || !isRecord(item.presentation)) return false;
    const shown = item.presentation;
    if (shown.number !== null && (!Number.isSafeInteger(shown.number) || Number(shown.number) < 1)) return false;
    if (shown.title !== null && typeof shown.title !== "string") return false;
    if (shown.url !== null && typeof shown.url !== "string") return false;
    return isRecord(shown.status) && typeof shown.status.code === "string" && typeof shown.status.label === "string"
      && isRecord(shown.reason) && typeof shown.reason.code === "string" && typeof shown.reason.summary === "string";
  })) return false;
  const indices = new Set<number>();
  const itemCount = value.items.length;
  for (const group of value.groups as unknown[]) {
    if (!isRecord(group) || typeof group.id !== "string" || typeof group.label !== "string" || !Array.isArray(group.item_indices)) return false;
    for (const index of group.item_indices as unknown[]) {
      if (typeof index !== "number" || !Number.isSafeInteger(index) || index < 0 || index >= itemCount || indices.has(index)) return false;
      indices.add(index);
    }
  }
  return indices.size === itemCount;
}

function isNamedValue(value: unknown, names: readonly string[]): value is string {
  return typeof value === "string" && names.includes(value);
}

function isIssueReference(value: unknown): value is IssueEventReference {
  if (!isRecord(value) || !isNamedValue(value.provider, ["github", "jira"]) || !isRecord(value.project_scope)) return false;
  return [value.server_id, value.repository_id, value.issue_id, value.project_scope.workspace_dir]
    .every((part) => typeof part === "string" && part.length > 0 && part.trim() === part && !/[\u0000-\u001f\u007f]/.test(part));
}

/** 표시 번호나 URL 대신 서버가 반환한 안정 좌표 전체를 비교한다. */
function sameIssue(left: IssueEventReference | null, right: IssueEventReference): boolean {
  return left !== null && left.provider === right.provider && left.server_id === right.server_id
    && left.repository_id === right.repository_id && left.issue_id === right.issue_id
    && left.project_scope.workspace_dir === right.project_scope.workspace_dir;
}

function isRevision(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function hasCoherentReport(value: Record<string, unknown>): boolean {
  if (value.report === undefined || value.report === null) return true;
  const report = value.report;
  return isRecord(report) && report.revision === value.revision
    && ["total", "automatic_start", "human_approval", "deferred", "excluded", "needs_info", "unknown", "observation_only", "stale", "untriaged", "blocked"]
      .every((key) => isRevision(report[key]));
}

function isIssueDetailAnswer(value: unknown): value is IssueDetailAnswer {
  if (!isRecord(value) || value.type !== "issue") return false;
  if (value.status === "unavailable") return true;
  if (!isRevision(value.revision) || !hasCoherentReport(value)) return false;
  if (value.status === "not_found") return true;
  if (value.status !== "ready" || !isRecord(value.item)) return false;
  const item = value.item;
  // 목록과 같은 표시 계약을 재사용하고, 상세에서 실제로 그릴 공유본의 모양도 확인한다.
  if (!isIssuesPageAnswer({ type: "issues", status: "ready", revision: value.revision, items: [item], groups: [{ id: "detail", label: "", item_indices: [0] }] })) return false;
  if (!isRecord(item.current)) return false;
  if (item.current.status === "observed") {
    if (!isRecord(item.current.content)) return false;
    const content = item.current.content;
    if (content.kind === "shared") {
      if (!isRecord(content.shared_copy) || typeof content.shared_copy.title !== "string" || typeof content.shared_copy.body !== "string") return false;
    } else if (content.kind !== "withheld") return false;
  } else if (!isNamedValue(item.current.status, ["unknown", "not_yet_observed"])) return false;
  if (item.triage === undefined || item.triage === null) return true;
  const triage = item.triage;
  return isRecord(triage) && typeof triage.stale === "boolean"
    && Array.isArray(triage.gates) && triage.gates.every((gate: unknown) => isRecord(gate)
      && isNamedValue(gate.gate, ["start", "spec", "plan", "delivery", "merge", "close"])
      && isNamedValue(gate.action, ["human", "timeout_default", "timeout_arbiter", "auto"])
      && typeof gate.enabled === "boolean" && isRevision(gate.gate_revision) && gate.gate_revision > 0)
    && Array.isArray(triage.reasons) && triage.reasons.every((reason: unknown) => isRecord(reason)
      && typeof reason.code === "string" && typeof reason.summary === "string" && typeof reason.next_summary === "string" && Array.isArray(reason.next));
}

function isIssueAutomationPolicyAnswer(value: unknown): value is IssueAutomationPolicyAnswer {
  if (!isRecord(value) || value.type !== "issue_automation_policy") return false;
  if (value.status === "unavailable") return true;
  // 서버의 대표 checkout은 attach 요청의 worktree/하위 폴더와 다를 수 있다.
  if (typeof value.workspace !== "string" || value.workspace.trim().length === 0) return false;
  if (!Array.isArray(value.files) || !value.files.every((file: unknown) => isRecord(file)
    && isNamedValue(file.scope, ["user", "project"]) && typeof file.label === "string"
    && (file.path === null || typeof file.path === "string")
    && isNamedValue(file.status, ["missing", "readable", "unavailable"])
    && (file.open_target === null || file.open_target === file.scope))) return false;
  if (value.status === "policy_blocked") {
    const blocked = value.blocked;
    return isRecord(blocked) && isNamedValue(blocked.scope, ["user", "project", "effective"])
      && typeof blocked.field === "string" && typeof blocked.reason === "string" && typeof blocked.summary === "string"
      && typeof blocked.next_summary === "string" && Array.isArray(blocked.file_scopes)
      && blocked.file_scopes.every((scope: unknown) => scope === "user" || scope === "project");
  }
  return value.status === "ready" && typeof value.version === "string" && Array.isArray(value.settings)
    && value.settings.every((setting: unknown) => isRecord(setting) && typeof setting.key === "string"
      && typeof setting.label === "string" && typeof setting.value === "string"
      && isNamedValue(setting.resolution, ["default", "override", "minimum", "and", "intersection", "union", "cap"])
      && typeof setting.resolution_summary === "string" && Array.isArray(setting.origins)
      && setting.origins.every((origin: unknown) => isRecord(origin)
        && isNamedValue(origin.scope, ["builtin", "user", "project"]) && typeof origin.label === "string"));
}

export class KernelConnection {
  private wanted: string | null = null;
  private interests = new Set<KernelInterest>();
  private connected = false;
  private generation = 0;
  private overviewGeneration = 0;
  private overviewRequest = 0;
  /** resync는 조회 응답을 대체하지만, 명시적인 planning 새로 고침 요청을 취소하지 않는다. */
  private overviewRefreshRequest = 0;
  private readonly eventCounts = new Map<string, number>();

  private issuesGeneration = 0;
  private issueStateRequest = 0;
  private issuePageRequest = 0;
  private issueDetailRequest = 0;
  private issuePolicyRequest = 0;
  /** 마지막 정책 요청이 시작될 때 알고 있던 catalog 판이다. 목록/상세의 최고 판과 별도로 유지한다. */
  private policyKnownRevision = -1;
  private scheduledPolicyRefresh: IssueRequestToken | null = null;
  private issueCommandRequest = 0;
  private activeIssuePage: IssuePageRequest | null = null;
  private activeIssueDetail: IssueDetailRequest | null = null;
  private detailKnownRevision = 0;
  private latestIssueRevision = 0;
  private currentIssueCursor: string | null = null;
  private previousIssueCursors: Array<string | null> = [];
  private issueStateError: string | null = null;
  private issuePageError: string | null = null;
  private issueCommandError: string | null = null;

  constructor(
    private readonly client: JsonRpcClient,
    private readonly store: KernelStore,
  ) {}

  /** 같은 폴더에서는 관심 영역만 바꾸고, 관심 영역이 없으면 커널에서 뗀다. */
  async show(workspace: string | null, interests: readonly KernelInterest[] = ["overview"]): Promise<void> {
    const next = new Set(interests);
    const wanted = next.size === 0 ? null : workspace;
    const sameWorkspace = wanted === this.wanted;
    const overviewChanged = next.has("overview") !== this.interests.has("overview");
    const issuesChanged = next.has("issues") !== this.interests.has("issues");
    if (sameWorkspace && !overviewChanged && !issuesChanged) return;

    const previous = this.wanted;
    this.wanted = wanted;
    this.interests = next;
    if (overviewChanged || !sameWorkspace) {
      this.overviewGeneration += 1;
      this.overviewRequest += 1;
      this.overviewRefreshRequest += 1;
    }
    if (issuesChanged || !sameWorkspace) this.invalidateIssues();
    if (!sameWorkspace) this.generation += 1;
    this.store.setKernel({ ...this.store.getKernel(), message: null });

    if (wanted === null) {
      this.store.setKernel(EMPTY_KERNEL_VIEW);
      if (previous !== null && this.connected) {
        await this.client.request(KERNEL_METHOD.detach, {}).catch(() => undefined);
      }
      return;
    }
    if (sameWorkspace && this.store.getKernel().phase === "attached") {
      await Promise.all([
        overviewChanged && next.has("overview") ? this.readOverview(true) : Promise.resolve(),
        issuesChanged && next.has("issues") ? this.refreshIssues() : Promise.resolve(),
      ]);
      return;
    }
    if (!sameWorkspace || this.store.getKernel().phase !== "attaching") await this.attach();
  }

  connectionChanged(connected: boolean): void {
    this.connected = connected;
    this.generation += 1;
    this.overviewGeneration += 1;
    this.overviewRequest += 1;
    this.overviewRefreshRequest += 1;
    this.eventCounts.clear();
    this.invalidateIssues();
    void this.attach();
  }

  /** 개요를 요청한 패널에서만 planning 큐를 다시 읽게 한다. */
  async refresh(): Promise<void> {
    if (!this.interests.has("overview")) return;
    if (this.store.getKernel().phase !== "attached") {
      if (this.connected && this.store.getKernel().phase !== "attaching") await this.attach();
      return;
    }
    await this.readOverview(true);
  }

  /** 상태와 첫 페이지를 다시 읽는다. 새로고침은 이전 페이지 이동 요청을 대체한다. */
  async refreshIssues(): Promise<void> {
    if (!this.canRead("issues")) {
      if (this.interests.has("issues") && this.connected && this.store.getKernel().phase !== "attaching") await this.attach();
      return;
    }
    this.issueCommandError = null;
    await Promise.all([
      this.readIssueAutomation(),
      this.readIssuesPage(null, [], null),
      this.refreshIssueAutomationPolicy(),
      this.refreshIssueDetail(),
    ]);
  }

  async nextIssuesPage(): Promise<void> {
    const view = this.store.getKernel().issues;
    if (!this.canNavigateIssues() || view.page?.status !== "ready" || !view.page.next_after) return;
    this.clearIssueSelection();
    await this.readIssuesPage(view.page.next_after, [...this.previousIssueCursors, this.currentIssueCursor], view.page.revision);
  }

  async previousIssuesPage(): Promise<void> {
    if (!this.canNavigateIssues() || this.previousIssueCursors.length === 0) return;
    const page = this.store.getKernel().issues.page;
    if (page?.status !== "ready") return;
    const after = this.previousIssueCursors.at(-1) ?? null;
    this.clearIssueSelection();
    await this.readIssuesPage(after, this.previousIssueCursors.slice(0, -1), page.revision);
  }

  /** 현재 페이지가 실제로 제공한 전체 안정 좌표만 선택한다. worktree 경로를 대표 workspace로 추론하지 않는다. */
  async selectIssue(issue: IssueEventReference): Promise<void> {
    const issues = this.store.getKernel().issues;
    if (!this.canRead("issues") || issues.stale || issues.page?.status !== "ready" || !isIssueReference(issue)) return;
    const selected = issues.page.items.find((item) => sameIssue(item.issue, issue))?.issue;
    if (!selected) return;
    if (sameIssue(issues.selectedIssue, selected) && (issues.detailLoading || (issues.detail !== null && !issues.detailStale))) return;
    this.clearIssueSelection();
    const reference = { ...selected, project_scope: { ...selected.project_scope } };
    this.updateIssues({ selectedIssue: reference });
    await this.readIssueDetail(reference);
  }

  clearIssueSelection(): void {
    this.issueDetailRequest += 1;
    this.activeIssueDetail = null;
    this.detailKnownRevision = 0;
    this.updateIssues({ selectedIssue: null, detail: null, detailLoading: false, detailError: null, detailStale: false });
  }

  async refreshIssueDetail(): Promise<void> {
    const issue = this.store.getKernel().issues.selectedIssue;
    if (issue !== null) await this.readIssueDetail(issue);
  }

  async refreshIssueAutomationPolicy(): Promise<void> {
    if (!this.canRead("issues")) return;
    const token = this.issueToken(++this.issuePolicyRequest);
    const revision = this.latestIssueRevision;
    this.policyKnownRevision = revision;
    this.scheduledPolicyRefresh = null;
    this.updateIssues({ policy: null, policyLoading: true, policyError: null });
    try {
      const policy = await this.client.request<unknown>(KERNEL_METHOD.query, { query: { type: "issue_automation_policy" } });
      if (!this.policyCurrent(token, revision)) return;
      if (!isIssueAutomationPolicyAnswer(policy)) throw new Error("정책 표시 계약을 읽을 수 없습니다. Gateway를 업데이트하거나 다시 시작해 주세요.");
      this.updateIssues({ policy });
    } catch (error) {
      if (this.policyCurrent(token, revision)) {
        this.updateIssues({ policy: null, policyError: "이슈 정책을 읽지 못했습니다: " + failureMessage(error) });
      }
    } finally {
      if (this.policyCurrent(token, revision)) this.updateIssues({ policyLoading: false });
    }
  }

  setIssueAutomationMode(mode: IssueAutomationModeRecord): Promise<void> {
    return this.runIssueCommand({ type: "set_issue_automation_mode", mode }, "mode");
  }

  syncIssues(): Promise<void> {
    return this.runIssueCommand({ type: "sync_issues" }, "sync");
  }

  handleEvent(params: unknown): void {
    const event = (params as { event?: KernelEvent } | undefined)?.event;
    if (!event || this.store.getKernel().phase !== "attached") return;
    this.eventCounts.set(event.type, (this.eventCounts.get(event.type) ?? 0) + 1);
    if (event.type === "resync") {
      if (this.interests.has("overview")) void this.readOverview(false);
      if (this.interests.has("issues")) void this.refreshIssues();
      return;
    }
    if (event.type === "issues") {
      if (this.canRead("issues")) {
        const { type: _type, ...state } = event;
        this.applyIssueAutomation({ ...state, type: "issue_automation" });
      }
      return;
    }
    if (this.interests.has("overview")) this.applyOverview(event);
  }

  private async attach(): Promise<void> {
    const workspace = this.wanted;
    if (workspace === null) return;
    if (!this.connected) {
      this.store.setKernel({ ...EMPTY_KERNEL_VIEW, workspace, message: "Gateway 연결을 기다립니다." });
      return;
    }
    const generation = ++this.generation;
    const previous = this.store.getKernel();
    this.store.setKernel({ ...previous, phase: "attaching", workspace, message: null });
    let attached: AttachResponse;
    try {
      attached = await this.client.request<AttachResponse>(KERNEL_METHOD.attach, { workspace, client_name: CLIENT_NAME });
    } catch (error) {
      if (generation === this.generation) {
        this.store.setKernel({ ...EMPTY_KERNEL_VIEW, phase: "failed", workspace, message: failureMessage(error) });
      }
      return;
    }
    if (generation !== this.generation) return;
    const kept = previous.kernel === attached.kernel ? this.store.getKernel() : EMPTY_KERNEL_VIEW;
    this.store.setKernel({ ...kept, phase: "attached", workspace, kernel: attached.kernel, message: null });
    await Promise.all([
      this.interests.has("overview") ? this.readOverview(true) : Promise.resolve(),
      this.interests.has("issues") ? this.refreshIssues() : Promise.resolve(),
    ]);
  }

  private canRead(interest: KernelInterest): boolean {
    return this.connected && this.wanted !== null && this.interests.has(interest) && this.store.getKernel().phase === "attached";
  }

  private overviewScopeCurrent(generation: number, interest: number): boolean {
    return this.canRead("overview") && generation === this.generation && interest === this.overviewGeneration;
  }

  private overviewCurrent(generation: number, interest: number, request: number): boolean {
    return this.overviewScopeCurrent(generation, interest) && request === this.overviewRequest;
  }

  private overviewRefreshCurrent(generation: number, interest: number, refresh: number): boolean {
    return this.overviewScopeCurrent(generation, interest) && refresh === this.overviewRefreshRequest;
  }

  private async readOverview(refreshPlanning: boolean): Promise<void> {
    if (!this.canRead("overview")) return;
    const generation = this.generation;
    const interest = this.overviewGeneration;
    const request = ++this.overviewRequest;
    const refresh = refreshPlanning ? ++this.overviewRefreshRequest : null;
    this.store.setKernel({ ...this.store.getKernel(), message: null });
    try {
      await Promise.all(KERNEL_VIEW_QUERIES.map(async (type) => {
        const seen = this.eventCounts.get(type) ?? 0;
        const event = await this.client.request<KernelViewAnswer>(KERNEL_METHOD.query, { query: { type } });
        if (this.overviewCurrent(generation, interest, request) && (this.eventCounts.get(type) ?? 0) === seen) {
          this.applyOverview(event);
        }
      }));
      if (refresh !== null) await this.refreshPlanningForOverview(generation, interest, refresh);
    } catch (error) {
      if (this.overviewCurrent(generation, interest, request)) {
        this.store.setKernel({ ...this.store.getKernel(), message: `커널 상태를 읽지 못했습니다: ${failureMessage(error)}` });
      }
    }
  }

  private async refreshPlanningForOverview(generation: number, interest: number, refresh: number): Promise<void> {
    if (!this.overviewRefreshCurrent(generation, interest, refresh)) return;
    try {
      await sendKernelCommand(this.client, { type: "refresh_planning" });
    } catch (error) {
      // 명령 실패는 같은 관심 영역의 resync 조회가 끝났더라도 해당 새로 고침에 남긴다.
      if (this.overviewRefreshCurrent(generation, interest, refresh)) {
        this.store.setKernel({ ...this.store.getKernel(), message: `커널 상태를 읽지 못했습니다: ${failureMessage(error)}` });
      }
    }
  }

  private invalidateIssues(): void {
    this.issuesGeneration += 1;
    this.issueStateRequest += 1;
    this.issuePageRequest += 1;
    this.issueDetailRequest += 1;
    this.issuePolicyRequest += 1;
    this.policyKnownRevision = -1;
    this.scheduledPolicyRefresh = null;
    this.issueCommandRequest += 1;
    this.activeIssuePage = null;
    this.activeIssueDetail = null;
    this.detailKnownRevision = 0;
    this.latestIssueRevision = 0;
    this.currentIssueCursor = null;
    this.previousIssueCursors = [];
    this.issueStateError = null;
    this.issuePageError = null;
    this.issueCommandError = null;
    this.updateIssues(EMPTY_KERNEL_ISSUES);
  }

  private issueToken(request: number): IssueRequestToken {
    return { connection: this.generation, interest: this.issuesGeneration, request };
  }

  private issueCurrent(token: IssueRequestToken, request: number): boolean {
    return this.canRead("issues") && token.connection === this.generation && token.interest === this.issuesGeneration && token.request === request;
  }

  private updateIssues(update: Partial<KernelIssuesView>): void {
    const view = this.store.getKernel();
    this.store.setKernel({
      ...view,
      issues: { ...view.issues, ...update, error: this.issueCommandError ?? this.issuePageError ?? this.issueStateError },
    });
  }

  private async readIssueAutomation(): Promise<void> {
    const token = this.issueToken(++this.issueStateRequest);
    const seen = this.eventCounts.get("issues") ?? 0;
    this.issueStateError = null;
    this.updateIssues({});
    try {
      const state = await this.client.request<IssueAutomationAnswer>(KERNEL_METHOD.query, { query: { type: "issue_automation" } });
      if (this.issueCurrent(token, this.issueStateRequest) && (this.eventCounts.get("issues") ?? 0) === seen) {
        this.applyIssueAutomation(state);
      }
    } catch (error) {
      if (this.issueCurrent(token, this.issueStateRequest) && (this.eventCounts.get("issues") ?? 0) === seen) {
        this.issueStateError = `이슈 수집 상태를 읽지 못했습니다: ${failureMessage(error)}`;
        this.updateIssues({});
      }
    }
  }

  private applyIssueAutomation(automation: IssueAutomationAnswer): void {
    const issues = this.store.getKernel().issues;
    if (automation.status === "ready" && issues.automation?.status === "ready" && automation.revision < issues.automation.revision) return;
    this.issueStateError = null;
    if (automation.status === "ready") this.noteIssueRevision(automation.revision);
    this.updateIssues({ automation });
    if (automation.status !== "ready") return;
    this.invalidateDetailAtRevision(automation.revision);
    const pageRevision = issues.page?.status === "ready" ? issues.page.revision : -1;
    if (automation.revision <= pageRevision) return;
    this.updateIssues({ stale: issues.page !== null, canGoBack: false });
    // 첫 페이지 요청은 최신 revision을 확인해 답을 고른다. 이전 커서의 요청은 즉시 대체한다.
    if (this.activeIssuePage?.after === null) return;
    void this.readIssuesPage(null, [], null);
  }

  private canNavigateIssues(): boolean {
    const issues = this.store.getKernel().issues;
    return this.canRead("issues") && !issues.loading && !issues.stale && issues.commandPending === null;
  }

  private async readIssuesPage(
    after: string | null,
    history: Array<string | null>,
    expectedRevision: number | null,
    retried = false,
  ): Promise<void> {
    if (!this.canRead("issues")) return;
    const token = this.issueToken(++this.issuePageRequest);
    const request = { token, after };
    this.activeIssuePage = request;
    this.issuePageError = null;
    this.updateIssues({ loading: true, ...(after === null && history.length === 0 ? { canGoBack: false } : {}) });
    let restart = false;
    try {
      const page = await this.client.request<unknown>(KERNEL_METHOD.query, { query: { type: "issues", after, limit: ISSUE_PAGE_SIZE } });
      if (!this.issueCurrent(token, this.issuePageRequest)) return;
      if (!isIssuesPageAnswer(page) || !hasCoherentReport(page)) throw new Error("이슈 표시 계약을 읽을 수 없습니다. Gateway를 업데이트하거나 다시 시작해 주세요.");
      const outdated = page.status === "ready" && (page.revision < this.latestIssueRevision || (expectedRevision !== null && page.revision !== expectedRevision));
      if (page.status === "ready") {
        this.noteIssueRevision(page.revision);
        this.invalidateDetailAtRevision(page.revision);
      }
      if (outdated) {
        restart = true;
      } else {
        this.currentIssueCursor = page.status === "ready" ? after : null;
        this.previousIssueCursors = page.status === "ready" ? history : [];
        this.updateIssues({ page, pageNumber: this.previousIssueCursors.length + 1, canGoBack: this.previousIssueCursors.length > 0, stale: false });
      }
    } catch (error) {
      if (this.issueCurrent(token, this.issuePageRequest)) {
        this.issuePageError = `이슈 목록을 읽지 못했습니다: ${failureMessage(error)}`;
        this.updateIssues({ stale: this.store.getKernel().issues.page !== null, canGoBack: false });
      }
    } finally {
      if (this.issueCurrent(token, this.issuePageRequest)) {
        this.activeIssuePage = null;
        this.updateIssues({ loading: false });
      }
    }
    if (restart && this.issueCurrent(token, this.issuePageRequest)) {
      this.updateIssues({ stale: this.store.getKernel().issues.page !== null, canGoBack: false });
      if (retried) {
        this.issuePageError = "이슈 목록이 조회 중 바뀌었습니다. 첫 페이지를 다시 읽어 주세요.";
        this.updateIssues({});
      } else {
        await this.readIssuesPage(null, [], null, true);
      }
    }
  }

  private policyCurrent(token: IssueRequestToken, revision: number): boolean {
    return this.issueCurrent(token, this.issuePolicyRequest) && revision >= this.latestIssueRevision;
  }

  /** state/page/detail 중 먼저 도착한 새 판이 정책도 무효화한다. 이벤트가 나중에 와도 정책 판을 놓치지 않는다. */
  private noteIssueRevision(revision: number): void {
    this.latestIssueRevision = Math.max(this.latestIssueRevision, revision);
    if (this.latestIssueRevision <= this.policyKnownRevision) return;
    this.updateIssues({ policy: null, policyLoading: true, policyError: null });
    if (this.scheduledPolicyRefresh !== null) return;
    const scheduled = this.issueToken(this.issuePolicyRequest);
    this.scheduledPolicyRefresh = scheduled;
    // 같은 응답 묶음에서 관찰한 여러 판은 최신 판 하나의 조회로 합친다. 다른 I/O 완료를 기다리지 않는다.
    queueMicrotask(() => {
      if (this.scheduledPolicyRefresh !== scheduled) return;
      this.scheduledPolicyRefresh = null;
      if (!this.issueCurrent(scheduled, this.issuePolicyRequest)) return;
      if (this.latestIssueRevision > this.policyKnownRevision) void this.refreshIssueAutomationPolicy();
    });
  }

  private detailCurrent(token: IssueRequestToken, issue: IssueEventReference): boolean {
    return this.issueCurrent(token, this.issueDetailRequest) && sameIssue(this.store.getKernel().issues.selectedIssue, issue);
  }

  private invalidateDetailAtRevision(revision: number): void {
    const issues = this.store.getKernel().issues;
    if (issues.selectedIssue === null || revision <= this.detailKnownRevision) return;
    this.detailKnownRevision = revision;
    const detailRevision = issues.detail?.status === "ready" || issues.detail?.status === "not_found" ? issues.detail.revision : -1;
    if (detailRevision >= revision) return;
    this.updateIssues({ detail: null, detailStale: true });
    const pending = this.activeIssueDetail;
    // 진행 중인 요청은 응답의 catalog 판으로 검사한다. 이벤트마다 같은 요청을 겹치지 않는다.
    if (pending !== null && this.detailCurrent(pending.token, pending.issue)) return;
    void this.readIssueDetail(issues.selectedIssue);
  }

  private async readIssueDetail(issue: IssueEventReference, retried = false): Promise<void> {
    if (!this.canRead("issues") || !sameIssue(this.store.getKernel().issues.selectedIssue, issue)) return;
    const token = this.issueToken(++this.issueDetailRequest);
    this.activeIssueDetail = { token, issue };
    this.detailKnownRevision = this.latestIssueRevision;
    this.updateIssues({ detail: null, detailLoading: true, detailError: null, detailStale: true });
    let restart = false;
    try {
      const detail = await this.client.request<unknown>(KERNEL_METHOD.query, { query: { type: "issue", issue } });
      if (!this.detailCurrent(token, issue)) return;
      if (!isIssueDetailAnswer(detail) || (detail.status === "ready" && !sameIssue(detail.item.issue, issue))) {
        throw new Error("이슈 상세 표시 계약이나 저장소 좌표가 일치하지 않습니다. Gateway를 업데이트하거나 다시 시작해 주세요.");
      }
      if (detail.status !== "unavailable" && detail.revision < this.latestIssueRevision) {
        restart = true;
      } else {
        if (detail.status !== "unavailable") {
          this.noteIssueRevision(detail.revision);
          this.detailKnownRevision = Math.max(this.detailKnownRevision, detail.revision);
        }
        this.updateIssues({ detail, detailStale: false });
        const page = this.store.getKernel().issues.page;
        if (detail.status !== "unavailable" && page?.status === "ready" && page.revision < detail.revision) {
          this.updateIssues({ stale: true, canGoBack: false });
          if (this.activeIssuePage?.after !== null) void this.readIssuesPage(null, [], null);
        }
      }
    } catch (error) {
      if (this.detailCurrent(token, issue)) {
        this.updateIssues({ detail: null, detailError: "이슈 상세를 읽지 못했습니다: " + failureMessage(error), detailStale: true });
      }
    } finally {
      if (this.detailCurrent(token, issue)) {
        this.activeIssueDetail = null;
        this.updateIssues({ detailLoading: false });
      }
    }
    if (restart && this.detailCurrent(token, issue)) {
      if (retried) {
        this.updateIssues({ detailError: "이슈 상세가 조회 중 바뀌었습니다. 다시 읽어 주세요.", detailStale: true });
      } else {
        await this.readIssueDetail(issue, true);
      }
    }
  }

  private async runIssueCommand(command: KernelCommand, pending: "mode" | "sync"): Promise<void> {
    if (!this.canRead("issues") || this.store.getKernel().issues.commandPending !== null) return;
    const token = this.issueToken(++this.issueCommandRequest);
    this.issueCommandError = null;
    this.updateIssues({ commandPending: pending });
    try {
      const response = await sendKernelCommand(this.client, command);
      if (!this.issueCurrent(token, this.issueCommandRequest)) return;
      if (response.rejection) {
        throw new Error(response.rejection.reason === "issue_automation_off"
          ? "이슈 수집이 꺼져 있습니다. 관찰로 바꾼 뒤 다시 동기화하세요."
          : `커널이 요청을 거절했습니다: ${response.rejection.message}`);
      }
      if (response.failure) throw new Error(response.failure);
      if (!response.processed) throw new Error("커널이 요청을 처리하지 못했습니다. 상태를 다시 읽어 주세요.");
      await this.refreshIssues();
    } catch (error) {
      if (this.issueCurrent(token, this.issueCommandRequest)) {
        this.issueCommandError = `이슈 요청을 완료하지 못했습니다: ${failureMessage(error)}`;
        this.updateIssues({});
      }
    } finally {
      if (this.issueCurrent(token, this.issueCommandRequest)) this.updateIssues({ commandPending: null });
    }
  }

  private applyOverview(event: KernelEvent): void {
    const view = this.store.getKernel();
    switch (event.type) {
      case "thread_activity":
        this.store.setKernel({ ...view, threads: event.threads });
        break;
      case "auto_follow":
        this.store.setKernel({ ...view, autoFollow: { max_turns: event.max_turns, paused: event.paused } });
        break;
      case "planning": {
        const { type: _type, ...planning } = event;
        this.store.setKernel({ ...view, planning });
        break;
      }
      case "parallel": {
        const { type: _type, ...parallel } = event;
        this.store.setKernel({ ...view, parallel });
        break;
      }
      default:
        break;
    }
  }
}
