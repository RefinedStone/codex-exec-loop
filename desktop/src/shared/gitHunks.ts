export interface GitHunk {
  id: string;
  text: string;
}

export interface GitHunkDiff {
  hunks: GitHunk[];
  message: string | null;
}

export type GitHunkAction = "stage" | "discard";
