/** A10: 로컬 Gateway 연결 진단. 원문과 파일 경로는 이 계약에 넣지 않는다. */
export interface DiagnosticVersions {
  gatewayVersion?: string | null;
  codexVersion?: string | null;
}

export interface DiagnosticPreview {
  /** main이 보관하는 정제된 사본의 식별자. 저장할 본문을 renderer에서 다시 받지 않는다. */
  id: string;
  content: string;
  collected: string[];
  excluded: string[];
  missing: string[];
  logStatus: "collected" | "absent" | "empty" | "unreadable";
  offline: boolean;
  truncated: boolean;
}

export type DiagnosticSaveResult =
  | { status: "saved" }
  | { status: "cancelled" }
  | { status: "failed"; reason: "expired" | "busy" | "write_failed" };

export interface DiagnosticsBridge {
  preview(versions: DiagnosticVersions): Promise<DiagnosticPreview>;
  save(previewId: string): Promise<DiagnosticSaveResult>;
  discard(previewId: string): Promise<void>;
}
