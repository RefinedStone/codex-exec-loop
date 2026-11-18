import { readFile, rename, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { randomUUID } from "node:crypto";

import type { LinearFilters, LinearIssue, LinearStatus, LinearSummary } from "../shared/linear";
import type { SecretBox } from "./jiraService";

const ENDPOINT = "https://api.linear.app/graphql";
const REQUEST_TIMEOUT_MS = 20_000;
const RESPONSE_LIMIT = 8 << 20;
const OMITTED = "\n[Content omitted]";
const ISSUE_FIELDS = `
  id identifier title url priority updatedAt branchName
  state { id name type }
  team { id key name }
  project { id name }
  cycle { id number name isActive }
`;
const VIEWER_QUERY = "query AkraLinearViewer { viewer { id name } }";
const ISSUES_QUERY = `query AkraLinearAssignedIssues {
  viewer {
    assignedIssues(first:30, orderBy:updatedAt, includeArchived:false,
      filter:{state:{type:{nin:["completed","canceled","duplicate"]}}}) {
      nodes { ${ISSUE_FIELDS} }
    }
  }
}`;
const DETAIL_QUERY = `query AkraLinearIssue($id: String!) {
  issue(id:$id) {
    ${ISSUE_FIELDS}
    description
    comments(first:10, orderBy:createdAt) {
      nodes { body createdAt user { name } }
      pageInfo { hasNextPage }
    }
  }
}`;

export type LinearFetch = (url: string, init: {
  method: "POST";
  headers: Record<string, string>;
  body: string;
  redirect: "error";
  credentials: "omit";
  signal: AbortSignal;
}) => Promise<Pick<Response, "status" | "redirected" | "headers" | "body">>;

interface StoredConnection {
  token: string;
  viewer: { id: string; name: string };
}

function invalid(): never {
  throw new Error("Linear returned invalid data.");
}

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) invalid();
  return value as Record<string, unknown>;
}

function text(value: unknown, max = 1_000): string {
  if (typeof value !== "string" || !value.trim() || value.length > max) invalid();
  return value;
}

function id(value: unknown): string {
  if (typeof value !== "string" || !/^[A-Za-z0-9_-]{1,128}$/.test(value)) invalid();
  return value;
}

function nullableText(value: unknown, max: number): string | null {
  if (value === null) return null;
  if (typeof value !== "string" || value.length > max) invalid();
  return value;
}

function timestamp(value: unknown): string {
  const result = text(value, 64);
  if (!/^\d{4}-\d{2}-\d{2}T/.test(result) || !Number.isFinite(Date.parse(result))) invalid();
  return result;
}

function boolean(value: unknown): boolean {
  if (typeof value !== "boolean") invalid();
  return value;
}

function nodes(value: unknown, limit: number): unknown[] {
  const result = object(value).nodes;
  if (!Array.isArray(result) || result.length > limit) invalid();
  return result;
}

