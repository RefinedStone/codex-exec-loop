// 서버가 만든 목록·상세·관찰 보고·정책 표시값을 읽는 이슈 패널이다 (#2539, #2542).
import { useState } from "react";
import { Icon } from "../design/icons";
import { useDesktopStore } from "../session/desktopStore";
import { folderName } from "../sidebar/threadStatus";
import { IssueDetail } from "./IssueDetail";
import { IssueAutonomyControl } from "./IssueAutonomyControl";
import type { IssueAutonomyLevel, IssueCommandOutcome, IssueInterventionName } from "./issueInteractions";
import { IssueObserveReport } from "./IssueObserveReport";
import { IssuePolicy, type IssuePolicyFileScope } from "./IssuePolicy";
import type { IssueAutomationModeRecord, IssueCollectionFailureState, IssueEventReference, IssueListItem, IssueSourceFailureState } from "./protocol";

const SOURCE_FAILURE_TEXT: Record<IssueSourceFailureState, string> = {
  invalid_request: "이슈 조회 요청을 처리할 수 없습니다.",
  unsupported_provider: "이 이슈 공급자를 아직 지원하지 않습니다.",
  invalid_cursor: "수집 위치를 다시 확인해야 합니다.",
  provider_unavailable: "이슈 공급자에 연결하지 못했습니다.",
  malformed_response: "이슈 공급자의 응답을 확인할 수 없습니다.",
  identity_mismatch: "연결된 저장소의 신원을 확인할 수 없습니다.",
  metadata_incomplete: "이슈의 메타데이터를 모두 확인하지 못했습니다.",
  feedback_unconfirmed: "이슈의 진행 표시가 반영됐는지 확인이 필요합니다.",
};

/** 닫힌 수집 오류 코드의 표시 문구다. 목록의 상태나 재시도 여부를 판단하지 않는다. */
function collectionFailureText(failure: IssueCollectionFailureState): string {
  switch (failure.kind) {
    case "origin_unavailable":
      return "연결할 GitHub 저장소를 확인하지 못했습니다.";
    case "policy_unavailable":
      return "이슈 공개 정책을 읽지 못했습니다.";
    case "source":
      return SOURCE_FAILURE_TEXT[failure.reason];
  }
}

function successTime(value: number | null | undefined): string {
  if (value === null || value === undefined) return "아직 없음";
  const time = new Date(value);
  if (Number.isNaN(time.valueOf())) return "시각을 확인할 수 없습니다.";
  return new Intl.DateTimeFormat("ko-KR", { dateStyle: "medium", timeStyle: "short" }).format(time);
}

interface IssuePanelProps {
  onRefresh(): Promise<void>;
  onSelectIssue(issue: IssueEventReference): Promise<void>;
  onClearIssueSelection(): void;
  onRefreshDetail(): Promise<void>;
  onRefreshPolicy(): Promise<void>;
  onOpenPolicyFile(workspace: string, scope: IssuePolicyFileScope, expectedPath: string): Promise<void>;
  onNextPage(): Promise<void>;
  onPreviousPage(): Promise<void>;
  onSetMode(mode: IssueAutomationModeRecord): Promise<void>;
  onSync(): Promise<void>;
  onSetAutonomy(level: IssueAutonomyLevel): Promise<void>;
  onAnswerDecision(decisionId: string, option: string, note: string): Promise<void>;
  onIntervene(action: IssueInterventionName, message?: string, confirm?: boolean): Promise<IssueCommandOutcome>;
  onOpenConversation(): Promise<void>;
  onOpenUrl(url: string): void;
}

function IssueRow({ item, onSelectIssue, onOpenUrl }: { item: IssueListItem; onSelectIssue: IssuePanelProps["onSelectIssue"]; onOpenUrl: IssuePanelProps["onOpenUrl"] }) {
  const shown = item.presentation;
  const title = shown.title ?? "제목을 확인할 수 없습니다.";
  return (
    <li className="issues-row" data-testid="issues-row">
      <div className="issues-row-heading">
        <span className="issues-number" data-testid="issues-number">{shown.number == null ? "번호 미확인" : `#${shown.number}`}</span>
        <span className="issues-status" data-testid="issues-status" data-status={shown.status.code}>{shown.status.label}</span>
        {shown.url && <button type="button" className="icon-button issues-external" title="브라우저에서 열기" aria-label={title + ", 브라우저에서 열기"} onClick={() => onOpenUrl(shown.url!)} data-testid="issues-external"><Icon name="popOut" size={14} /></button>}
      </div>
      <button type="button" className="issues-title" onClick={() => void onSelectIssue(item.issue)} aria-label={title + ", 상세 보기"} data-testid="issues-title">{title}</button>
      <p className="issues-reason" title={shown.reason.summary} data-testid="issues-reason">{shown.reason.summary}</p>
    </li>
  );
}

