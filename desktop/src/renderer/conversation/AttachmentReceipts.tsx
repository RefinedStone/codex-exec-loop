import { useAttachmentReceipts } from "./attachmentReceipt";

export function AttachmentReceipts({ scope }: { readonly scope: string }) {
  const records = useAttachmentReceipts(state => state.records);
  return records.filter(({ receipt }) => receipt.scope === scope).map(({ key, receipt }) =>
    <details key={key} data-testid="attachment-receipt" data-status={receipt.turnId === null ? "unknown" : "acknowledged"}>
      <summary>{receipt.turnId === null
        ? "첨부 전송 결과 불명 (자동 재전송하지 않음)"
        : "첨부 요청 접수만 확인 (모델 읽음 확인 아님)"}</summary>
      <p>요청 ID: <code>{receipt.requestId}</code>{receipt.turnId !== null && <> · 턴 ID: <code>{receipt.turnId}</code></>}</p>
      <ul>{receipt.manifestIds.map((id, index) => <li key={`${index}:${id}`}><code>{id}</code></li>)}</ul>
    </details>);
}
