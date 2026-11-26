// 창마다 하나의 검토 사본을 보관한다. 오프라인에서도 동작하고 Gateway/RPC/업로드를 호출하지 않는다.
import { randomUUID } from "node:crypto";
import { renameSync } from "node:fs";
import { open, rm } from "node:fs/promises";
import { dirname, join } from "node:path";

import type { ConnectionState } from "../shared/bridge";
import type { DiagnosticPreview, DiagnosticSaveResult } from "../shared/diagnostics";
import type { DiagnosticLogs } from "./diagnosticLogs";

const VERSION = /^(?:codex-cli )?v?(\d{1,6}\.\d{1,6}\.\d{1,6})(-(?:development|alpha|beta|rc)(?:\.\d{1,6})?)?$/;
const ERROR_CODES = new Set(["ENOENT", "ECONNREFUSED", "ECONNRESET", "EACCES", "EPERM", "ETIMEDOUT", "EPIPE", "ENOTFOUND"]);
const EXCLUDED = ["대화·프롬프트·응답·RPC 본문", "인증값·쿠키·환경 변수·Vault", "파일 경로·사용자명·호스트명", "로그의 메시지·오류 원문·허용하지 않은 필드"];

interface DiagnosticConnection {
  phase: ConnectionState["phase"] | "unobserved";
  endpointKind: "named_pipe" | "unix_socket" | "unavailable";
  errorCode: string | null;
  retryInMs: number | null;
}

export interface DiagnosticsOptions {
  desktopVersion: string;
  electronVersion: string;
  nodeVersion: string;
  platform: string;
  readLogs(): Promise<DiagnosticLogs>;
  now?: () => Date;
}

function version(value: unknown): string | null {
  const matched = typeof value === "string" ? VERSION.exec(value) : null;
  return matched ? matched[1]! + (matched[2] ?? "") : null;
}

class DiagnosticPreviewExpired extends Error {}

export class DiagnosticsService {
  private connection: DiagnosticConnection = { phase: "unobserved", endpointKind: "unavailable", errorCode: null, retryInMs: null };
  private previewValue: DiagnosticPreview | null = null;
  private generation = 0;
  private saving = false;

  constructor(private readonly options: DiagnosticsOptions) {}

  observe(state: ConnectionState): void {
    this.connection = {
      phase: state.phase,
      endpointKind: !state.endpoint ? "unavailable" : state.endpoint.startsWith("\\\\.\\pipe\\") ? "named_pipe" : "unix_socket",
      errorCode: state.detail ? ERROR_CODES.has(state.detail) ? state.detail : /^closed \d{4}$/.test(state.detail) ? state.detail : "detail_omitted" : null,
      retryInMs: Number.isFinite(state.retryInMs) && state.retryInMs! >= 0 && state.retryInMs! <= 30_000 ? state.retryInMs! : null,
    };
  }

  invalidate(): void { this.generation++; this.previewValue = null; }

  discard(id: unknown): void { if (this.previewValue?.id === id) this.invalidate(); }

