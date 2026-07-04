#!/usr/bin/env bash
set -euo pipefail

script_name="gh-akra"

usage_error() {
  echo "${script_name}: $*" >&2
  exit 1
}

require_value() {
  local option="$1"
  local value="${2-}"
  if [[ -z "${value}" ]]; then
    usage_error "missing value for ${option}"
  fi
}

read_option_file() {
  local option="$1"
  local path="$2"

  require_value "${option}" "${path}"
  if [[ ! -f "${path}" ]]; then
    usage_error "file for ${option} not found: ${path}"
  fi
  cat "${path}"
}

pr_create_uses_title_file() {
  while (($# > 0)); do
    case "$1" in
      --title-file)
        return 0
        ;;
      --title-file=*)
        return 0
        ;;
    esac
    shift
  done
  return 1
}

desired_login="${AKRA_GITHUB_LOGIN:-}"
while (($# > 0)); do
  case "$1" in
    --github-login)
      require_value "$1" "${2-}"
      desired_login="$2"
      shift 2
      ;;
    --github-login=*)
      require_value "--github-login" "${1#--github-login=}"
      desired_login="${1#--github-login=}"
      shift
      ;;
    *)
      break
      ;;
  esac
done

auth_status_only=false
if [[ "${1-}:${2-}" == "auth:status" ]]; then
  auth_status_only=true
fi

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "${repo_root}" ]]; then
  if [[ "${auth_status_only}" == "true" ]]; then
    repo_root="${PWD}"
    git_dir=""
    git_common_dir=""
  else
    usage_error "not inside a git repository"
  fi
else
  git_dir="$(git rev-parse --path-format=absolute --git-dir 2>/dev/null || true)"
  if [[ -z "${git_dir}" ]]; then
    usage_error "failed to resolve git dir"
  fi
  git_common_dir="$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
fi

if [[ -z "${desired_login}" && -n "${git_dir:-}" ]]; then
  desired_login="$(git -C "${repo_root}" config --get akra.githubLogin 2>/dev/null || true)"
fi


parse_repo_full_name() {
  local origin_url
  origin_url="$(git -C "${repo_root}" remote get-url origin)"

  case "${origin_url}" in
    git@github.com:*)
      origin_url="${origin_url#git@github.com:}"
      ;;
    ssh://git@github.com/*)
      origin_url="${origin_url#ssh://git@github.com/}"
      ;;
    https://*github.com/*)
      origin_url="${origin_url#https://}"
      origin_url="${origin_url#*@github.com/}"
      origin_url="${origin_url#github.com/}"
      ;;
    *)
      usage_error "unsupported origin URL ${origin_url}"
      ;;
  esac

  origin_url="${origin_url%.git}"
  printf '%s\n' "${origin_url}"
}

repo_full_name=""


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

curl_config_escape() {
  local value
  value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//$'\n'/\\n}"
  value="${value//$'\r'/\\r}"
  printf '%s' "${value}"
}

run_github_curl() {
  local method
  local endpoint
  local payload
  local response_file
  local config

  method="$1"
  endpoint="$2"
  payload="${3-}"
  response_file="$4"

  config=$(
    printf 'silent\nshow-error\nlocation\noutput = "%s"\nwrite-out = "%%{http_code}"\nconnect-timeout = 10\nmax-time = 30\nrequest = "%s"\nheader = "Accept: application/vnd.github+json"\nheader = "Authorization: Bearer %s"\nheader = "User-Agent: gh-akra.sh"\nheader = "X-GitHub-Api-Version: 2022-11-28"\nurl = "https://api.github.com%s"\n' \
      "$(curl_config_escape "${response_file}")" \
      "$(curl_config_escape "${method}")" \
      "$(curl_config_escape "${token}")" \
      "$(curl_config_escape "${endpoint}")"
    if [[ -n "${payload}" ]]; then
      printf 'data = "%s"\n' "$(curl_config_escape "${payload}")"
    fi
  )
  printf '%s' "${config}" | curl --config -
}


json_string_field() {
  local body
  local field_name
  body="$1"
  field_name="$2"

  JSON_BODY="${body}" python3 -c '
import json
import os
import sys

field_name = sys.argv[1]
data = json.loads(os.environ["JSON_BODY"])
if isinstance(data, list):
    data = data[0] if data else None
if isinstance(data, dict):
    value = data.get(field_name)
    if isinstance(value, str):
        print(value)
' "${field_name}"
}

