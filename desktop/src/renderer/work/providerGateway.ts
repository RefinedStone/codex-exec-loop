// Renderer facade: public metadata-only Gateway RPC plus the narrow main migration/link bridge.
import type { JiraBridge, JiraIssue, JiraSummary } from "../../shared/bridge";
import type { LinearBridge, LinearFilters, LinearIssue, LinearSummary } from "../../shared/linear";
import {
  credentialMessage, isCredentialError, isProfileId, isProvider, isRevision, operationPending,
  parseConfiguration, parseOperationReceipt, parseProviderState, publicRecord, sameConnection,
  type ConnectionConfiguration, type ConnectionRef, type CredentialError, type CredentialOperationRequest,
  type LegacyAvailability, type OperationReceipt, type Provider, type ProviderConnectionBridge,
  type ProviderCredentialBridge, type ProviderCredentialState,
} from "../../shared/providerCredentials";
import { isJiraIssueKey, isJiraIssueUrl, isLinearIssueUrl } from "../../shared/providerLinks";
import type { GatewayRequest } from "./notionHub";

const CREDENTIALS = "akra/providerCredentials/";
const READS = "akra/providers/";
const NO_FILTERS: LinearFilters = { teamId: null, projectId: null, currentCycle: false };
export interface ProviderGatewaySubscriptions {
  onChanged(listener: (change: unknown) => void): () => void;
  onConnectionChanged(listener: () => void): () => void;
}
export class ProviderGatewayError extends Error {
  constructor(readonly reason: CredentialError, message = credentialMessage(reason)) { super(message); }
}
function safeError(error: unknown): ProviderGatewayError {
  if (error instanceof ProviderGatewayError) return error;
  const raw = error && typeof error === "object" ? error as { code?: unknown; data?: unknown } : {};
  if (raw.code === -32601) return new ProviderGatewayError("unsupported");
  const data = raw.data && typeof raw.data === "object" ? raw.data as { kind?: unknown; reason?: unknown; error?: unknown } : {};
  const reason = isCredentialError(data.kind) ? data.kind : isCredentialError(data.reason) ? data.reason : isCredentialError(data.error) ? data.error : "unavailable";
  return new ProviderGatewayError(reason);
}
function key(reference: ConnectionRef | null | undefined): string {
  return reference ? [reference.profileId, reference.provider, reference.connectionId, reference.revision].join(":") : "";
}
function terminalFailure(receipt: OperationReceipt): boolean { return ["failed", "cancelled", "superseded"].includes(receipt.status); }
function deadline<T>(work: Promise<T>, milliseconds: number, reason: CredentialError): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new ProviderGatewayError(reason)), milliseconds);
    work.then(value => { clearTimeout(timer); resolve(value); }, error => { clearTimeout(timer); reject(error); });
  });
}

