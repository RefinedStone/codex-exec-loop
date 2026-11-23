import { lstat } from "node:fs/promises";
import { isAbsolute } from "node:path";
export interface StorageRevealRequest { path: string; host: "local"; }
/** 커널에서 node를 해석한 로컬 경로를 탐색기에서 선택한다. 파일 실행 API를 사용하지 않는다. */
export async function revealStoragePath(input: unknown, show: (path: string) => void): Promise<void> {
  if (!input || typeof input !== "object") throw new Error("선택한 경로가 없습니다.");
  const value = input as Partial<StorageRevealRequest>;
  if (value.host !== "local" || typeof value.path !== "string" || !isAbsolute(value.path) || value.path.includes("\0") || /^[a-z][a-z0-9+.-]*:\/\//i.test(value.path)) throw new Error("확인된 로컬 경로만 열 수 있습니다.");
  await lstat(value.path);
  show(value.path);
}
