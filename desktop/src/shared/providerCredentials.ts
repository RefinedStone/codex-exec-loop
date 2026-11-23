// Provider credential wire values. A secret never belongs in a public DTO.
export type Provider = "jira" | "linear";
export type JiraDeployment = "cloud" | "server";
export interface ConnectionConfiguration {
  readonly siteUrl?: string;
  readonly deployment?: JiraDeployment;
  readonly email?: string;
}
export interface ConnectionRef {
  readonly profileId: string;
  readonly provider: Provider;
  readonly connectionId: string;
  readonly revision: number;
}
export interface ConnectionMetadata {
  readonly configuration: ConnectionConfiguration;
  readonly accountId: string;
  readonly accountName: string;
}
export interface ConnectionSummary {
  readonly reference: ConnectionRef;
  readonly metadata: ConnectionMetadata;
}
export interface ConnectRequest {
  readonly operationId: string;
  readonly profileId: string;
  readonly provider: Provider;
  readonly expectedRevision: number;
  readonly configuration: ConnectionConfiguration;
}
export type DisconnectRequest = Omit<ConnectRequest, "configuration">;
export type CredentialOperationRequest = Omit<DisconnectRequest, "expectedRevision">;
export type CredentialOperationKind = "connect" | "legacyImport" | "disconnect";
export type CredentialOperationStatus = "awaitingInput" | "prepared" | "verifying" | "storing" | "committed" | "cancelled" | "superseded" | "failed" | "outcomeUnknown";
export type CredentialError = "invalidRequest" | "unsupported" | "locked" | "unavailable" | "conflict" | "busy" | "cancelled" | "outcomeUnknown" | "corrupt" | "tooLarge" | "capacityExceeded" | "notConnected" | "notFound" | "unauthorized";
export interface OperationReceipt {
  readonly operationId: string;
  readonly profileId: string;
  readonly provider: Provider;
  readonly expectedRevision: number;
  readonly kind: CredentialOperationKind;
  readonly status: CredentialOperationStatus;
  readonly connection: ConnectionRef | null;
  readonly cleanupPending: boolean;
  readonly legacySnapshotId: string | null;
  readonly legacyCleanupAllowed: boolean;
  readonly error: CredentialError | null;
}
export interface ProviderCredentialState {
  readonly profileId: string;
  readonly provider: Provider;
  readonly revision: number;
  readonly status: "disconnected" | "connected";
  readonly activeConnection: ConnectionSummary | null;
  readonly operation: OperationReceipt | null;
  readonly capabilities: {
    readonly nativeInput: boolean;
    readonly legacyImport: boolean;
    readonly keyring: boolean;
  };
}
export interface LegacyMigrationTarget {
  readonly profileId: string;
  readonly provider: Provider;
  readonly expectedRevision: number;
}
export type LegacyAvailability = { readonly kind: "absent" | "available" | "pending" | "unavailable" };
export type LegacyMigrationResult =
  | { readonly kind: "committed"; readonly operationId: string; readonly cleanupPending: boolean }
  | { readonly kind: "held"; readonly reason: CredentialError }
  | { readonly kind: "unknown"; readonly operationId: string };
export type LegacyDisconnectPreparation = { readonly kind: "prepared" } | { readonly kind: "held"; readonly reason: CredentialError };
export interface ProviderLinkRequest {
  readonly connection: ConnectionRef;
  readonly url: string;
}
/** The only provider methods exposed by Electron main. No caller supplies a file path or token. */
export interface ProviderCredentialBridge {
  legacyStatus(provider: Provider): Promise<LegacyAvailability>;
  migrateLegacy(target: LegacyMigrationTarget): Promise<LegacyMigrationResult>;
  prepareLegacyDisconnect(request: DisconnectRequest): Promise<LegacyDisconnectPreparation>;
  cleanupLegacy(request: CredentialOperationRequest): Promise<LegacyMigrationResult>;
  openExternal(request: ProviderLinkRequest): Promise<void>;
}

