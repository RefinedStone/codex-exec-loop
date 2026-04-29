#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  bash scripts/cleanup_merged_worktrees.sh [options]

Options:
  --apply               Perform the cleanup. Default is dry-run.
  --base <ref>          Base ref that finished branches must be merged into.
                        Default: origin/prerelease
  --remote <name>       Remote to delete merged branch refs from. Default: origin
  --branch <name>       Clean up this finished branch explicitly. May be repeated.
                        Explicit targets only need a clean worktree; they do not
                        require ancestor detection against --base.
  --path <path>         Clean up this finished worktree path explicitly. May be repeated.
  --force-dirty         Allow explicit targets to be removed even when the
                        worktree is dirty. Use only after the lane is finished
                        and any remaining local changes are disposable.
  --keep-branch <name>  Skip this branch. May be repeated.
  --keep-path <path>    Skip this worktree path. May be repeated.
  -h, --help            Show this help text.
EOF
}

require_value() {
  local option="$1"
  local value="${2-}"
  if [[ -z "${value}" ]]; then
    printf 'missing value for %s\n' "${option}" >&2
    exit 1
  fi
}

canonicalize_path() {
  python3 -c 'import os, sys; print(os.path.realpath(sys.argv[1]))' "$1"
}

path_is_kept() {
  local candidate="$1"
  local kept
  for kept in "${keep_paths[@]}"; do
    if [[ "${candidate}" == "${kept}" ]]; then
      return 0
    fi
  done
  return 1
}

branch_is_kept() {
  local candidate="$1"
  local kept
  for kept in "${keep_branches[@]}"; do
    if [[ "${candidate}" == "${kept}" ]]; then
      return 0
    fi
  done
  return 1
}

path_is_targeted() {
  local candidate="$1"
  local target
  for target in "${target_paths[@]+"${target_paths[@]}"}"; do
    if [[ "${candidate}" == "${target}" ]]; then
      return 0
    fi
  done
  return 1
}

branch_is_targeted() {
  local candidate="$1"
  local target
  for target in "${target_branches[@]+"${target_branches[@]}"}"; do
    if [[ "${candidate}" == "${target}" ]]; then
      return 0
    fi
  done
  return 1
}

worktree_is_clean() {
  local path="$1"
  if [[ ! -d "${path}" ]]; then
    return 1
  fi
  [[ -z "$(git -C "${path}" status --porcelain --untracked-files=normal)" ]]
}

branch_is_merged_into_base() {
  local branch_name="$1"
  git merge-base --is-ancestor "${branch_name}" "${base_ref}"
}

remote_branch_exists() {
  local branch_name="$1"
  git show-ref --verify --quiet "refs/remotes/${remote_name}/${branch_name}"
}

apply_cleanup() {
  local path="$1"
  local branch_name="$2"
  local explicit_target="$3"
  local force_remove_path="$4"

  printf 'removing worktree %s (%s)\n' "${path}" "${branch_name}"
  if [[ "${force_remove_path}" == "true" || ("${explicit_target}" == "true" && "${force_dirty}" == "true") ]]; then
    if [[ -d "${path}" ]]; then
      git worktree remove --force "${path}"
    fi
    git branch -D "${branch_name}"
  else
    if [[ -d "${path}" ]]; then
      git worktree remove "${path}"
    fi
    if [[ "${explicit_target}" == "true" ]]; then
      git branch -D "${branch_name}"
    else
      git branch -d "${branch_name}"
    fi
  fi
  if remote_branch_exists "${branch_name}"; then
    git push "${remote_name}" --delete "${branch_name}" || true
  fi
}

report_cleanup() {
  local mode_label="$1"
  local reason="$2"
  local path="$3"
  local branch_name="$4"
  printf '[%s] %s :: %s (%s)\n' "${mode_label}" "${reason}" "${path}" "${branch_name}"
}

apply_mode=false
force_dirty=false
base_ref="origin/prerelease"
remote_name="origin"
target_branches=()
target_paths=()
keep_branches=()
keep_paths=()

while (($# > 0)); do
  case "$1" in
    --apply)
      apply_mode=true
      shift
      ;;
    --force-dirty)
      force_dirty=true
      shift
      ;;
    --base)
      require_value "$1" "${2-}"
      base_ref="$2"
      shift 2
      ;;
    --remote)
      require_value "$1" "${2-}"
      remote_name="$2"
      shift 2
      ;;
    --branch)
      require_value "$1" "${2-}"
      target_branches+=("$2")
      shift 2
      ;;
    --path)
      require_value "$1" "${2-}"
      target_paths+=("$(canonicalize_path "$2")")
      shift 2
      ;;
    --keep-branch)
      require_value "$1" "${2-}"
      keep_branches+=("$2")
      shift 2
      ;;
    --keep-path)
      require_value "$1" "${2-}"
      keep_paths+=("$(canonicalize_path "$2")")
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf 'unknown option: %s\n' "$1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "${repo_root}" ]]; then
  printf 'cleanup_merged_worktrees: not inside a git repository\n' >&2
  exit 1