export function IssuePanel({ onRefresh, onSelectIssue, onClearIssueSelection, onRefreshDetail, onRefreshPolicy, onOpenPolicyFile, onNextPage, onPreviousPage, onSetMode, onSync, onSetAutonomy, onAnswerDecision, onIntervene, onOpenConversation, onOpenUrl }: IssuePanelProps) {
  const [view, setView] = useState<"list" | "report" | "policy">("list");
  const kernel = useDesktopStore((state) => state.kernel);
  const issues = kernel.issues;
  const automation = issues.automation?.status === "ready" ? issues.automation : null;
  // 무효화된 페이지의 과거 제목을 새 관찰 결과처럼 보여 주지 않는다.
  const page = !issues.stale && issues.page?.status === "ready" ? issues.page : null;
  const selected = view === "list" ? issues.selectedIssue : null;
  const attached = kernel.phase === "attached";
  const refresh = view === "policy" ? onRefreshPolicy : selected ? onRefreshDetail : onRefresh;
  const refreshing = view === "policy" ? issues.policyLoading : selected ? issues.detailLoading : issues.loading;
  const refreshLabel = view === "policy" ? "정책 새로 고침" : selected ? "이슈 상세 새로 고침" : view === "report" ? "관찰 보고 새로 고침" : "이슈 목록 새로 고침";
  const busy = issues.commandPending !== null;
  const connectedFolder = automation?.binding?.project_scope.workspace_dir ?? kernel.workspace;
  const changeMode = (value: string) => {
    if (value === "off" || value === "observe") void onSetMode(value);
  };

  return (
    <section className="issues-panel" aria-label="저장소 이슈" data-testid="issues-panel">
      <header className="issues-header">
        <div className="issues-heading">
          <Icon name="eye" size={18} />
          <strong>이슈</strong>
          {kernel.workspace && <span className="issues-workspace" title={kernel.workspace}>{folderName(kernel.workspace)}</span>}
          <button type="button" className="icon-button" title={refreshLabel} aria-label={refreshLabel} disabled={!attached || refreshing} onClick={() => void refresh()} data-testid="issues-refresh">
            <Icon name="refresh" size={15} />
          </button>
        </div>
        {kernel.workspace && (
          <p className="issues-binding" data-testid="issues-binding">
            <Icon name="branch" size={13} />
            {automation?.binding ? (
              <span title={automation.binding.project_scope.workspace_dir}>
                {automation.binding.provider === "github" ? "GitHub" : "Jira"} · {connectedFolder ? folderName(connectedFolder) : automation.binding.server_id}
              </span>
            ) : <span>저장소 연결 확인 전</span>}
          </p>
        )}
      </header>

      {kernel.workspace === null ? (
        <div className="issues-empty" data-testid="issues-workspace-empty">
          <Icon name="folder" size={24} />
          <strong>저장소의 이슈를 확인하세요</strong>
          <p>대화를 열면 그 작업 폴더에 연결된 이슈를 보여 줍니다.</p>
        </div>
      ) : (
        <>
          <nav className="issues-tabs" aria-label="이슈 보기">
            <button type="button" aria-pressed={view === "list"} onClick={() => { setView("list"); onClearIssueSelection(); }} data-testid="issues-list-tab">목록</button>
            <button type="button" aria-pressed={view === "report"} onClick={() => setView("report")} data-testid="issues-report-tab">관찰 보고</button>
            <button type="button" aria-pressed={view === "policy"} onClick={() => setView("policy")} data-testid="issues-policy-tab">정책</button>
          </nav>
          {view === "list" && !selected && (
            <>
              <div className="issues-controls">
                <label className="issues-mode-label">
                  <span>수집 모드</span>
                  <select aria-label="이슈 수집 모드" value={automation?.mode ?? ""} disabled={!attached || !automation || busy} onChange={(event) => changeMode(event.target.value)} data-testid="issues-mode">
                    {!automation && <option value="" disabled>확인 중</option>}
                    <option value="off">끄기</option>
                    <option value="observe">관찰</option>
                  </select>
                </label>
                <button type="button" className="issues-sync" disabled={!attached || !automation || automation.mode !== "observe" || busy || automation.collecting === true} title={automation?.mode === "off" ? "관찰 모드에서 동기화할 수 있습니다." : undefined} onClick={() => void onSync()} data-testid="issues-sync">
                  <Icon name="refresh" size={14} />
                  {issues.commandPending === "sync" ? "요청하는 중…" : automation?.collecting ? "수집 중…" : "지금 동기화"}
                </button>
              </div>

              <IssueAutonomyControl autonomy={automation?.autonomy} connected={attached} busy={busy} onChange={onSetAutonomy} />
              <div className="issues-collection-status" aria-live="polite">
                <span>{issues.commandPending === "mode" ? "모드를 저장하는 중…" : automation ? (automation.collecting ? "이슈를 수집하고 있습니다." : automation.mode === "off" ? "자동 수집이 꺼져 있습니다." : "다음 수집을 기다리고 있습니다.") : "수집 상태를 확인하고 있습니다."}</span>
                <span className="issues-last-success"><Icon name="clock" size={12} />마지막 성공 <span data-testid="issues-last-success">{successTime(automation?.last_success_at_ms)}</span></span>
              </div>

            </>
          )}

          <div className="issues-scroll" aria-busy={view === "policy" ? issues.policyLoading : selected ? issues.detailLoading || issues.detailStale : issues.loading || issues.stale}>
            {!attached && (
              <p className={kernel.phase === "failed" ? "issues-notice error" : "issues-notice muted"} role={kernel.phase === "failed" ? "alert" : "status"} data-testid="issues-connection">
                {kernel.message ?? "커널에 연결하는 중…"}
              </p>
            )}
            {attached && kernel.message && <p className="issues-notice warning">{kernel.message}</p>}
            {issues.error && <p className="issues-notice error" role="alert" data-testid="issues-error">{issues.error}</p>}
            {issues.notice && <p className="issues-notice" role="status" data-testid="issues-command-notice">{issues.notice}</p>}
            {automation?.last_failure && <p className="issues-notice error" role="alert" data-testid="issues-error">수집 오류: {collectionFailureText(automation.last_failure)}</p>}
            {issues.automation?.status === "unavailable" && <p className="issues-notice error" role="alert" data-testid="issues-error">수집 설정을 읽을 수 없습니다. 새로 고침으로 다시 확인하세요.</p>}
            {issues.page?.status === "unavailable" && <p className="issues-notice error" role="alert" data-testid="issues-error">이슈 목록을 읽을 수 없습니다. 새로 고침으로 다시 확인하세요.</p>}
            {(issues.loading || issues.stale) && <p className="issues-notice muted" role="status">이슈 목록을 확인하는 중…</p>}
            {view === "list" && !selected && page && page.items.length === 0 && (
              <div className="issues-empty">
                <Icon name="eye" size={24} />
                <strong>아직 수집한 이슈가 없습니다</strong>
                <p>관찰 모드에서 동기화하면 연결된 저장소의 이슈를 확인할 수 있습니다.</p>
              </div>
            )}
            {view === "list" && !selected && page?.groups.map((group) => (
              <section className="issues-group" key={group.id} aria-label={group.label} data-testid="issues-group">
                <h3 data-testid="issues-group-title">{group.label}<span>{group.item_indices.length}</span></h3>
                <ul>
                  {group.item_indices.map((index) => {
                    const item = page.items[index];
                    return item ? <IssueRow key={JSON.stringify(item.issue)} item={item} onSelectIssue={onSelectIssue} onOpenUrl={onOpenUrl} /> : <li key={index} className="issues-notice error" role="alert">목록 항목을 확인할 수 없습니다. 새로 고침으로 다시 확인하세요.</li>;
                  })}
                </ul>
              </section>
            ))}
            {selected && <IssueDetail selectedIssue={selected} detail={issues.detail} loading={issues.detailLoading} stale={issues.detailStale} error={issues.detailError} busy={busy || !attached} onAnswer={onAnswerDecision} onIntervene={onIntervene} onOpenConversation={onOpenConversation} onBack={onClearIssueSelection} onOpenUrl={onOpenUrl} />}
            {view === "report" && <IssueObserveReport report={page?.report} />}
            {view === "policy" && <IssuePolicy key={kernel.workspace} policy={issues.policy} loading={issues.policyLoading} error={issues.policyError} onOpenFile={onOpenPolicyFile} />}
          </div>

          {view === "list" && !selected && (
            <footer className="issues-pagination">
              <span data-testid="issues-page-count">{issues.pageNumber}페이지{page ? ` · ${page.items.length}개` : ""}</span>
              <div>
                <button type="button" disabled={!attached || !issues.canGoBack || issues.loading || issues.stale} onClick={() => void onPreviousPage()} data-testid="issues-previous-page"><Icon name="chevronLeft" size={13} />이전</button>
                <button type="button" disabled={!attached || !page?.next_after || issues.loading || issues.stale} onClick={() => void onNextPage()} data-testid="issues-next-page">다음<Icon name="chevronRight" size={13} /></button>
              </div>
            </footer>
          )}
        </>
      )}
    </section>
  );
}
