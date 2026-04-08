#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "${repo_root}" ]]; then
  echo "gh-refinedstone: not inside a git repository" >&2
  exit 1
fi

find_credential_file() {
  local repo_credential_file
  repo_credential_file="${repo_root}/.git/refinedstone-credentials"
  if [[ -f "${repo_credential_file}" ]]; then
    printf '%s\n' "${repo_credential_file}"
    return 0
  fi

  while IFS= read -r windows_credential_file; do
    if grep -q '^https://RefinedStone:.*@github\.com' "${windows_credential_file}"; then
      printf '%s\n' "${windows_credential_file}"
      return 0
    fi
  done < <(find /mnt/c/Users -maxdepth 2 -type f -name '.git-credentials' 2>/dev/null | sort)

  echo "gh-refinedstone: missing ${repo_credential_file} and no Windows RefinedStone credential was found under /mnt/c/Users/*/.git-credentials" >&2
  exit 1
}

read_credential_line() {
  local credential_file
  credential_file="$1"

  if [[ "${credential_file}" == *.git-credentials ]]; then
    grep -m1 '^https://RefinedStone:.*@github\.com' "${credential_file}" || true
    return 0
  fi

  head -n 1 "${credential_file}"
}

parse_token() {
  local credential_line
  credential_line="$1"

  local token
  token="${credential_line#https://RefinedStone:}"
  token="${token%@github.com/*}"
  token="${token%@github.com}"

  if [[ -z "${token}" || "${token}" == "${credential_line}" ]]; then
    return 1
  fi

  printf '%s\n' "${token}"
}

parse_repo_full_name() {
  local origin_url
  origin_url="$(git remote get-url origin)"

  case "${origin_url}" in
    git@github.com:*)
      origin_url="${origin_url#git@github.com:}"
      ;;
    https://github.com/*)
      origin_url="${origin_url#https://github.com/}"
      ;;
    *)
      echo "gh-refinedstone: unsupported origin URL ${origin_url}" >&2
      exit 1
      ;;
  esac

  origin_url="${origin_url%.git}"
  printf '%s\n' "${origin_url}"
}

json_escape() {
  local value
  value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//$'\n'/\\n}"
  value="${value//$'\r'/\\r}"
  value="${value//$'\t'/\\t}"
  printf '%s' "${value}"
}

extract_json_field() {
  local body
  local field_name
  body="$1"
  field_name="$2"
  printf '%s' "${body}" | tr -d '\n' | sed -n "s/.*\"${field_name}\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" | head -n 1
}

api_request() {
  local method
  local endpoint
  local payload
  local response_file
  local status_code

  method="$1"
  endpoint="$2"
  payload="${3-}"
  response_file="$(mktemp)"

  if [[ -n "${payload}" ]]; then
    status_code="$(
      curl -sS -o "${response_file}" -w '%{http_code}' \
        -X "${method}" \
        -H "Accept: application/vnd.github+json" \
        -H "Authorization: Bearer ${token}" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        -d "${payload}" \
        "https://api.github.com${endpoint}"
    )"
  else
    status_code="$(
      curl -sS -o "${response_file}" -w '%{http_code}' \
        -X "${method}" \
        -H "Accept: application/vnd.github+json" \
        -H "Authorization: Bearer ${token}" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        "https://api.github.com${endpoint}"
    )"
  fi

  if [[ "${status_code}" != 2* ]]; then
    cat "${response_file}" >&2
    rm -f "${response_file}"
    return 1
  fi

  cat "${response_file}"
  rm -f "${response_file}"
}

create_pr_with_api() {
  local repo_full_name
  local base_branch
  local head_branch
  local title
  local body
  local draft

  repo_full_name="$(parse_repo_full_name)"
  base_branch=""
  head_branch=""
  title=""
  body=""
  draft="false"

  while (($# > 0)); do
    case "$1" in
      --base)
        base_branch="${2-}"
        shift 2
        ;;
      --head)
        head_branch="${2-}"
        shift 2
        ;;
      --title)
        title="${2-}"
        shift 2
        ;;
      --body)
        body="${2-}"
        shift 2
        ;;
      --body-file)
        body="$(cat "${2-}")"
        shift 2
        ;;
      --draft)
        draft="true"
        shift
        ;;
      *)
        echo "gh-refinedstone: unsupported pr create option ${1} without gh installed" >&2
        exit 1
        ;;
    esac
  done

  if [[ -z "${base_branch}" || -z "${head_branch}" || -z "${title}" ]]; then
    echo "gh-refinedstone: pr create requires --base, --head, and --title" >&2
    exit 1
  fi

  local payload
  payload=$(
    printf '{"title":"%s","head":"%s","base":"%s","body":"%s","draft":%s}' \
      "$(json_escape "${title}")" \
      "$(json_escape "${head_branch}")" \
      "$(json_escape "${base_branch}")" \
      "$(json_escape "${body}")" \
      "${draft}"
  )

  local response_body
  if response_body="$(api_request POST "/repos/${repo_full_name}/pulls" "${payload}" 2>/tmp/gh-refinedstone-create-pr-error.$$)"; then
    :
  else
    if grep -q 'A pull request already exists' /tmp/gh-refinedstone-create-pr-error.$$; then
      response_body="$(api_request GET "/repos/${repo_full_name}/pulls?state=open&head=RefinedStone:${head_branch}&base=${base_branch}")"
      local existing_url
      existing_url="$(extract_json_field "${response_body}" "html_url")"
      if [[ -n "${existing_url}" ]]; then
        printf '%s\n' "${existing_url}"
        rm -f /tmp/gh-refinedstone-create-pr-error.$$
        return 0
      fi
    fi
    cat /tmp/gh-refinedstone-create-pr-error.$$ >&2
    rm -f /tmp/gh-refinedstone-create-pr-error.$$
    exit 1
  fi
  rm -f /tmp/gh-refinedstone-create-pr-error.$$

  local pr_url
  pr_url="$(extract_json_field "${response_body}" "html_url")"
  if [[ -n "${pr_url}" ]]; then
    printf '%s\n' "${pr_url}"
    return 0
  fi

  printf '%s\n' "${response_body}"
}

credential_file="$(find_credential_file)"
credential_line="$(read_credential_line "${credential_file}")"
token="$(parse_token "${credential_line}" || true)"

if [[ -z "${token}" ]]; then
  echo "gh-refinedstone: failed to parse RefinedStone token from ${credential_file}" >&2
  exit 1
fi

if command -v gh >/dev/null 2>&1; then
  GH_TOKEN="${token}" GH_HOST=github.com exec gh "$@"
fi

if [[ "${1-}" == "pr" && "${2-}" == "create" ]]; then
  shift 2
  create_pr_with_api "$@"
  exit 0
fi

echo "gh-refinedstone: gh is not installed and direct fallback currently supports only 'pr create'" >&2
exit 1
