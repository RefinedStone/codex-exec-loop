import { useState } from "react";
import { Icon } from "../design/icons";
import type { IssueAutomationPolicyAnswer } from "./protocol";

export type IssuePolicyFileScope = "user" | "project";

interface IssuePolicyProps {
  policy: IssueAutomationPolicyAnswer | null;
  loading: boolean;
  error: string | null;
  onOpenFile(workspace: string, scope: IssuePolicyFileScope, expectedPath: string): Promise<void>;
}

const FILE_STATUS = { missing: "파일 없음", readable: "읽기 가능", unavailable: "읽기 불가" } as const;

/** 서버가 확정한 값·출처·해석 문구와 경로만 표시한다. 파일 열기는 좁은 host IPC 의도다. */
export function IssuePolicy({ policy, loading, error, onOpenFile }: IssuePolicyProps) {
  const [opening, setOpening] = useState<IssuePolicyFileScope | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const current = !loading && !error && policy?.status !== "unavailable" ? policy : null;
  const openFile = async (scope: IssuePolicyFileScope, path: string) => {
    setOpening(scope);
    setOpenError(null);
    try {
      if (!current) return;
      await onOpenFile(current.workspace, scope, path);
    } catch (failure) {
      setOpenError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setOpening(null);
    }
  };
  return (
    <section className="issues-policy" aria-label="이슈 자동화 정책" data-testid="issues-policy" aria-busy={loading}>
      <h2>정책</h2>
      <p className="issues-section-description">현재 작업 폴더에 적용되는 유효 정책과 출처입니다.</p>
      {loading && <p className="issues-inline-notice muted" role="status">정책을 확인하고 있습니다.</p>}
      {error && <p className="issues-inline-notice error" role="alert" data-testid="issues-policy-error">{error}</p>}
      {!loading && !error && (!policy || policy.status === "unavailable") && <p className="issues-inline-notice error" role="alert" data-testid="issues-policy-error">정책을 읽을 수 없습니다. 새로 고침으로 다시 확인하세요.</p>}
      {current && <p className="issues-section-description">적용 저장소 <code data-testid="issues-policy-workspace">{current.workspace}</code></p>}
      {current?.status === "policy_blocked" && (
        <section className="issues-policy-blocked" aria-label="정책 확인 필요" data-testid="issues-policy-blocked">
          <h3><Icon name="alert" size={16} />정책 확인 필요</h3>
          <p data-testid="issues-policy-blocked-summary">{current.blocked.summary}</p>
          <dl className="issues-facts">
            <div><dt>설정</dt><dd><code>{current.blocked.field}</code></dd></div>
            <div><dt>원인</dt><dd data-testid="issues-policy-blocked-reason">{current.blocked.reason}</dd></div>
          </dl>
          <p className="issues-next-summary" data-testid="issues-policy-blocked-next">{current.blocked.next_summary}</p>
        </section>
      )}
      {current?.status === "ready" && (
        <dl className="issues-policy-settings">
          {current.settings.map((setting) => (
            <div key={setting.key} className="issues-policy-setting" data-testid="issues-policy-setting" data-setting={setting.key} data-resolution={setting.resolution}>
              <dt>{setting.label}</dt>
              <dd className="issues-policy-value" data-testid="issues-policy-value">{setting.value}</dd>
              <dd className="issues-policy-origins" data-testid="issues-policy-origins">
                <span>출처</span>{setting.origins.map((origin, index) => <span className="issues-origin" data-scope={origin.scope} key={index}>{origin.label}</span>)}
              </dd>
              <dd className="issues-policy-resolution" data-testid="issues-policy-resolution">{setting.resolution_summary}</dd>
            </div>
          ))}
        </dl>
      )}
      {current && (
        <section className="issues-policy-files" aria-label="정책 파일">
          <h3>정책 파일</h3>
          <ul>
            {current.files.map((file) => (
              <li key={file.scope} className="issues-policy-file" data-testid="issues-policy-file" data-scope={file.scope} data-blocked={current.status === "policy_blocked" && current.blocked.file_scopes.includes(file.scope)}>
                <div className="issues-policy-file-heading">
                  <strong>{file.label}</strong><span className="muted">{FILE_STATUS[file.status]}</span>
                  {file.path && file.open_target && <button type="button" className="issues-file-open" disabled={opening !== null} onClick={() => void openFile(file.open_target!, file.path!)} data-testid="issues-policy-file-open"><Icon name="popOut" size={13} />{opening === file.open_target ? "여는 중…" : "파일 열기"}</button>}
                </div>
                <code className="issues-policy-path" data-testid="issues-policy-path">{file.path ?? "파일 위치를 확인할 수 없습니다."}</code>
              </li>
            ))}
          </ul>
          {openError && <p className="issues-inline-notice error" role="alert" data-testid="issues-policy-open-error">{openError}</p>}
        </section>
      )}
    </section>
  );
}