json_path_string_field() {
  local body
  local field_path
  body="$1"
  field_path="$2"

  JSON_BODY="${body}" python3 -c '
import json
import os
import sys

value = json.loads(os.environ["JSON_BODY"])
for part in sys.argv[1].split("."):
    if not isinstance(value, dict):
        value = None
        break
    value = value.get(part)
if isinstance(value, str):
    print(value)
' "${field_path}"
}

url_encode() {
  local value
  value="$1"

  printf '%s' "${value}" | python3 -c '
import sys
import urllib.parse

print(urllib.parse.quote(sys.stdin.read(), safe=""))
'
}

parse_token_from_credential_url() {
  local credential_line
  local credentials
  local password
  credential_line="$1"

  [[ "${credential_line}" == https://*@github.com* ]] || return 1
  credentials="${credential_line#https://}"
  credentials="${credentials%@github.com*}"
  [[ "${credentials}" == *:* ]] || return 1
  password="${credentials#*:}"
  [[ -n "${password}" ]] || return 1
  printf '%s\n' "${password}"
}

credential_url_matches_desired_login() {
  local credential_line
  local credentials
  local username
  credential_line="$1"

  [[ -n "${desired_login}" ]] || return 0
  [[ "${credential_line}" == https://*@github.com* ]] || return 0
  credentials="${credential_line#https://}"
  credentials="${credentials%@github.com*}"
  username="${credentials%%:*}"
  [[ "${username}" == "${desired_login}" ]]
}

parse_git_credential_password() {
  local credential_output
  local token
  credential_output="$1"

  token="$(printf '%s\n' "${credential_output}" | awk -F= '$1 == "password" && $2 != "" {print substr($0, 10); exit}')"
  [[ -n "${token}" ]] || return 1
  printf '%s\n' "${token}"
}

parse_git_credential_username() {
  local credential_output
  local username
  credential_output="$1"

  username="$(printf '%s\n' "${credential_output}" | awk -F= '$1 == "username" && $2 != "" {print substr($0, 10); exit}')"
  [[ -n "${username}" ]] || return 1
  printf '%s\n' "${username}"
}

github_login_for_token() {
  local candidate_token
  local previous_token
  local response_body
  candidate_token="$1"
  previous_token="${token-}"
  token="${candidate_token}"

  if ! response_body="$(api_request GET "/user" 2>/dev/null)"; then
    token="${previous_token}"
    return 1
  fi
  token="${previous_token}"
  json_string_field "${response_body}" "login"
}


token_matches_desired_login() {
  local candidate_token
  local actual_login
  candidate_token="$1"

  [[ -n "${desired_login}" ]] || return 0
  actual_login="$(github_login_for_token "${candidate_token}")"
  [[ "${actual_login}" == "${desired_login}" ]]
}

verify_git_credential_token_login() {
  local candidate_token
  local credential_output
  local username
  candidate_token="$1"
  credential_output="$2"

  [[ -n "${desired_login}" ]] || return 0
  username="$(parse_git_credential_username "${credential_output}" 2>/dev/null || true)"
  if [[ "${username}" != "${desired_login}" ]] &&
    ! token_matches_desired_login "${candidate_token}"; then
    return 1
  fi
  return 0
}

token_from_git_credential_fill() {
  local credential_output
  local token
  local username_line
  local repo_path
  repo_path="${repo_full_name}"
  username_line=""
  if [[ -n "${desired_login}" ]]; then
    username_line="$(printf 'username=%s\n' "${desired_login}")"
  fi

  credential_output="$(
    printf 'protocol=https\nhost=github.com\n%spath=%s\n\n' "${username_line}" "${repo_path}" |
      GIT_TERMINAL_PROMPT=0 git -C "${repo_root}" credential fill 2>/dev/null || true
  )"
  token="$(parse_git_credential_password "${credential_output}")"
  if [[ -n "${token}" ]]; then
    verify_git_credential_token_login "${token}" "${credential_output}" || return 1
    printf '%s\n' "${token}"
    return 0
  fi

  credential_output="$(
    printf 'protocol=https\nhost=github.com\n%s\n' "${username_line}" |
      GIT_TERMINAL_PROMPT=0 git -C "${repo_root}" credential fill 2>/dev/null || true
  )"
  token="$(parse_git_credential_password "${credential_output}")" || return 1
  verify_git_credential_token_login "${token}" "${credential_output}" || return 1
  printf '%s\n' "${token}"
}

read_first_non_empty_line() {
  local path
  path="$1"
  awk 'NF { sub(/^[[:space:]]+/, ""); sub(/[[:space:]]+$/, ""); print; exit }' "${path}"
}

token_from_named_credential_files() {
  local candidate
  local line
  local token

for candidate in \
  "${git_dir}/akra-github-credentials" \
  "${git_dir}/github-credentials" \
  "${git_dir}/refinedstone-credentials" \
  "${git_common_dir}/akra-github-credentials" \
  "${git_common_dir}/github-credentials" \
  "${git_common_dir}/refinedstone-credentials"; do
  [[ -n "${git_dir}" && -n "${git_common_dir}" ]] || break
  [[ -f "${candidate}" ]] || continue
  line="$(read_first_non_empty_line "${candidate}")"
  [[ -n "${line}" ]] || continue
  if token="$(parse_token_from_credential_url "${line}" 2>/dev/null)" &&
    credential_url_matches_desired_login "${line}"; then
    printf '%s\n' "${token}"
    return 0
  fi
  if [[ "${line}" != https://* ]] && token_matches_desired_login "${line}"; then
    printf '%s\n' "${line}"
    return 0
  fi
done
return 1
}

windows_current_user_credential_file() {
  local user_name
  local dir
  local direct_path
  local duplicate_name
  local -a user_names
  user_names=()
  [[ -n "${USERNAME:-}" ]] && user_names+=("${USERNAME}")
  if [[ -n "${USER:-}" ]]; then
    duplicate_name='false'
    for user_name in "${user_names[@]}"; do
      if [[ "${user_name,,}" == "${USER,,}" ]]; then
        duplicate_name='true'
        break
      fi
    done
    [[ "${duplicate_name}" == 'false' ]] && user_names+=("${USER}")
  fi
  ((${#user_names[@]} > 0)) || return 1
  for user_name in "${user_names[@]}"; do
    direct_path="/mnt/c/Users/${user_name}/.git-credentials"
    if [[ -f "${direct_path}" ]]; then
      printf '%s\n' "${direct_path}"
      return 0
    fi
  done
  for user_name in "${user_names[@]}"; do
    while IFS= read -r dir; do
      [[ "${dir##*/}" != "" ]] || continue
      if [[ "${dir##*/}" == "${user_name}" || "${dir##*/}" == "${user_name^}" || "${dir##*/,,}" == "${user_name,,}" ]]; then
        printf '%s\n' "${dir}/.git-credentials"
        return 0
      fi
    done < <(find /mnt/c/Users -maxdepth 1 -mindepth 1 -type d 2>/dev/null | sort)
  done
  return 1
}

credential_files_to_scan() {
  windows_current_user_credential_file || true
  if [[ -n "${HOME:-}" ]]; then
    printf '%s\n' "${HOME}/.git-credentials"
  fi
  if [[ -n "${USERPROFILE:-}" ]]; then
    printf '%s\n' "${USERPROFILE}/.git-credentials"
  fi
}

token_from_git_credential_files() {
  local file
  local line
  local token

  while IFS= read -r file; do
    [[ -f "${file}" ]] || continue
    while IFS= read -r line; do
      line="${line#"${line%%[![:space:]]*}"}"
      line="${line%"${line##*[![:space:]]}"}"
      [[ "${line}" == https://*@github.com* ]] || continue
      credential_url_matches_desired_login "${line}" || continue
      if token="$(parse_token_from_credential_url "${line}" 2>/dev/null)"; then
        printf '%s\n' "${token}"
        return 0
      fi
    done < "${file}"
  done < <(credential_files_to_scan)
  return 1
}

resolve_token() {
  if [[ -n "${AKRA_GITHUB_TOKEN:-}" ]]; then
    printf '%s\n' "${AKRA_GITHUB_TOKEN}"
    return 0
  fi
  if [[ -n "${GH_TOKEN:-}" ]]; then
    printf '%s\n' "${GH_TOKEN}"
    return 0
  fi
  if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    printf '%s\n' "${GITHUB_TOKEN}"
    return 0
  fi
  if command -v gh >/dev/null 2>&1; then
    local gh_auth_token
    gh_auth_token="$(GH_HOST=github.com gh auth token 2>/dev/null || true)"
    if [[ -n "${gh_auth_token}" ]]; then
      printf '%s\n' "${gh_auth_token}"
      return 0
    fi
  fi
  if token_from_git_credential_fill ||
    token_from_named_credential_files ||
    token_from_git_credential_files; then
    return 0
  fi
  true
}

resolve_gh_exec_token() {
  resolve_token
}

gh_api_login() {
  local candidate_token
  candidate_token="$(resolve_gh_exec_token)"
  if [[ -z "${candidate_token}" ]]; then
    candidate_token="$(resolve_token)"
  fi
  [[ -n "${candidate_token}" ]] || return 0
  github_login_for_token "${candidate_token}"
}

verify_gh_login_if_requested() {
  local actual_login
  [[ -n "${desired_login}" ]] || return 0
  actual_login="$(gh_api_login)"
  if [[ "${actual_login}" != "${desired_login}" ]]; then
    usage_error "expected GitHub login ${desired_login}, but token returned ${actual_login:-unknown}"
  fi
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

  status_code="$(run_github_curl "${method}" "${endpoint}" "${payload}" "${response_file}")"

  if [[ "${status_code}" != 2* ]]; then
    cat "${response_file}" >&2
    rm -f "${response_file}"
    return 1
  fi

  cat "${response_file}"
  rm -f "${response_file}"
}

verify_api_login_if_requested() {
  local response_body
  local actual_login
  [[ -n "${desired_login}" ]] || return 0
  response_body="$(api_request GET "/user")"
  actual_login="$(json_string_field "${response_body}" "login")"
  if [[ "${actual_login}" != "${desired_login}" ]]; then
    usage_error "expected GitHub login ${desired_login}, but token returned ${actual_login:-unknown}"
  fi
}

auth_status_with_api() {
  local response_body
  local login

  response_body="$(api_request GET "/user")"
  login="$(json_string_field "${response_body}" "login")"
  if [[ -n "${desired_login}" && "${login}" != "${desired_login}" ]]; then
    usage_error "expected GitHub login ${desired_login}, but token returned ${login:-unknown}"
  fi

  printf 'Logged in to github.com as %s\n' "${login:-unknown}"
}

list_prs_with_api() {
  local state
  local base_branch
  local head_branch
  local json_fields

  state="open"
  base_branch=""
  head_branch=""
  json_fields=""

  while (($# > 0)); do
    case "$1" in
      --state)
        require_value "$1" "${2-}"
        state="$2"
        shift 2
        ;;
      --base)
        require_value "$1" "${2-}"
        base_branch="$2"
        shift 2
        ;;
      --head)
        require_value "$1" "${2-}"
        head_branch="$2"
        shift 2
        ;;
      --json)
        require_value "$1" "${2-}"
        json_fields="$2"
        shift 2
        ;;
      *)
        usage_error "unsupported pr list option ${1} without gh installed"
        ;;
    esac
  done

  local endpoint
  endpoint="/repos/${repo_full_name}/pulls?state=$(url_encode "${state}")"
  if [[ -n "${base_branch}" ]]; then
    endpoint="${endpoint}&base=$(url_encode "${base_branch}")"
  fi
  if [[ -n "${head_branch}" ]]; then
    local head_owner
    head_owner="${repo_full_name%%/*}"
    if [[ "${head_branch}" == *:* ]]; then
      endpoint="${endpoint}&head=$(url_encode "${head_branch}")"
    else
      endpoint="${endpoint}&head=$(url_encode "${head_owner}:${head_branch}")"
    fi
  fi

  local response_body
  response_body="$(api_request GET "${endpoint}")"
  printf '%s' "${response_body}" | JSON_FIELDS="${json_fields}" python3 -c '
import json
import os
import sys

items = json.load(sys.stdin)
fields = {field for field in os.environ.get("JSON_FIELDS", "").split(",") if field}

def state_label(item):
    state = item.get("state", "").upper()
    if state == "CLOSED" and item.get("merged_at"):
        return "MERGED"
    return state

field_map = {
    "number": lambda item: item.get("number"),
    "url": lambda item: item.get("html_url"),
    "title": lambda item: item.get("title"),
    "state": state_label,
    "baseRefName": lambda item: (item.get("base") or {}).get("ref"),
    "headRefName": lambda item: (item.get("head") or {}).get("ref"),
    "isDraft": lambda item: bool(item.get("draft")),
}

result = []
for item in items:
    row = {
        field_name: field_map[field_name](item)
        for field_name in (fields or field_map.keys())
        if field_name in field_map
    }
    result.append(row)

print(json.dumps(result))
'
}

create_pr_with_api() {
  local base_branch
  local head_branch
  local title
  local body
  local draft
  local title_from_file
  local body_from_file
  local error_log

  base_branch=""
  head_branch=""
  title=""
  body=""
  draft="false"
  title_from_file="false"
  body_from_file="false"

  while (($# > 0)); do
    case "$1" in
      --base)
        require_value "$1" "${2-}"
        base_branch="$2"
        shift 2
        ;;
      --head)
        require_value "$1" "${2-}"
        head_branch="$2"
        shift 2
        ;;
      --title)
        require_value "$1" "${2-}"
        title="$2"
        shift 2
        ;;
      --title-file)
        title="$(read_option_file "$1" "${2-}")"
        title_from_file="true"
        shift 2
        ;;
      --body)
        require_value "$1" "${2-}"
        body="$2"
        shift 2
        ;;
      --body-file)
        body="$(read_option_file "$1" "${2-}")"
        body_from_file="true"
        shift 2
        ;;
      --draft)
        draft="true"
        shift
        ;;
      *)
        usage_error "unsupported pr create option ${1} without gh installed"
        ;;
    esac
  done

  if [[ -z "${base_branch}" || -z "${head_branch}" || -z "${title}" ]]; then
    usage_error "pr create requires --base, --head, and one of --title or --title-file"
  fi
  if [[ "${title_from_file}" != "true" || "${body_from_file}" != "true" ]]; then
    usage_error "pr create requires --title-file and --body-file for privacy-safe invocation"
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
  error_log="$(mktemp)"
  if response_body="$(api_request POST "/repos/${repo_full_name}/pulls" "${payload}" 2>"${error_log}")"; then
    :
  else
    if grep -q 'A pull request already exists' "${error_log}"; then
      local head_owner
      head_owner="${repo_full_name%%/*}"
      response_body="$(api_request GET "/repos/${repo_full_name}/pulls?state=open&head=$(url_encode "${head_owner}:${head_branch}")&base=$(url_encode "${base_branch}")")"
      local existing_url
      existing_url="$(json_string_field "${response_body}" "html_url")"
      if [[ -n "${existing_url}" ]]; then
        printf '%s\n' "${existing_url}"
        rm -f "${error_log}"
        return 0
      fi
    fi
    cat "${error_log}" >&2
    rm -f "${error_log}"
    exit 1
  fi
  rm -f "${error_log}"

  local pr_url
  pr_url="$(json_string_field "${response_body}" "html_url")"
  if [[ -n "${pr_url}" ]]; then
    printf '%s\n' "${pr_url}"
    return 0
  fi

  printf '%s\n' "${response_body}"
}

view_pr_with_api() {
  local pr_number
  local json_fields

  pr_number="${1-}"
  shift || true
  json_fields=""

  if [[ -z "${pr_number}" ]]; then
    usage_error "pr view requires a pull request number"
  fi

  while (($# > 0)); do
    case "$1" in
      --json)
        require_value "$1" "${2-}"
        json_fields="$2"
        shift 2
        ;;
      *)
        usage_error "unsupported pr view option ${1} without gh installed"
        ;;
    esac
  done

  local response_body
  response_body="$(api_request GET "/repos/${repo_full_name}/pulls/${pr_number}")"
  if [[ -z "${json_fields}" ]]; then
    printf '%s\n' "${response_body}"
    return 0
  fi

  printf '%s' "${response_body}" | JSON_FIELDS="${json_fields}" python3 -c '
import json
import os
import sys

item = json.load(sys.stdin)
fields = {field for field in os.environ.get("JSON_FIELDS", "").split(",") if field}

def state_label(item):
    state = item.get("state", "").upper()
    if state == "CLOSED" and item.get("merged_at"):
        return "MERGED"
    return state

field_map = {
    "number": lambda item: item.get("number"),
    "url": lambda item: item.get("html_url"),
    "title": lambda item: item.get("title"),
    "state": state_label,
    "baseRefName": lambda item: (item.get("base") or {}).get("ref"),
    "headRefName": lambda item: (item.get("head") or {}).get("ref"),
    "isDraft": lambda item: bool(item.get("draft")),
}

row = {
    field_name: field_map[field_name](item)
    for field_name in (fields or field_map.keys())
    if field_name in field_map
}

print(json.dumps(row))
'
}

close_pr_with_api() {
  local pr_number
  local response_body

  pr_number="${1-}"

  if [[ -z "${pr_number}" ]]; then
    usage_error "pr close requires a pull request number"
  fi

  response_body="$(api_request PATCH "/repos/${repo_full_name}/pulls/${pr_number}" '{"state":"closed"}')"
  printf '%s\n' "$(json_string_field "${response_body}" "html_url")"
}

merge_pr_with_api() {
  local pr_number
  local merge_method
  local delete_branch
  local pull_request_body
  local pull_request_url
  local head_branch
  local head_repo_full_name
  local payload

  pr_number="${1-}"
  shift || true
  merge_method="merge"
  delete_branch="false"

  if [[ -z "${pr_number}" ]]; then
    usage_error "pr merge requires a pull request number"
  fi

  while (($# > 0)); do
    case "$1" in
      --rebase)
        merge_method="rebase"
        shift
        ;;
      --squash)
        merge_method="squash"
        shift
        ;;
      --merge)
        merge_method="merge"
        shift
        ;;
      --delete-branch)
        delete_branch="true"
        shift
        ;;
      *)
        usage_error "unsupported pr merge option ${1} without gh installed"
        ;;
    esac
  done

  pull_request_body="$(api_request GET "/repos/${repo_full_name}/pulls/${pr_number}")"
  pull_request_url="$(json_string_field "${pull_request_body}" "html_url")"
  head_branch="$(json_path_string_field "${pull_request_body}" "head.ref")"
  head_repo_full_name="$(json_path_string_field "${pull_request_body}" "head.repo.full_name")"
  payload="$(printf '{"merge_method":"%s"}' "${merge_method}")"
  api_request PUT "/repos/${repo_full_name}/pulls/${pr_number}/merge" "${payload}" >/dev/null

  if [[ "${delete_branch}" == "true" && "${head_repo_full_name}" == "${repo_full_name}" && -n "${head_branch}" ]]; then
    api_request DELETE "/repos/${repo_full_name}/git/refs/heads/${head_branch}" >/dev/null || true
  fi

  printf '%s\n' "${pull_request_url:-https://github.com/${repo_full_name}/pull/${pr_number}}"
}

