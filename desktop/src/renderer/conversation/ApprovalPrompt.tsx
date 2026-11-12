// 열린 스레드의 승인 요청이다 (P3 §6.4). 입력창 위에 붙는다. 다른 창에서 답하면 `serverRequest/resolved` 로 사라진다.
// 파일 변경 승인은 요청에 변경 내용이 없으므로, 같은 itemId 의 `fileChange` 아이템에서 diff 를 찾아 보여 준다 (R1).
// 진행 중인 `fileChange` 아이템은 실시간 `item/started` 로만 온다. `thread/turns/list`, `thread/read` 에는 없다
// (codex 0.159, 2026-11-06 실측). 그래서 요청이 온 뒤에 스레드를 연 창은 변경 내용을 받을 수 없다.
import { useEffect, useMemo } from "react";

import { Icon } from "../design/icons";
import { useDesktopStore } from "../session/desktopStore";
import { MOD_KEY } from "../session/uiStore";
import { APPROVAL_DECISIONS, type ApprovalDecision, type PendingApproval } from "./approvals";
import { diffFileFromChange } from "./diff";
import { DiffFileList } from "./DiffView";
import type { Transcript } from "./transcript";

const APPROVAL_TITLE: Record<PendingApproval["kind"], string> = {
  command: "명령 실행 승인",
  writeStdin: "실행 중인 프로세스에 입력 보내기 승인",
  fileChange: "파일 변경 승인",
};

const DECISION_LABEL: Record<ApprovalDecision, string> = {
  accept: "허용",
  acceptForSession: "이 세션 동안 허용",
  decline: "거부",
};

const DECISION_KEY: Partial<Record<ApprovalDecision, string>> = { accept: `${MOD_KEY}⏎`, decline: `${MOD_KEY}⌫` };

function findFileChange(transcript: Transcript | null, turnId: string, itemId: string) {
  const item = transcript?.turns.find((turn) => turn.id === turnId)?.items.find((candidate) => candidate.id === itemId);
  return item?.type === "fileChange" ? item : null;
}

function ApprovalChanges({ turnId, itemId }: { turnId: string; itemId: string }) {
  const item = useDesktopStore((state) => findFileChange(state.transcript, turnId, itemId));
  const files = useMemo(() => item?.changes.map(diffFileFromChange) ?? [], [item]);
  if (files.length === 0) {
    return (
      <p data-testid="approval-changes-missing">
        이 창은 요청이 온 뒤에 스레드를 열어서 변경 내용을 받지 못했습니다. 요청을 처음 받은 창에서 확인하세요.
      </p>
    );
  }
  return (
    <div className="approval-changes" data-testid="approval-changes">
      <DiffFileList files={files} defaultOpen />
    </div>
  );
}

/** 글을 쓰는 중의 `Ctrl+Backspace` 는 낱말 지우기이다. 그때는 거부 단축키로 보지 않는다. */
function isTyping(target: EventTarget | null): boolean {
  return (target instanceof HTMLTextAreaElement || target instanceof HTMLInputElement) && target.value.length > 0;
}

export function ApprovalPrompt({
  onAnswer,
}: {
  onAnswer(requestId: PendingApproval["requestId"], decision: ApprovalDecision): void;
}) {
  const openThreadId = useDesktopStore((state) => state.openThreadId);
  const pending = useDesktopStore((state) => state.pendingApprovals);
  const approvals = pending.filter((approval) => approval.threadId === openThreadId);
  const first = approvals[0];

  // 단축키는 맨 위의 요청에 답한다.
  useEffect(() => {
    if (!first) return;
    const onKey = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey)) return;
      if (event.key === "Enter") {
        event.preventDefault();
        onAnswer(first.requestId, "accept");
      } else if (event.key === "Backspace" && !isTyping(event.target)) {
        event.preventDefault();
        onAnswer(first.requestId, "decline");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [first, onAnswer]);

  if (approvals.length === 0) {
    return null;
  }
  return (
    <section className="approvals" data-testid="approvals">
      {approvals.map((approval) => (
        <div key={String(approval.requestId)} className="approval-card" role="alertdialog" aria-label={APPROVAL_TITLE[approval.kind]} data-testid="approval">
          <header>
            <Icon name="alert" />
            <strong>{APPROVAL_TITLE[approval.kind]}</strong>
            {approval.cwd && <span className="approval-cwd">{approval.cwd}</span>}
          </header>
          {approval.command && (
            <pre className="approval-command">
              <span className="prompt">$</span> {approval.command}
            </pre>
          )}
          {approval.reason && <p>{approval.reason}</p>}
          {approval.kind === "fileChange" && <ApprovalChanges turnId={approval.turnId} itemId={approval.itemId} />}
          <div className="approval-buttons">
            {APPROVAL_DECISIONS.map((decision) => (
              <button
                key={decision}
                type="button"
                className={decision === "accept" ? "button primary" : "button"}
                onClick={() => onAnswer(approval.requestId, decision)}
                data-testid={`approval-${decision}`}
              >
                {DECISION_LABEL[decision]}
                {approval === first && DECISION_KEY[decision] && <kbd>{DECISION_KEY[decision]}</kbd>}
              </button>
            ))}
          </div>
        </div>
      ))}
    </section>
  );
}
