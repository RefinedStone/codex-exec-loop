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
  worktree_stat_observed: {
    title: "추적 파일 메타데이터 관측",
    description: "Git 인덱스와 현재 파일의 크기·유형·시각을 대조했습니다. 같은 메타데이터도 파일 내용이 같다는 뜻은 아닙니다.",
  },
  worktree_stat_changes: {
    title: "추적 파일 메타데이터 차이",
    description: "인덱스와 다른 크기·유형·시각이나 사라진 경로가 있습니다. 본문 변경이 확정된 것은 아닙니다. 작업 내용을 확인하세요.",
  },
  worktree_stat_unknown: {
    title: "추적 파일 메타데이터 일부 미확인",
    description: "링크·접근 실패·특수 인덱스 항목·조회 한도 등으로 일부 경로를 대조하지 못했습니다. 다른 확인된 보호 사유는 유지됩니다.",
  },
  untracked_files: {
    title: "저장소 규칙 적용 후 미추적 관측",
    description: "저장소의 무시 규칙을 적용해도 인덱스 밖에 남는 관측 파일이 있습니다. 사용자 전역 제외 규칙과 스캔 이후 변경은 확인 범위에 포함되지 않습니다.",
  },
  ignore_rules_observed: {
    title: "저장소 무시 규칙 관측",
    description: "확인한 저장소의 .gitignore와 info/exclude를 적용했습니다. 전역 제외 규칙이나 관측하지 못한 파일까지 확인한 것은 아닙니다.",
  },
  ignore_unknown: {
    title: "저장소 무시 규칙 미확인",
    description: "제외 파일의 범위·접근·변경·링크 또는 조회 한도 때문에 규칙을 확인하지 못했습니다. 인덱스 밖 경로에는 무시된 파일도 포함될 수 있습니다.",
  },
  git_operation_locked: {
    title: "Git 작업 잠금 관측",
    description: "Git 작업 잠금 경로가 존재합니다. 잠금 소유자나 작업 종료 여부를 추정하지 말고 진행 중인 작업을 확인하세요.",
  },
  git_lock_unknown: {
    title: "Git 작업 잠금 일부 미확인",
    description: "일부 잠금 경로를 확인하지 못했습니다. 잠금이 없거나 작업이 멈췄다고 판단할 수 없습니다.",
  },
  status_unknown: {
    title: "작업 트리 상태 미확인",
    description: "파일 내용의 변경 여부는 비교하지 않았습니다. 관측한 파일 메타데이터와 저장소 규칙만으로 내용 동일성이나 삭제 가능 여부를 판단할 수 없으며, 미확인 범위는 관측 근거를 확인하세요.",
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
