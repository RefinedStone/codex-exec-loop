#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "${repo_root}" ]]; then
  echo "gh-refinedstone: not inside a git repository" >&2
  exit 1
fi

credential_file="${repo_root}/.git/refinedstone-credentials"
if [[ ! -f "${credential_file}" ]]; then
  echo "gh-refinedstone: missing ${credential_file}" >&2
  exit 1
fi

credential_line="$(head -n 1 "${credential_file}")"
token="${credential_line#https://RefinedStone:}"
token="${token%@github.com/*}"

if [[ -z "${token}" || "${token}" == "${credential_line}" ]]; then
  echo "gh-refinedstone: failed to parse RefinedStone token from ${credential_file}" >&2
  exit 1
fi

GH_TOKEN="${token}" GH_HOST=github.com exec gh "$@"
