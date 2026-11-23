import { vi } from "vitest";
import type { LinearIssue, LinearSummary } from "../src/shared/linear";
import type { ConnectionRef, OperationReceipt, Provider, ProviderCredentialBridge, ProviderCredentialState } from "../src/shared/providerCredentials";

export const PROFILE = "a".repeat(64);
export const SECOND_PROFILE = "b".repeat(64);
export const SECRET_CANARY = "synthetic-only-token-NOT-A-REAL-CREDENTIAL";
export function reference(provider: Provider = "linear", revision = 1, id = "c".repeat(32)): ConnectionRef {
  return { profileId: PROFILE, provider, connectionId: id, revision };
}
export function receipt(patch: Partial<OperationReceipt> = {}): OperationReceipt {
  return { operationId: "operation-1", profileId: PROFILE, provider: "linear", expectedRevision: 0, kind: "connect",
    status: "committed", connection: reference(), cleanupPending: false, legacySnapshotId: null,
    legacyCleanupAllowed: false, error: null, ...patch };
}
export function disconnected(provider: Provider = "linear", revision = 0): ProviderCredentialState {
  return { profileId: PROFILE, provider, revision, status: "disconnected", activeConnection: null, operation: null,
    capabilities: { nativeInput: true, legacyImport: true, keyring: true } };
}
export function connected(provider: Provider = "linear", revision = 1, id = "c".repeat(32)): ProviderCredentialState {
  return { ...disconnected(provider, revision), status: "connected", activeConnection: {
    reference: reference(provider, revision, id), metadata: { accountId: "viewer-1", accountName: "Fixture Account",
      configuration: provider === "jira" ? { siteUrl: "https://jira.example/jira", deployment: "server" } : {} },
  } };
}
export function linearIssue(number: number, patch: Partial<LinearIssue> = {}): LinearIssue {
  return { id: "issue-" + number, identifier: "APP-" + number, title: "Fixture issue " + number,
    url: "https://linear.app/fixture/issue/APP-" + number + "/fixture", priority: 2, updatedAt: "2026-11-23T00:00:00Z", branchName: null,
    state: { id: "started", type: "started", name: "In Progress" }, team: { id: "team-a", key: "APP", name: "App" },
    project: { id: "project-a", name: "Project A" }, cycle: { id: "cycle-1", number: 1, name: null, isActive: true }, ...patch };
}
export function linearSummary(issues = [linearIssue(1)]): LinearSummary {
  return { issues, teams: [], projects: [], totalCached: issues.length, limit: 30 };
}
export function nativeBridge() {
  return {
    legacyStatus: vi.fn<ProviderCredentialBridge["legacyStatus"]>(async () => ({ kind: "absent" })),
    migrateLegacy: vi.fn<ProviderCredentialBridge["migrateLegacy"]>(async () => ({ kind: "held", reason: "unsupported" })),
    prepareLegacyDisconnect: vi.fn<ProviderCredentialBridge["prepareLegacyDisconnect"]>(async () => ({ kind: "prepared" })),
    cleanupLegacy: vi.fn<ProviderCredentialBridge["cleanupLegacy"]>(async request => ({ kind: "committed", operationId: request.operationId, cleanupPending: false })),
    openExternal: vi.fn<ProviderCredentialBridge["openExternal"]>(async () => undefined),
  };
}
export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