fi
repo_root="$(canonicalize_path "${repo_root}")"
current_dir="$(canonicalize_path "${PWD}")"

if ! git rev-parse --verify --quiet "${base_ref}" >/dev/null; then
  printf 'cleanup_merged_worktrees: base ref not found: %s\n' "${base_ref}" >&2
  exit 1
fi

keep_paths+=("${repo_root}" "${current_dir}")
base_branch_name="${base_ref#${remote_name}/}"
keep_branches+=("${base_branch_name}")

removed_count=0
dry_run_count=0
skipped_count=0
targeted_mode=false

if ((${#target_branches[@]} > 0 || ${#target_paths[@]} > 0)); then
  targeted_mode=true
fi

current_path=""
current_branch=""

process_entry() {
  local path="$1"
  local branch_ref="$2"
  local branch_name
  local explicitly_targeted=false
  local path_missing=false

  if [[ -z "${path}" || -z "${branch_ref}" ]]; then
    return 0
  fi

  branch_name="${branch_ref#refs/heads/}"
  path="$(canonicalize_path "${path}")"

  if branch_is_targeted "${branch_name}" || path_is_targeted "${path}"; then
    explicitly_targeted=true
  fi
  if [[ ! -d "${path}" ]]; then
    path_missing=true
  fi

  if [[ "${targeted_mode}" == "true" && "${explicitly_targeted}" != "true" ]]; then
    report_cleanup "skip" "not in explicit target set" "${path}" "${branch_name}"
    skipped_count=$((skipped_count + 1))
    return 0
  fi

  if path_is_kept "${path}"; then
    report_cleanup "skip" "kept path" "${path}" "${branch_name}"
    skipped_count=$((skipped_count + 1))
    return 0
  fi

  if branch_is_kept "${branch_name}"; then
    report_cleanup "skip" "kept branch" "${path}" "${branch_name}"
    skipped_count=$((skipped_count + 1))
    return 0
  fi

  if [[ "${explicitly_targeted}" != "true" ]] && ! branch_is_merged_into_base "${branch_name}"; then
    report_cleanup "skip" "branch not merged into ${base_ref}" "${path}" "${branch_name}"
    skipped_count=$((skipped_count + 1))
    return 0
  fi

  if [[ "${path_missing}" == "true" ]]; then
    if [[ "${apply_mode}" == "true" ]]; then
      apply_cleanup "${path}" "${branch_name}" "${explicitly_targeted}" "true"
      removed_count=$((removed_count + 1))
    else
      report_cleanup "dry-run" "missing worktree path eligible for cleanup" "${path}" "${branch_name}"
      dry_run_count=$((dry_run_count + 1))
    fi
    return 0
  fi

  if ! worktree_is_clean "${path}"; then
    if [[ "${explicitly_targeted}" == "true" && "${force_dirty}" == "true" ]]; then
      if [[ "${apply_mode}" == "true" ]]; then
        apply_cleanup "${path}" "${branch_name}" "${explicitly_targeted}" "false"
        removed_count=$((removed_count + 1))
      else
        report_cleanup "dry-run" "explicit dirty target eligible with --force-dirty" "${path}" "${branch_name}"
        dry_run_count=$((dry_run_count + 1))
      fi
      return 0
    fi
    report_cleanup "skip" "dirty worktree" "${path}" "${branch_name}"
    skipped_count=$((skipped_count + 1))
    return 0
  fi

  if [[ "${apply_mode}" == "true" ]]; then
    apply_cleanup "${path}" "${branch_name}" "${explicitly_targeted}" "false"
    removed_count=$((removed_count + 1))
  else
    report_cleanup "dry-run" "eligible for cleanup" "${path}" "${branch_name}"
    dry_run_count=$((dry_run_count + 1))
  fi
}

while IFS= read -r line || [[ -n "${line}" ]]; do
  if [[ -z "${line}" ]]; then
    process_entry "${current_path}" "${current_branch}"
    current_path=""
    current_branch=""
    continue
  fi

  case "${line}" in
    worktree\ *)
      current_path="${line#worktree }"
      ;;
    branch\ refs/heads/*)
      current_branch="${line#branch }"
      ;;
  esac
done < <(git worktree list --porcelain)

process_entry "${current_path}" "${current_branch}"

if [[ "${apply_mode}" == "true" ]]; then
  git worktree prune
  printf 'cleanup complete: removed %d worktree(s), skipped %d\n' "${removed_count}" "${skipped_count}"
else
  printf 'dry-run complete: %d eligible, %d skipped\n' "${dry_run_count}" "${skipped_count}"
fi