parse_review_reply_args() {
  pr_number=""
  comment_id=""
  body=""
  local body_from_file
  body_from_file="false"

  while (($# > 0)); do
    case "$1" in
      --pr)
        require_value "$1" "${2-}"
        pr_number="$2"
        shift 2
        ;;
      --comment-id)
        require_value "$1" "${2-}"
        comment_id="$2"
        shift 2
        ;;
      --body)
        require_value "$1" "${2-}"
        body="$2"
        shift 2
        ;;
      --body-file)
        body="$(read_option_file "$1" "${2-}")"
        body_from_file="true"
        shift 2
        ;;
      *)
        usage_error "unsupported review-reply option ${1}"
        ;;
    esac
  done

  if [[ -z "${pr_number}" || -z "${comment_id}" || -z "${body}" ]]; then
    usage_error "review-reply requires --pr, --comment-id, and --body"
  fi
  if [[ "${body_from_file}" != "true" ]]; then
    usage_error "review-reply requires --body-file for privacy-safe invocation"
  fi
}

reply_review_comment_with_gh() {
  parse_review_reply_args "$@"
  if [[ -n "${gh_exec_token:-}" ]]; then
    GH_TOKEN="${gh_exec_token}" GH_HOST=github.com gh api \
      -X POST \
      "repos/${repo_full_name}/pulls/comments/${comment_id}/replies" \
      -f "body=${body}" >/dev/null
  elif [[ -n "${AKRA_GITHUB_TOKEN:-${GH_TOKEN:-${GITHUB_TOKEN:-}}}" ]]; then
    GH_TOKEN="${AKRA_GITHUB_TOKEN:-${GH_TOKEN:-${GITHUB_TOKEN:-}}}" GH_HOST=github.com gh api \
      -X POST \
      "repos/${repo_full_name}/pulls/comments/${comment_id}/replies" \
      -f "body=${body}" >/dev/null
  else
    GH_HOST=github.com gh api \
      -X POST \
      "repos/${repo_full_name}/pulls/comments/${comment_id}/replies" \
      -f "body=${body}" >/dev/null
  fi
}