class ProviderConnection implements ProviderConnectionBridge {
  private state: ProviderCredentialState | undefined;
  private stateEpoch = 0;
  private acceptedStateEpoch = -1;
  private readEpoch = 0;
  private pendingState: Promise<ProviderCredentialState> | undefined;
  private pendingOperation: CredentialOperationRequest | undefined;
  private readonly listeners = new Set<(connectionChanged: boolean) => void>();
  private readonly invalidators = new Set<() => void>();
  constructor(
    readonly provider: Provider,
    private readonly request: GatewayRequest,
    private readonly native: ProviderCredentialBridge,
  ) {}
  onChanged(listener: (connectionChanged: boolean) => void): () => void {
    this.listeners.add(listener); return () => this.listeners.delete(listener);
  }
  onInvalidated(listener: () => void): () => void {
    this.invalidators.add(listener); return () => this.invalidators.delete(listener);
  }
  private invalidateReads(): void { this.readEpoch++; for (const invalidate of this.invalidators) invalidate(); }
  private emit(changed: boolean): void { for (const listener of this.listeners) listener(changed); }
  connectionChanged(): void {
    this.stateEpoch++; this.pendingState = undefined; this.state = undefined;
    this.invalidateReads(); this.emit(true);
  }
  changed(value: unknown): void {
    let raw: Record<string, unknown>;
    try { raw = publicRecord(value, ["provider", "profileId", "revision", "lagged"]); } catch { this.connectionChanged(); return; }
    if (!isProvider(raw.provider) || !isProfileId(raw.profileId) || !isRevision(raw.revision) || (raw.lagged !== undefined && typeof raw.lagged !== "boolean")) { this.connectionChanged(); return; }
    if (raw.provider !== this.provider) return;
    if (raw.lagged === true) { this.connectionChanged(); return; }
    if (this.state && raw.profileId === this.state.profileId && raw.revision < this.state.revision) return;
    const changed = !this.state || this.state.profileId !== raw.profileId || this.state.revision !== raw.revision;
    this.stateEpoch++; this.pendingState = undefined;
    if (changed) { this.state = undefined; this.invalidateReads(); }
    // Operation phases can change at the same active revision. They must not discard the
    // current account's cache or reject its in-flight reads after a cancelled reconnect.
    this.emit(changed);
  }
  status(): Promise<ProviderCredentialState> {
    if (this.pendingState) return this.pendingState;
    const epoch = this.stateEpoch;
    const pending = (async () => {
      try {
        const value = await deadline(this.request<unknown>(CREDENTIALS + "state", { provider: this.provider }), 30_000, "unavailable");
        if (epoch !== this.stateEpoch) return this.state && this.acceptedStateEpoch === this.stateEpoch ? this.state : this.status();
        const next = parseProviderState(value, this.provider);
        if (this.state && key(this.state.activeConnection?.reference) !== key(next.activeConnection?.reference)) this.invalidateReads();
        if (this.pendingOperation && (this.pendingOperation.profileId !== next.profileId ||
          (next.operation?.operationId === this.pendingOperation.operationId && !operationPending(next.operation)))) this.pendingOperation = undefined;
        this.state = next; this.acceptedStateEpoch = epoch;
        return next;
      } catch (error) { throw safeError(error); }
    })();
    this.pendingState = pending;
    void pending.finally(() => { if (this.pendingState === pending) this.pendingState = undefined; }).catch(() => {});
    return pending;
  }
  private async freshState(): Promise<ProviderCredentialState> {
    const old = key(this.state?.activeConnection?.reference);
    this.stateEpoch++; this.pendingState = undefined;
    const next = await this.status();
    this.emit(old !== key(next.activeConnection?.reference));
    return next;
  }
  private async current(): Promise<ProviderCredentialState> { return this.state ?? this.status(); }

