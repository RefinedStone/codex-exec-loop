// `git status --porcelain=v1` 의 두 글자 상태를 읽는다. main 과 renderer 가 함께 쓴다.
// 첫 글자는 스테이지한 변경, 둘째 글자는 아직 스테이지하지 않은 변경이다.

/** 병합 충돌이 남은 파일이다 (`UU`, `AA`, `DD`, `AU`, `UD` 등). */
export function isConflictStatus(status: string): boolean {
  return status.includes("U") || status === "AA" || status === "DD";
}

export function stageState(file: { status: string; untracked: boolean }): { staged: boolean; unstaged: boolean } {
  return { staged: !file.untracked && file.status[0] !== " ", unstaged: file.untracked || file.status[1] !== " " };
}
