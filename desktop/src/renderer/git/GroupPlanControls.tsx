import { useLayoutEffect, useState } from "react";
import { groupDestination, type GitOperation, type GroupDestination } from "./gitOperations";

export const GROUP_DROP = "akra:group-base-drop";
export const DRAG_BRANCH = "application/x-akra-current-branch";
const LOCAL_ONLY = "로컬 변경만 계획합니다. 원격 push 단계는 제외합니다.";

type Props = {
  cwd: string;
  branch: string;
  base: string;
  picks: readonly string[];
  disabled: boolean;
  plan(operation: GitOperation, branch: string): void;
};

export function GroupPlanControls({ cwd, branch, base, picks, disabled, plan }: Props) {
  const [publish, setPublish] = useState<GroupDestination>();
  const [note, setNote] = useState(LOCAL_ONLY);
  const [checking, setChecking] = useState(false);
  const unavailable = disabled || checking || picks.length > 32;

  function preview() {
    if (!unavailable) plan({ kind: "realign", base, commits: picks, ...(publish ? { publish } : {}) }, branch);
  }

  useLayoutEffect(() => {
    window.addEventListener(GROUP_DROP, preview);
    return () => window.removeEventListener(GROUP_DROP, preview);
  });

  async function inspect() {
    setChecking(true);
    setPublish(undefined);
    try {
      const raw = await window.akra.gitOperator(cwd, "group-preview", branch);
      const target = groupDestination(JSON.parse(raw));
      if (!target || target.branch !== branch) throw new Error("목적지를 확인할 수 없습니다.");
      setPublish(target);
      setNote("추가 확인 후에만 원격에 반영합니다: " + target.url + " " + target.branch + " @ " + target.sha);
    } catch (error) {
      setNote("원격 단계 제외: " + (error instanceof Error ? error.message : String(error)));
    } finally {
      setChecking(false);
    }
  }

  return (
    <div>
      <button type="button" disabled={unavailable} onClick={preview}>이 커밋을 새 base로 계획</button>
      <button type="button" disabled={disabled || checking} onClick={() => void inspect()}>원격 단계 검토</button>
      {publish && <button type="button" onClick={() => { setPublish(undefined); setNote(LOCAL_ONLY); }}>로컬만</button>}
      <small role="status">{note}</small>
    </div>
  );
}