export function isLinearIssueUrl(input: unknown): input is string {
  if (typeof input !== "string" || input.length > 2_048
    || !input.startsWith("https://linear.app/") || /[\s\\\u0000-\u001f\u007f]/.test(input)) return false;
  try {
    const url = new URL(input);
    return url.protocol === "https:" && url.hostname === "linear.app" && !url.port
      && !url.username && !url.password
      && /^\/[A-Za-z0-9_-]+\/issue\/[A-Za-z][A-Za-z0-9_]*-\d+(?:\/[A-Za-z0-9_%.-]+)?\/?$/.test(url.pathname)
      && !/%(?:2f|5c|00|0a|0d)/i.test(url.pathname)
      && !input.split(/[?#]/, 1)[0]!.split("/").some((part) => part === "." || part === "..");
  } catch {
    return false;
  }
}

function issue(value: unknown): LinearIssue {
  const raw = object(value);
  const state = object(raw.state);
  const team = object(raw.team);
  const project = raw.project === null ? null : object(raw.project);
  const cycle = raw.cycle === null ? null : object(raw.cycle);
  const identifier = text(raw.identifier, 128);
  if (!/^[A-Za-z][A-Za-z0-9_]*-\d+$/.test(identifier) || !isLinearIssueUrl(raw.url)) invalid();
  if (typeof raw.priority !== "number" || !Number.isInteger(raw.priority) || raw.priority < 0 || raw.priority > 4) invalid();
  if (cycle && (typeof cycle.number !== "number" || !Number.isSafeInteger(cycle.number) || cycle.number < 1)) invalid();
  const type = text(state.type, 32);
  if (!["triage", "backlog", "unstarted", "started", "completed", "canceled", "duplicate"].includes(type)) invalid();
  return {
    id: id(raw.id), identifier, title: text(raw.title, 2_000), url: raw.url,
    priority: raw.priority, updatedAt: timestamp(raw.updatedAt),
    branchName: nullableText(raw.branchName, 2_000),
    state: { id: id(state.id), name: text(state.name), type },
    team: { id: id(team.id), key: text(team.key, 128), name: text(team.name) },
    project: project ? { id: id(project.id), name: text(project.name) } : null,
    cycle: cycle ? {
      id: id(cycle.id), number: cycle.number as number,
      name: nullableText(cycle.name, 1_000), isActive: boolean(cycle.isActive),
    } : null,
  };
}

function filters(input: unknown): LinearFilters {
  const raw = object(input);
  return {
    teamId: raw.teamId === null ? null : id(raw.teamId),
    projectId: raw.projectId === null ? null : id(raw.projectId),
    currentCycle: boolean(raw.currentCycle),
  };
}

function token(input: unknown): string {
  if (typeof input !== "string" || !input.trim() || input.length > 4_096 || /[\u0000-\u0020\u007f]/.test(input.trim())) {
    throw new Error("Enter a valid Linear personal API key.");
  }
  return input.trim();
}

function clip(value: string, limit: number): string {
  return value.length > limit ? value.slice(0, limit - OMITTED.length) + OMITTED : value;
}

function prompt(raw: unknown): string {
  const item = issue(raw);
  const detail = object(raw);
  if (detail.description !== null && typeof detail.description !== "string") invalid();
  const comments = nodes(detail.comments, 10).map((value) => {
    const comment = object(value);
    if (typeof comment.body !== "string") invalid();
    const author = comment.user === null ? "Unknown author" : text(object(comment.user).name);
    return clip(`${author} (${timestamp(comment.createdAt)}):\n${comment.body}`, 2_000);
  });
  const moreComments = boolean(object(object(detail.comments).pageInfo).hasNextPage);
  const header = [
    "Work on this Linear issue.",
    `Issue: ${item.identifier} ${item.title}`,
    `URL: ${item.url}`,
    `Branch: ${item.branchName ?? "(none)"}`,
    ...(item.branchName ? ["Create the working branch with this suggested name."] : []),
    `State: ${item.state.name}`,
    `Team: ${item.team.name}`,
    `Project: ${item.project?.name ?? "(none)"}`,
    `Cycle: ${item.cycle ? `${item.cycle.number} ${item.cycle.name ?? ""}` : "(none)"}`,
    `Priority: ${item.priority}`,
  ].join("\n");
  const parts = [header, "", "Description:", clip(detail.description ?? "(No description)", 8_000)];
  if (comments.length) parts.push("", "Recent comments (newest first):", ...comments);
  if (moreComments) parts.push("[Older comments omitted]");
  return clip(parts.join("\n"), 20_000);
}

const defaultFetch: LinearFetch = async (url, init) => {
  const { net } = await import("electron");
  return net.fetch(url, init);
};

export class LinearService {
  constructor(
    private readonly directory: string,
    private readonly secrets: SecretBox,
    private readonly fetcher: LinearFetch = defaultFetch,
  ) {}

  private stored: StoredConnection | null | undefined;
  private revision = 0;
  private storage: Promise<unknown> = Promise.resolve();
  private readonly requests = new Set<AbortController>();
  private cached: readonly LinearIssue[] | null = null;
  private pendingSummary: Promise<readonly LinearIssue[]> | null = null;

  private get tokenPath(): string {
    return join(this.directory, "linear-token.bin");
  }

  private exclusive<T>(work: () => Promise<T>): Promise<T> {
    const result = this.storage.then(work);
    this.storage = result.catch(() => {});
    return result;
  }

  private invalidate(): number {
    this.revision += 1;
    for (const request of this.requests) request.abort();
    this.cached = null;
    this.pendingSummary = null;
    return this.revision;
  }

  private assertCurrent(revision: number): void {
    if (revision !== this.revision) throw new Error("Linear connection changed. Try again.");
  }

  private async load(): Promise<StoredConnection | null> {
    return this.exclusive(async () => {
      if (this.stored !== undefined) return this.stored;
      const revision = this.revision;
      let stored: StoredConnection | null = null;
      try {
        if (this.secrets.isEncryptionAvailable()) {
          const raw = object(JSON.parse(this.secrets.decryptString(await readFile(this.tokenPath))));
          const viewer = object(raw.viewer);
          stored = { token: token(raw.token), viewer: { id: id(viewer.id), name: text(viewer.name) } };
        }
      } catch {
        // Missing, corrupt, or inaccessible encrypted credentials are disconnected.
      }
      if (revision === this.revision) this.stored = stored;
      return revision === this.revision ? stored : null;
    });
  }

  private async require(revision: number): Promise<StoredConnection> {
    const stored = await this.load();
    this.assertCurrent(revision);
    if (!stored) throw new Error("Linear is not connected.");
    return stored;
  }

  private async query(key: string, query: string, variables = {}): Promise<Record<string, unknown>> {
    const controller = new AbortController();
    this.requests.add(controller);
    let rejectAbort: () => void = () => {};
    const aborted = new Promise<never>((_resolve, reject) => {
      rejectAbort = () => reject(new Error("Linear request canceled or timed out."));
      controller.signal.addEventListener("abort", rejectAbort, { once: true });
    });
    const timer = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
    try {
      return await Promise.race([aborted, (async () => {
        const response = await this.fetcher(ENDPOINT, {
          method: "POST", headers: { Authorization: key, Accept: "application/json", "Content-Type": "application/json" },
          body: JSON.stringify({ query, variables }), redirect: "error", credentials: "omit",
          signal: controller.signal,
        });
        if (controller.signal.aborted) throw new Error("Linear request canceled.");
        if (response.redirected || response.status < 200 || response.status >= 300) {
          throw new Error("Linear request failed. Check the API key and try again.");
        }
        const length = response.headers.get("content-length");
        if (length !== null && Number(length) > RESPONSE_LIMIT) throw new Error("Linear response is too large.");
        if (!response.body) invalid();
        const reader = response.body.getReader();
        const cancel = () => { void reader.cancel().catch(() => {}); };
        controller.signal.addEventListener("abort", cancel, { once: true });
        let size = 0;
        const chunks: Buffer[] = [];
        try {
          for (;;) {
            const chunk = await reader.read();
            if (controller.signal.aborted) throw new Error("Linear request canceled.");
            if (chunk.done) break;
            size += chunk.value.byteLength;
            if (size > RESPONSE_LIMIT) {
              cancel();
              throw new Error("Linear response is too large.");
            }
            chunks.push(Buffer.from(chunk.value));
          }
        } finally {
          controller.signal.removeEventListener("abort", cancel);
          reader.releaseLock();
        }
        const envelope = object(JSON.parse(Buffer.concat(chunks, size).toString("utf8")));
        if ("errors" in envelope && (!Array.isArray(envelope.errors) || envelope.errors.length !== 0)) invalid();
        return object(envelope.data);
      })()]);
    } catch {
      // Never expose HTTP bodies, GraphQL messages, or network errors that can contain the key.
      throw new Error("Linear request failed. Check the connection and personal API key.");
    } finally {
      clearTimeout(timer);
      controller.signal.removeEventListener("abort", rejectAbort);
      this.requests.delete(controller);
    }
  }

  async status(): Promise<LinearStatus> {
    const stored = await this.load();
    return stored ? { connected: true, viewerName: stored.viewer.name } : { connected: false };
  }

  async connect(input: unknown): Promise<LinearStatus> {
    const key = token(input);
    if (!this.secrets.isEncryptionAvailable()) throw new Error("OS encryption is unavailable. Linear key was not saved.");
    let revision = this.invalidate();
    const data = await this.query(key, VIEWER_QUERY);
    this.assertCurrent(revision);
    const raw = object(data.viewer);
    const stored: StoredConnection = { token: key, viewer: { id: id(raw.id), name: text(raw.name) } };
    const encrypted = this.secrets.encryptString(JSON.stringify(stored));
    await this.exclusive(async () => {
      this.assertCurrent(revision);
      const temporary = `${this.tokenPath}.${randomUUID()}.tmp`;
      try {
        const previous = await readFile(this.tokenPath).catch((error: NodeJS.ErrnoException) => {
          if (error.code === "ENOENT") return null;
          throw error;
        });
        this.assertCurrent(revision);
        await writeFile(temporary, encrypted, { mode: 0o600 });
        this.assertCurrent(revision);
        await rename(temporary, this.tokenPath);
        if (revision !== this.revision) {
          if (previous) {
            await writeFile(temporary, previous, { mode: 0o600 });
            await rename(temporary, this.tokenPath);
          } else {
            await rm(this.tokenPath, { force: true });
          }
        }
        this.assertCurrent(revision);
        this.stored = stored;
        revision = this.invalidate();
      } catch {
        throw new Error("Linear key was not saved.");
      } finally {
        await rm(temporary, { force: true });
      }
    });
    this.assertCurrent(revision);
    return { connected: true, viewerName: stored.viewer.name };
  }

  async disconnect(): Promise<LinearStatus> {
    this.invalidate();
    this.stored = null;
    await this.exclusive(async () => { await rm(this.tokenPath, { force: true }); });
    return { connected: false };
  }

  async summary(input: unknown, refresh: boolean): Promise<LinearSummary> {
    const selected = filters(input);
    boolean(refresh);
    const revision = this.revision;
    const stored = await this.require(revision);
    if (refresh || this.cached === null) {
      if (!this.pendingSummary) {
        const pending = (async () => {
          const data = await this.query(stored.token, ISSUES_QUERY);
          this.assertCurrent(revision);
          const result = nodes(object(data.viewer).assignedIssues, 30).map(issue)
            .filter((item) => !["completed", "canceled", "duplicate"].includes(item.state.type));
          if (new Set(result.map((item) => item.id)).size !== result.length) invalid();
          this.cached = result;
          return result;
        })();
        this.pendingSummary = pending;
        void pending.finally(() => {
          if (this.pendingSummary === pending) this.pendingSummary = null;
        }).catch(() => {});
      }
      await this.pendingSummary;
    }
    this.assertCurrent(revision);
    const all = this.cached!;
    const issues = all.filter((item) => (!selected.teamId || item.team.id === selected.teamId)
      && (!selected.projectId || item.project?.id === selected.projectId)
      && (!selected.currentCycle || item.cycle?.isActive === true));
    return {
      issues,
      teams: [...new Map(all.map((item) => [item.team.id, { id: item.team.id, name: item.team.name }])).values()],
      projects: [...new Map(all.flatMap((item) => item.project ? [[item.project.id, item.project] as const] : [])).values()],
      totalCached: all.length, limit: 30,
    };
  }

  async issuePrompt(input: unknown): Promise<string> {
    const issueId = id(input);
    const revision = this.revision;
    const stored = await this.require(revision);
    const data = await this.query(stored.token, DETAIL_QUERY, { id: issueId });
    this.assertCurrent(revision);
    if (object(data.issue).id !== issueId) invalid();
    return prompt(data.issue);
  }
}
