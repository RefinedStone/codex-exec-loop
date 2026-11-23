import type { StorageNode } from "./storageTypes";

const descriptions: Record<string, { title: string; description: string }> = {
  akra_known_repository: {
    title: "기존 Akra 커널의 저장소 관측",
    description: "기존 커널의 루트와 이 디렉터리가 일치합니다. 현재 사용 상태를 따로 확인하세요.",
  },
  runtime_candidates_partial: {
    title: "기존 커널 목록 일부 미확인",
    description: "조회 한도 등으로 기존 커널의 저장소 목록을 모두 확인하지 못했습니다. 목록에 없더라도 사용 중일 수 있습니다.",
  },
  untracked_observed: {
    title: "Git 인덱스에 없는 경로 관측",
    description: "발견한 파일 경로 일부가 Git 인덱스에 없습니다. ignore 규칙을 읽거나 적용하지 않아 Git의 미추적 파일 판정과 다를 수 있습니다.",
  },
  index_conflicts: {
    title: "Git 인덱스 충돌 관측",
    description: "Git 인덱스에 해결되지 않은 충돌 항목이 있습니다. 작업 내용을 확인하세요.",
  },
  index_paths_unknown: {
    title: "Git 인덱스 경로 미확인",
    description: "조회 실패·한도 초과·지원하지 않는 경로 등의 이유로 인덱스 경로를 확인하지 못했습니다. 작업 트리 상태를 판단할 수 없습니다.",
  },
  status_unknown: {
    title: "작업 트리 상태 미확인",
    description: "파일 본문과 ignore 규칙을 읽지 않았습니다. 관측한 인덱스·경로 정보만으로 변경 없음이나 삭제 가능 여부를 판단할 수 없습니다.",
  },
};

export function StorageProtection({ reason }: { reason: StorageNode["protection"][number] }) {
  const known = descriptions[reason.code];
  const title = "검토 필요 · " + (known?.title ?? "추가 보호 사유");
  return <div role="note" aria-label={title} className="storage-protection">
    <strong>{title}</strong>
    <p>{known?.description ?? reason.detail}</p>
    {known && reason.detail && <details>
      <summary>관측 근거</summary>
      <p>{reason.detail}</p>
      <p className="muted">관측 시점: {String(reason.observed_at)}</p>
    </details>}
  </div>;
}
