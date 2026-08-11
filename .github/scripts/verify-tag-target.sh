#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: .github/scripts/verify-tag-target.sh \
  --repo <owner/repo> --tag <tag> --expected-commit <full-commit-sha>

Resolve a lightweight or annotated GitHub tag to its final commit and require an exact SHA match.
EOF
}

require_value() {
  local option="$1"
  local value="${2-}"
  if [[ -z "${value}" ]]; then
    echo "verify-tag-target: missing value for ${option}" >&2
    usage >&2
    exit 1
  fi
}

repo=""
tag=""
expected_commit=""

while (($# > 0)); do
  case "$1" in
    --repo)
      require_value "$1" "${2-}"
      repo="$2"
      shift 2
      ;;
    --tag)
      require_value "$1" "${2-}"
      tag="$2"
      shift 2
      ;;
    --expected-commit)
      require_value "$1" "${2-}"
      expected_commit="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "verify-tag-target: unsupported option '$1'" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ ! "${repo}" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
  echo "verify-tag-target: --repo must be a plain GitHub owner/repository identity" >&2
  exit 1
fi
if [[ ! "${tag}" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$ ]]; then
  echo "verify-tag-target: --tag contains unsupported characters" >&2
  exit 1
fi
if ! command -v tr >/dev/null 2>&1; then
  echo "verify-tag-target: tr is required" >&2
  exit 1
fi
expected_commit="$(printf '%s' "${expected_commit}" | LC_ALL=C tr '[:upper:]' '[:lower:]')"
if [[ ! "${expected_commit}" =~ ^[0-9a-f]{40}$ ]]; then
  echo "verify-tag-target: --expected-commit must be a full 40-character commit SHA" >&2
  exit 1
fi
if ! command -v gh >/dev/null 2>&1; then
  echo "verify-tag-target: gh is required" >&2
  exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "verify-tag-target: python3 is required" >&2
  exit 1
fi

read_object_identity() {
  python3 -c '
import json
import re
import sys

try:
    payload = json.load(sys.stdin)
except (json.JSONDecodeError, UnicodeDecodeError) as error:
    raise SystemExit(f"verify-tag-target: GitHub returned invalid JSON: {error}")
if not isinstance(payload, dict):
    raise SystemExit("verify-tag-target: GitHub object response must be an object")
obj = payload.get("object")
if not isinstance(obj, dict):
    raise SystemExit("verify-tag-target: GitHub response has no object identity")
kind = obj.get("type")
sha = obj.get("sha")
if kind not in {"commit", "tag"}:
    raise SystemExit("verify-tag-target: tag must resolve only through commit or tag objects")
if not isinstance(sha, str) or re.fullmatch(r"[0-9a-fA-F]{40}", sha) is None:
    raise SystemExit("verify-tag-target: GitHub returned an invalid object SHA")
print(kind, sha.lower())
'
}

resolve_api_object() {
  local endpoint="$1"
  local response
  if ! response="$(gh api --method GET "${endpoint}")"; then
    echo "verify-tag-target: failed to resolve the GitHub tag object" >&2
    return 1
  fi
  printf '%s' "${response}" | read_object_identity
}

object_identity="$(resolve_api_object "repos/${repo}/git/ref/tags/${tag}")"
visited_tag_objects=$'\n'

for _ in {1..8}; do
  read -r object_type object_sha <<<"${object_identity}"
  if [[ "${object_type}" == "commit" ]]; then
    if [[ "${object_sha}" != "${expected_commit}" ]]; then
      echo "verify-tag-target: tag target does not match the frozen workflow commit" >&2
      exit 1
    fi
    printf 'verified GitHub tag %s at commit %s\n' "${tag}" "${expected_commit}"
    exit 0
  fi
  if [[ "${visited_tag_objects}" == *$'\n'"${object_sha}"$'\n'* ]]; then
    echo "verify-tag-target: annotated tag object cycle detected" >&2
    exit 1
  fi
  visited_tag_objects+="${object_sha}"$'\n'
  object_identity="$(resolve_api_object "repos/${repo}/git/tags/${object_sha}")"
done

echo "verify-tag-target: annotated tag chain exceeds the safety limit" >&2
exit 1