  async preview(versions: unknown): Promise<DiagnosticPreview> {
    const generation = ++this.generation;
    this.previewValue = null;
    const connection = { ...this.connection };
    const supplied = versions && typeof versions === "object" ? versions as Record<string, unknown> : {};
    const gatewayVersion = version(supplied.gatewayVersion);
    const codexVersion = version(supplied.codexVersion);
    const logs = await this.options.readLogs();
    if (this.generation !== generation) throw new Error("진단 창이 바뀌었습니다. 다시 미리보기를 여세요.");
    const id = randomUUID();
    const missing = [
      ...(!gatewayVersion ? ["Gateway 버전: 이 창에서 관측하지 못함"] : []),
      ...(!codexVersion ? ["Codex 버전: 이 창에서 관측하지 못함"] : []),
      ...(connection.phase === "unobserved" ? ["연결 상태: 아직 관측하지 못함"] : []),
      ...(!connection.errorCode ? ["연결 오류 코드: 없음 또는 관측하지 못함"] : []),
      ...(connection.errorCode === "detail_omitted" ? ["연결 오류 코드: 허용된 코드가 없어 자유형 원문을 제외함"] : []),
      ...(!logs.entries.some(entry => entry.correlationId) ? ["Gateway 요청 상관 ID: 관측하지 못함 (아래 ID는 진단 사본의 ID)"] : []),
      ...(logs.status === "absent" ? ["기존 trace 파일: 없음 또는 기록이 꺼져 있음"] : []),
      ...(logs.status === "empty" ? ["기존 trace 파일: 수집할 Gateway 메타데이터 없음"] : []),
      ...(logs.status === "unreadable" ? ["기존 trace 파일: 읽기 불가 또는 안전한 일반 파일이 아님"] : []),
    ];
    const collected = ["Desktop·Electron·Node 버전과 OS 종류", "로컬 Gateway 연결 상태·오류 코드·재시도 간격", "진단 사본 상관 ID", ...(logs.entries.length ? [`기존 Gateway trace 메타데이터 ${logs.entries.length}건`] : [])];
    const payload = {
      schemaVersion: 1, scenario: "local_gateway_connection", generatedAt: (this.options.now?.() ?? new Date()).toISOString(),
      diagnosticId: id, correlationScope: "local_export_only",
      versions: {
        desktop: version(this.options.desktopVersion), electron: version(this.options.electronVersion), node: version(this.options.nodeVersion),
        platform: ["win32", "darwin", "linux"].includes(this.options.platform) ? this.options.platform : "other",
        gateway: gatewayVersion, codex: codexVersion, gatewayVersionSource: "renderer_last_observed",
      },
      connection, logs, collected, excluded: EXCLUDED, missing,
      scope: "기존 trace의 최근 64 KiB에서 Gateway 메타데이터 최대 40건. 현재 연결과 동일 프로세스라는 보장은 없으며, 원문·회전 파일 전체·다른 시작 폴더는 수집하지 않음.",
    };
    this.previewValue = { id, content: JSON.stringify(payload, null, 2) + "\n", collected, excluded: [...EXCLUDED], missing,
      logStatus: logs.status, offline: connection.phase !== "connected", truncated: logs.truncated };
    return structuredClone(this.previewValue);
  }

  async save(id: unknown, choosePath: () => Promise<string | null>, write: (path: string, content: string, isCurrent: () => boolean) => Promise<void> = writeDiagnosticFile): Promise<DiagnosticSaveResult> {
    const preview = this.previewValue;
    if (!preview || preview.id !== id) return { status: "failed", reason: "expired" };
    if (this.saving) return { status: "failed", reason: "busy" };
    this.saving = true;
    try {
      const path = await choosePath();
      if (!path) return { status: "cancelled" };
      if (this.previewValue !== preview) return { status: "failed", reason: "expired" };
      await write(path, preview.content, () => this.previewValue === preview);
      return { status: "saved" };
    } catch (error) { return { status: "failed", reason: error instanceof DiagnosticPreviewExpired ? "expired" : "write_failed" }; }
    finally { this.saving = false; }
  }
}

/** 검토한 UTF-8 바이트를 같은 디렉터리의 임시 파일에 쓴 뒤 원자적으로 교체한다. */
export async function writeDiagnosticFile(path: string, content: string, isCurrent: () => boolean = () => true): Promise<void> {
  const temporary = join(dirname(path), `.akra-diagnostic-${randomUUID()}.tmp`);
  try {
    const file = await open(temporary, "wx", 0o600);
    try { await file.writeFile(content, "utf8"); await file.sync(); } finally { await file.close(); }
    if (!isCurrent()) throw new DiagnosticPreviewExpired();
    // 검토 사본의 유효성 검사와 교체 사이에는 이벤트 루프로 돌아가지 않는다.
    renameSync(temporary, path);
  } finally { await rm(temporary, { force: true }).catch(() => undefined); }
}