const ERROR_CODES: readonly string[] = ["invalidRequest", "unsupported", "locked", "unavailable", "conflict", "busy", "cancelled", "outcomeUnknown", "corrupt", "tooLarge", "capacityExceeded", "notConnected", "notFound", "unauthorized"];
const OPERATION_STATUSES: readonly string[] = ["awaitingInput", "prepared", "verifying", "storing", "committed", "cancelled", "superseded", "failed", "outcomeUnknown"];
export function isProvider(value: unknown): value is Provider { return value === "jira" || value === "linear"; }
export function isProfileId(value: unknown): value is string { return typeof value === "string" && /^[a-f0-9]{64}$/.test(value); }
export function isSnapshotId(value: unknown): value is string { return isProfileId(value); }
export function isOperationId(value: unknown): value is string { return typeof value === "string" && /^[A-Za-z0-9_-]{1,64}$/.test(value); }
export function isRevision(value: unknown): value is number { return typeof value === "number" && Number.isSafeInteger(value) && value >= 0; }
export function isCredentialError(value: unknown): value is CredentialError { return typeof value === "string" && ERROR_CODES.includes(value); }
function invalid(): never { throw new Error("자격 증명 서비스의 응답이 올바르지 않습니다."); }
export function publicRecord(value: unknown, allowed: readonly string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).some(key => !allowed.includes(key))) invalid();
  return value as Record<string, unknown>;
}
function plain(value: unknown, maximum: number): value is string {
  return typeof value === "string" && value.trim().length > 0 && new TextEncoder().encode(value).length <= maximum && !/[\u0000-\u001f\u007f]/.test(value);
}
export function parseConfiguration(value: unknown): ConnectionConfiguration {
  const raw = publicRecord(value, ["siteUrl", "deployment", "email"]);
  if (raw.siteUrl !== undefined && !plain(raw.siteUrl, 2_048)) invalid();
  if (raw.deployment !== undefined && raw.deployment !== "cloud" && raw.deployment !== "server") invalid();
  if (raw.email !== undefined && !plain(raw.email, 320)) invalid();
  return {
    ...(raw.siteUrl === undefined ? {} : { siteUrl: raw.siteUrl as string }),
    ...(raw.deployment === undefined ? {} : { deployment: raw.deployment as JiraDeployment }),
    ...(raw.email === undefined ? {} : { email: raw.email as string }),
  };
}
export function parseConnectionRef(value: unknown): ConnectionRef {
  const raw = publicRecord(value, ["profileId", "provider", "connectionId", "revision"]);
  if (!isProfileId(raw.profileId) || !isProvider(raw.provider) || typeof raw.connectionId !== "string" ||
    !/^[a-f0-9]{32}$/.test(raw.connectionId) || !isRevision(raw.revision)) invalid();
  return { profileId: raw.profileId, provider: raw.provider, connectionId: raw.connectionId, revision: raw.revision };
}
export function parseOperationRequest(value: unknown): CredentialOperationRequest {
  const raw = publicRecord(value, ["profileId", "provider", "operationId"]);
  if (!isProfileId(raw.profileId) || !isProvider(raw.provider) || !isOperationId(raw.operationId)) invalid();
  return { profileId: raw.profileId, provider: raw.provider, operationId: raw.operationId };
}
export function parseDisconnectRequest(value: unknown): DisconnectRequest {
  const raw = publicRecord(value, ["profileId", "provider", "operationId", "expectedRevision"]);
  if (!isProfileId(raw.profileId) || !isProvider(raw.provider) || !isOperationId(raw.operationId) || !isRevision(raw.expectedRevision)) invalid();
  return { profileId: raw.profileId, provider: raw.provider, operationId: raw.operationId, expectedRevision: raw.expectedRevision };
}
export function parseLegacyTarget(value: unknown): LegacyMigrationTarget {
  const raw = publicRecord(value, ["profileId", "provider", "expectedRevision"]);
  if (!isProfileId(raw.profileId) || !isProvider(raw.provider) || !isRevision(raw.expectedRevision)) invalid();
  return { profileId: raw.profileId, provider: raw.provider, expectedRevision: raw.expectedRevision };
}
export function parseOperationReceipt(value: unknown): OperationReceipt {
  const raw = publicRecord(value, ["operationId", "profileId", "provider", "expectedRevision", "kind", "status", "connection", "cleanupPending", "legacySnapshotId", "legacyCleanupAllowed", "error"]);
  if (!isOperationId(raw.operationId) || !isProfileId(raw.profileId) || !isProvider(raw.provider) || !isRevision(raw.expectedRevision) ||
    !["connect", "legacyImport", "disconnect"].includes(String(raw.kind)) || !OPERATION_STATUSES.includes(String(raw.status)) ||
    typeof raw.cleanupPending !== "boolean" || typeof raw.legacyCleanupAllowed !== "boolean" ||
    !(raw.legacySnapshotId === null || isSnapshotId(raw.legacySnapshotId)) || !(raw.error === null || isCredentialError(raw.error))) invalid();
  const connection = raw.connection === null ? null : parseConnectionRef(raw.connection);
  if (connection && (connection.profileId !== raw.profileId || connection.provider !== raw.provider)) invalid();
  return {
    operationId: raw.operationId, profileId: raw.profileId, provider: raw.provider, expectedRevision: raw.expectedRevision,
    kind: raw.kind as CredentialOperationKind, status: raw.status as CredentialOperationStatus,
    connection, cleanupPending: raw.cleanupPending, legacySnapshotId: raw.legacySnapshotId,
    legacyCleanupAllowed: raw.legacyCleanupAllowed, error: raw.error,
  };
}
export function parseProviderState(value: unknown, provider: Provider): ProviderCredentialState {
  const raw = publicRecord(value, ["profileId", "provider", "revision", "status", "activeConnection", "operation", "capabilities"]);
  if (!isProfileId(raw.profileId) || raw.provider !== provider || !isRevision(raw.revision) ||
    (raw.status !== "disconnected" && raw.status !== "connected")) invalid();
  let activeConnection: ConnectionSummary | null = null;
  if (raw.activeConnection !== null) {
    const connection = publicRecord(raw.activeConnection, ["reference", "metadata"]);
    const reference = parseConnectionRef(connection.reference);
    const metadata = publicRecord(connection.metadata, ["configuration", "accountId", "accountName"]);
    if (reference.profileId !== raw.profileId || reference.provider !== provider || reference.revision !== raw.revision ||
      !plain(metadata.accountId, 256) || !plain(metadata.accountName, 1024)) invalid();
    const configuration = parseConfiguration(metadata.configuration);
    if (provider === "linear" ? Object.keys(configuration).length !== 0 : !configuration.siteUrl || !configuration.deployment) invalid();
    activeConnection = { reference, metadata: { configuration, accountId: metadata.accountId, accountName: metadata.accountName } };
  }
  if ((raw.status === "connected") !== (activeConnection !== null)) invalid();
  const operation = raw.operation === null ? null : parseOperationReceipt(raw.operation);
  if (operation && (operation.profileId !== raw.profileId || operation.provider !== provider)) invalid();
  const capabilities = publicRecord(raw.capabilities, ["nativeInput", "legacyImport", "keyring"]);
  if (typeof capabilities.nativeInput !== "boolean" || typeof capabilities.legacyImport !== "boolean" || typeof capabilities.keyring !== "boolean") invalid();
  return {
    profileId: raw.profileId, provider, revision: raw.revision, status: raw.status, activeConnection, operation,
    capabilities: { nativeInput: capabilities.nativeInput, legacyImport: capabilities.legacyImport, keyring: capabilities.keyring },
  };
}
export function sameConnection(left: ConnectionRef, right: ConnectionRef): boolean {
  return left.profileId === right.profileId && left.provider === right.provider && left.connectionId === right.connectionId && left.revision === right.revision;
}
export function operationPending(operation: OperationReceipt | null): boolean {
  return operation !== null && ["awaitingInput", "prepared", "verifying", "storing", "outcomeUnknown"].includes(operation.status);
}
export function credentialMessage(code: CredentialError): string {
  const messages: Record<CredentialError, string> = {
    invalidRequest: "연결 요청이 올바르지 않습니다.", unsupported: "이 Gateway에서는 이 연결 기능을 지원하지 않습니다.",
    locked: "OS 키 저장소가 잠겼거나 접근이 허용되지 않았습니다.", unavailable: "자격 증명 서비스를 사용할 수 없습니다.",
    conflict: "연결이 바뀌었습니다. 현재 상태를 다시 확인하세요.", busy: "이미 진행 중인 연결 작업이 있습니다.",
    cancelled: "연결 작업을 취소했습니다.", outcomeUnknown: "저장 결과를 아직 확인하지 못했습니다. 상태 확인을 눌러 주세요.",
    corrupt: "연결 상태를 읽을 수 없습니다. 기존 자격 증명은 유지했습니다.", tooLarge: "기존 자격 증명이 지원 크기를 넘어서 옮기지 않았습니다.",
    capacityExceeded: "연결 작업 기록이 가득 찼습니다.", notConnected: "연결되어 있지 않습니다.",
    notFound: "연결 작업을 찾을 수 없습니다.", unauthorized: "Gateway의 소유자 또는 프로필을 확인할 수 없습니다.",
  };
  return messages[code];
}

/** Renderer-only facade contract; this is not exposed by Electron preload. */
export interface ProviderConnectionBridge {
  status(): Promise<ProviderCredentialState>;
  connect(configuration?: ConnectionConfiguration): Promise<ProviderCredentialState>;
  disconnect(): Promise<ProviderCredentialState>;
  cancel(): Promise<ProviderCredentialState>;
  retry(): Promise<ProviderCredentialState>;
  legacyStatus(): Promise<LegacyAvailability>;
  migrateLegacy(): Promise<ProviderCredentialState>;
  onChanged(listener: (connectionChanged: boolean) => void): () => void;
}
