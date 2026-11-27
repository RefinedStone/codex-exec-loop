import type { GitChangedFile } from "../../shared/bridge";

export function filterChangedFiles(files: readonly GitChangedFile[], query: string): GitChangedFile[] {
  const term = query.trim().toLocaleLowerCase();
  return files.filter((file) => !term || [file.path, file.originalPath ?? ""].some((path) => path.toLocaleLowerCase().includes(term)));
}