  async connect(configuration: ConnectionConfiguration = {}): Promise<ProviderCredentialState> {
    const state = await this.current();
    if (!state.capabilities.nativeInput || !state.capabilities.keyring) throw new ProviderGatewayError("unsupported");
    const operationId = crypto.randomUUID();
    const request = { operationId, profileId: state.profileId, provider: this.provider, expectedRevision: state.revision, configuration: parseConfiguration(configuration) };
    this.pendingOperation = { operationId, profileId: state.profileId, provider: this.provider };
    return this.mutate("connect", request);
  }
  async disconnect(): Promise<ProviderCredentialState> {
    const state = await this.current();
    const operationId = crypto.randomUUID();
    const request = { operationId, profileId: state.profileId, provider: this.provider, expectedRevision: state.revision };
    this.pendingOperation = { operationId, profileId: state.profileId, provider: this.provider };
    // Snapshot legacy ciphertext before authority is revoked. A filesystem failure must not
    // prevent the Gateway from fencing the active account and deleting its keyring slot.
    await this.native.prepareLegacyDisconnect(request).catch(() => undefined);
    return this.mutate("disconnect", request);
  }
  async cancel(): Promise<ProviderCredentialState> {
    const request = await this.operationTarget();
    if (!request) return this.freshState();
    return this.mutate("cancel", request);
  }
  async retry(): Promise<ProviderCredentialState> {
    const state = await this.current();
    const operation = state.operation;
    if (operation?.kind === "disconnect" && operation.status === "committed" && operation.cleanupPending) {
      // Explicit deletion retry reuses the original operation/revision; queries never re-send OS writes.
      return this.mutate("disconnect", {
        profileId: operation.profileId, provider: this.provider,
        operationId: operation.operationId, expectedRevision: operation.expectedRevision,
      });
    }
    if ((await this.legacyStatus()).kind === "pending" && operation?.kind !== "disconnect") return this.migrateLegacy();
    const request = await this.operationTarget();
    if (!request) return this.freshState();
    return this.mutate("operation", request);
  }
  private async operationTarget(): Promise<CredentialOperationRequest | undefined> {
    const state = await this.current();
    const operation = state.operation;
    if (operation && operationPending(operation)) return { profileId: state.profileId, provider: this.provider, operationId: operation.operationId };
    if (this.pendingOperation?.profileId === state.profileId) return this.pendingOperation;
    return operation ? { profileId: state.profileId, provider: this.provider, operationId: operation.operationId } : undefined;
  }
  private async mutate(method: string, request: CredentialOperationRequest | object): Promise<ProviderCredentialState> {
    let receipt: OperationReceipt;
    try {
      receipt = parseOperationReceipt(await deadline(
        this.request<unknown>(CREDENTIALS + method, request), 140_000, "outcomeUnknown",
      ));
      const expected = request as CredentialOperationRequest;
      if (receipt.profileId !== expected.profileId || receipt.provider !== this.provider || receipt.operationId !== expected.operationId) throw new ProviderGatewayError("conflict");
    } catch (error) {
      // Do not issue another connect with a new operation ID when the response is lost.
      await this.freshState().catch(() => undefined);
      throw safeError(error);
    }
    if (this.pendingOperation?.operationId === receipt.operationId && !operationPending(receipt)) this.pendingOperation = undefined;
    const state = await this.freshState();
    if (receipt.kind === "disconnect" && receipt.status === "committed") {
      const result = await this.native.cleanupLegacy({ profileId: receipt.profileId, provider: this.provider, operationId: receipt.operationId });
      if (result.kind !== "committed" || result.cleanupPending) throw new ProviderGatewayError("unavailable",
        receipt.cleanupPending ? "연결은 해제했습니다. 저장소에서 남은 자격 증명을 지우려면 삭제 다시 시도를 눌러 주세요."
          : "연결은 해제했습니다. 기존 암호문 정리가 남았습니다. 기존 이관을 확인하거나 남은 기존 연결 삭제를 선택하세요.");
    }
    if (terminalFailure(receipt)) throw new ProviderGatewayError(receipt.error ?? (receipt.status === "cancelled" ? "cancelled" : "conflict"));
    if (receipt.status === "outcomeUnknown") throw new ProviderGatewayError("outcomeUnknown");
    return state;
  }
  async legacyStatus(): Promise<LegacyAvailability> {
    // Unsupported/remote Gateway paths must not probe local legacy credentials.
    const state = await this.current();
    if (!state.capabilities.legacyImport) return { kind: "unavailable" };
    return this.native.legacyStatus(this.provider);
  }
  async migrateLegacy(): Promise<ProviderCredentialState> {
    const state = await this.current();
    if (!state.capabilities.legacyImport || !state.capabilities.keyring) throw new ProviderGatewayError("unsupported");
    const result = await this.native.migrateLegacy({ profileId: state.profileId, provider: this.provider, expectedRevision: state.revision });
    const current = await this.freshState();
    if (result.kind === "held") throw new ProviderGatewayError(result.reason);
    if (result.kind === "unknown") {
      this.pendingOperation = { profileId: state.profileId, provider: this.provider, operationId: result.operationId };
      throw new ProviderGatewayError("outcomeUnknown");
    }
    if (result.cleanupPending) throw new ProviderGatewayError("unavailable", "기존 연결을 옮겼습니다. 남은 암호문 정리를 위해 상태 확인을 눌러 주세요.");
    return current;
  }
  async read<T>(method: string, params: object, parse: (value: unknown) => T): Promise<T> {
    const state = await this.current();
    const connection = state.activeConnection?.reference;
    if (!connection) throw new ProviderGatewayError("notConnected");
    const epoch = this.readEpoch;
    try {
      const value = await deadline(this.request<unknown>(READS + this.provider + "/" + method, { connection, ...params }), 30_000, "unavailable");
      if (epoch !== this.readEpoch || !this.state?.activeConnection || !sameConnection(connection, this.state.activeConnection.reference)) throw new ProviderGatewayError("conflict");
      return parse(value);
    } catch (error) { throw safeError(error); }
  }
  async openExternal(url: string): Promise<void> {
    const connection = (await this.current()).activeConnection?.reference;
    if (!connection) throw new ProviderGatewayError("notConnected");
    try { await this.native.openExternal({ connection, url }); } catch { throw new ProviderGatewayError("conflict", "현재 연결의 이슈 주소를 확인할 수 없습니다."); }
  }
}

