import type { TaskVerificationRecipe } from "../../shared/kernel-protocol/generated";

export function isTaskVerificationRecipe(value: unknown): value is TaskVerificationRecipe {
  return value === "git_diff_check_v1" || value === "repo_git_grep_v1";
}