reply_review_comment_with_api() {
  local payload
  parse_review_reply_args "$@"
  payload=$(printf '{"body":"%s"}' "$(json_escape "${body}")")
  api_request POST "/repos/${repo_full_name}/pulls/comments/${comment_id}/replies" "${payload}" >/dev/null
}

if [[ "${1-}:${2-}" == "auth:status" ]]; then
  token="$(resolve_gh_exec_token)"
  if [[ -z "${token}" ]]; then
    token="$(resolve_token)"
  fi
  if [[ -z "${token}" ]]; then
    usage_error "gh auth status requires a GitHub token in AKRA_GITHUB_TOKEN, GH_TOKEN, GITHUB_TOKEN, gh auth token, git credential fill, or local git credential files"
  fi
  shift 2
  auth_status_with_api "$@"
  exit 0
fi

repo_full_name="$(parse_repo_full_name)"

if command -v gh >/dev/null 2>&1; then
  gh_exec_token="$(resolve_gh_exec_token)"
  verify_gh_login_if_requested
  if [[ "${1-}" == "review-reply" ]]; then
    token="${gh_exec_token}"
    if [[ -z "${token}" ]]; then
      token="$(resolve_token)"
    fi
    if [[ -z "${token}" ]]; then
      usage_error "review-reply requires a GitHub token in AKRA_GITHUB_TOKEN, GH_TOKEN, GITHUB_TOKEN, gh auth token, git credential fill, or local git credential files"
    fi
    shift
    reply_review_comment_with_api "$@"
    exit 0
  fi
  if [[ "${1-}" == "pr" && "${2-}" == "create" ]]; then
    token="${gh_exec_token}"
    if [[ -z "${token}" ]]; then
      token="$(resolve_token)"
    fi
    if [[ -z "${token}" ]]; then
      usage_error "pr create requires a GitHub token in AKRA_GITHUB_TOKEN, GH_TOKEN, GITHUB_TOKEN, gh auth token, git credential fill, or local git credential files"
    fi
    shift 2
    create_pr_with_api "$@"
    exit 0
  fi
  token="${gh_exec_token}"
  if [[ -z "${token}" ]]; then
    token="$(resolve_token)"
  fi
  if [[ -n "${token}" ]]; then
    GH_TOKEN="${token}" GH_HOST=github.com exec gh "$@"
  fi
  GH_HOST=github.com exec gh "$@"
fi

token="$(resolve_token)"
if [[ -z "${token}" ]]; then
  usage_error "gh is not installed and no GitHub token was found in AKRA_GITHUB_TOKEN, GH_TOKEN, GITHUB_TOKEN, git credential fill, or local git credential files"
fi

case "${1-}:${2-}" in
  auth:status)
    shift 2
    auth_status_with_api "$@"
    ;;
  pr:create)
    verify_api_login_if_requested
    shift 2
    create_pr_with_api "$@"
    ;;
  pr:list)
    verify_api_login_if_requested
    shift 2
    list_prs_with_api "$@"
    ;;
  pr:view)
    verify_api_login_if_requested
    shift 2
    view_pr_with_api "$@"
    ;;
  pr:close)
    verify_api_login_if_requested
    shift 2
    close_pr_with_api "$@"
    ;;
  pr:merge)
    verify_api_login_if_requested
    shift 2
    merge_pr_with_api "$@"
    ;;
  review-reply:*)
    verify_api_login_if_requested
    shift
    reply_review_comment_with_api "$@"
    ;;
  *)
    usage_error "gh is not installed and direct fallback supports 'auth status', 'pr create', 'pr list', 'pr view', 'pr close', 'pr merge', and 'review-reply'"
    ;;
esac