function text(value: unknown, maximum: number): string {
  if (typeof value !== "string" || value.length > maximum) throw new ProviderGatewayError("corrupt");
  return value;
}
function identifier(value: unknown): string {
  const result = text(value, 128);
  if (!/^[A-Za-z0-9_-]+$/.test(result)) throw new ProviderGatewayError("corrupt");
  return result;
}
function parsePrompt(value: unknown): string {
  const prompt = publicRecord(value, ["text"]).text;
  // The Gateway clips Rust chars() (Unicode scalar values), not UTF-16 code units.
  if (typeof prompt !== "string" || Array.from(prompt).length > 20_000) throw new ProviderGatewayError("corrupt");
  return prompt;
}
function parseJiraSummary(value: unknown): JiraSummary {
  const raw = publicRecord(value, ["siteUrl", "issues"]);
  const siteUrl = text(raw.siteUrl, 2_048);
  if (!Array.isArray(raw.issues) || raw.issues.length > 30) throw new ProviderGatewayError("corrupt");
  const issues = raw.issues.map((value: unknown): JiraIssue => {
    const item = publicRecord(value, ["key", "summary", "url", "status", "statusCategory", "issueType", "priority", "updatedAt"]);
    if (!isJiraIssueKey(item.key) || !isJiraIssueUrl(siteUrl, item.url)) throw new ProviderGatewayError("corrupt");
    const nullable = (field: unknown) => field === null ? null : text(field, 1_000);
    return { key: item.key, summary: text(item.summary, 2_000), url: item.url, status: nullable(item.status),
      statusCategory: nullable(item.statusCategory), issueType: nullable(item.issueType), priority: nullable(item.priority), updatedAt: text(item.updatedAt, 64) };
  });
  return { siteUrl, issues };
}
function parseLinearSummary(value: unknown): LinearSummary {
  const raw = publicRecord(value, ["issues", "teams", "projects", "totalCached", "limit"]);
  if (!Array.isArray(raw.issues) || raw.issues.length > 30 || raw.limit !== 30) throw new ProviderGatewayError("corrupt");
  const issues = raw.issues.map((value: unknown): LinearIssue => {
    const item = publicRecord(value, ["id", "identifier", "title", "url", "priority", "updatedAt", "branchName", "state", "team", "project", "cycle"]);
    const state = publicRecord(item.state, ["id", "name", "type"]);
    const team = publicRecord(item.team, ["id", "key", "name"]);
    const project = item.project === null ? null : publicRecord(item.project, ["id", "name"]);
    const cycle = item.cycle === null ? null : publicRecord(item.cycle, ["id", "number", "name", "isActive"]);
    if (!isLinearIssueUrl(item.url) || !Number.isInteger(item.priority) || (item.priority as number) < 0 || (item.priority as number) > 4 ||
      !/^[A-Za-z][A-Za-z0-9_]*-\d+$/.test(String(item.identifier)) ||
      !["triage", "backlog", "unstarted", "started"].includes(String(state.type)) ||
      (cycle && (!Number.isSafeInteger(cycle.number) || (cycle.number as number) < 1 || typeof cycle.isActive !== "boolean"))) throw new ProviderGatewayError("corrupt");
    return {
      id: identifier(item.id), identifier: text(item.identifier, 128), title: text(item.title, 2_000), url: item.url,
      priority: item.priority as number, updatedAt: text(item.updatedAt, 64), branchName: item.branchName === null ? null : text(item.branchName, 2_000),
      state: { id: identifier(state.id), name: text(state.name, 1_000), type: text(state.type, 32) },
      team: { id: identifier(team.id), key: text(team.key, 128), name: text(team.name, 1_000) },
      project: project ? { id: identifier(project.id), name: text(project.name, 1_000) } : null,
      cycle: cycle ? { id: identifier(cycle.id), number: cycle.number as number, name: cycle.name === null ? null : text(cycle.name, 1_000), isActive: cycle.isActive as boolean } : null,
    };
  });
  if (new Set(issues.map(item => item.id)).size !== issues.length) throw new ProviderGatewayError("corrupt");
  return {
    issues, teams: [...new Map(issues.map(item => [item.team.id, { id: item.team.id, name: item.team.name }])).values()],
    projects: [...new Map(issues.flatMap(item => item.project ? [[item.project.id, item.project] as const] : [])).values()],
    totalCached: issues.length, limit: 30,
  };
}
function bridge(connection: ProviderConnection): ProviderConnectionBridge {
  return {
    status: () => connection.status(), connect: configuration => connection.connect(configuration),
    disconnect: () => connection.disconnect(), cancel: () => connection.cancel(), retry: () => connection.retry(),
    legacyStatus: () => connection.legacyStatus(), migrateLegacy: () => connection.migrateLegacy(),
    onChanged: listener => connection.onChanged(listener),
  };
}
export function createProviderGateway(request: GatewayRequest, native: ProviderCredentialBridge, subscriptions: ProviderGatewaySubscriptions): {
  jira: JiraBridge; linear: LinearBridge; start(): void; dispose(): void;
} {
  const jira = new ProviderConnection("jira", request, native);
  const linear = new ProviderConnection("linear", request, native);
  let cached: LinearSummary | undefined;
  let pending: Promise<LinearSummary> | undefined;
  let cacheEpoch = 0;
  linear.onInvalidated(() => { cacheEpoch++; cached = undefined; pending = undefined; });
  let unsubscribers: Array<() => void> = [];
  return {
    start() {
      if (unsubscribers.length) return;
      unsubscribers = [
        subscriptions.onChanged(change => { jira.changed(change); linear.changed(change); }),
        subscriptions.onConnectionChanged(() => { jira.connectionChanged(); linear.connectionChanged(); }),
      ];
    },
    jira: {
      ...bridge(jira), summary: () => jira.read("summary", {}, parseJiraSummary),
      issuePrompt: issueKey => {
        if (!isJiraIssueKey(issueKey)) return Promise.reject(new ProviderGatewayError("invalidRequest"));
        return jira.read("issuePrompt", { key: issueKey }, parsePrompt);
      },
      currentVersion: query => jira.read("currentVersion", { key: query.key, url: query.url }, value => {
        const version = text(publicRecord(value, ["version"]).version, 64);
        if (!/^\d{4}-\d{2}-\d{2}T/.test(version) || !Number.isFinite(Date.parse(version))) throw new ProviderGatewayError("corrupt");
        return { version };
      }),
      openExternal: url => jira.openExternal(url),
    },
    linear: {
      ...bridge(linear),
      async summary(filters, refresh) {
        const selected = publicRecord(filters, ["teamId", "projectId", "currentCycle"]);
        if (typeof refresh !== "boolean" || typeof selected.currentCycle !== "boolean") throw new ProviderGatewayError("invalidRequest");
        const teamId = selected.teamId === null ? null : identifier(selected.teamId);
        const projectId = selected.projectId === null ? null : identifier(selected.projectId);
        if (refresh || !cached) {
          if (!pending) {
            const epoch = cacheEpoch;
            const work = linear.read("summary", { filters: NO_FILTERS, refresh }, parseLinearSummary).then(result => {
              if (epoch !== cacheEpoch) throw new ProviderGatewayError("conflict");
              cached = result; return result;
            });
            pending = work;
            void work.finally(() => { if (pending === work) pending = undefined; }).catch(() => {});
          }
        }
        // A filter change joins an already running refresh even when an older cache exists.
        // It does not issue a new request or make the late refresh disappear from the visible filter.
        if (pending) await pending;
        if (!cached) throw new ProviderGatewayError("conflict");
        return { ...cached, issues: cached.issues.filter(item => (!teamId || item.team.id === teamId) &&
          (!projectId || item.project?.id === projectId) && (!selected.currentCycle || item.cycle?.isActive === true)) };
      },
      issuePrompt: issueId => linear.read("issuePrompt", { id: identifier(issueId) }, parsePrompt),
      openExternal: url => linear.openExternal(url),
    },
    dispose() { for (const unsubscribe of unsubscribers.splice(0)) unsubscribe(); jira.connectionChanged(); linear.connectionChanged(); },
  };
}
