#!/usr/bin/env bash
set -euo pipefail

script_name="gh-akra"
github_response_max_bytes=8388608
temporary_files=()

cleanup_temporary_files() {
  local path
  for path in "${temporary_files[@]}"; do
    rm -f -- "${path}"
  done
}

trap cleanup_temporary_files EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

register_temporary_file() {
  temporary_files+=("$1")
}

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

sanitize_inherited_git_environment() {
  local git_env_name
  for git_env_name in "${!GIT_@}"; do
    unset "${git_env_name}"
  done
  unset SSH_ASKPASS SSH_ASKPASS_REQUIRE
  export GIT_TERMINAL_PROMPT=0
  export GIT_NO_REPLACE_OBJECTS=1
  export GIT_NO_LAZY_FETCH=1
  export GIT_OPTIONAL_LOCKS=0
  export GIT_CONFIG_NOSYSTEM=1
  export GIT_ATTR_NOSYSTEM=1
  export GIT_EDITOR=:
  export GIT_SEQUENCE_EDITOR=:
  export GCM_INTERACTIVE=Never
  export GCM_GUI_PROMPT=0
  if [[ -n "${AKRA_TRUSTED_GIT_SSL_CAINFO:-}" ]]; then
    export GIT_SSL_CAINFO="${AKRA_TRUSTED_GIT_SSL_CAINFO}"
  fi
  if [[ -n "${AKRA_TRUSTED_GIT_SSL_CAPATH:-}" ]]; then
    export GIT_SSL_CAPATH="${AKRA_TRUSTED_GIT_SSL_CAPATH}"
  fi
}

safe_git_impl() (
  local -a hardening_args=(
    --no-pager
    --no-replace-objects
    -c "core.hooksPath=/dev/null"
    -c "core.fsmonitor=false"
    -c "core.attributesFile=/dev/null"
    -c "commit.gpgSign=false"
    -c "tag.gpgSign=false"
    -c "merge.gpgSign=false"
    -c "push.gpgSign=false"
    -c "maintenance.auto=false"
    -c "gc.auto=0"
    -c "gc.autoDetach=false"
    -c "fetch.writeCommitGraph=false"
  )

  sanitize_inherited_git_environment

  command git "${hardening_args[@]}" "$@" </dev/null
)

safe_git() {
  safe_git_impl "$@"
}

sanitize_inherited_git_environment

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

repo_root="$(safe_git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "${repo_root}" ]]; then
  if [[ "${auth_status_only}" == "true" ]]; then
    repo_root="${PWD}"
    git_dir=""
  else
    usage_error "not inside a git repository"
  fi
else
  git_dir="$(safe_git rev-parse --path-format=absolute --git-dir 2>/dev/null || true)"
  if [[ -z "${git_dir}" ]]; then
    usage_error "failed to resolve git dir"
  fi
fi

if [[ -z "${desired_login}" && -n "${git_dir:-}" ]]; then
  desired_login="$(safe_git -C "${repo_root}" config --get akra.githubLogin 2>/dev/null || true)"
fi

if [[ -n "${AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN+x}" ]]; then
  usage_error "AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN is no longer supported; use an explicit token environment variable or gh auth token"
fi

push_remote="${AKRA_GITHUB_PUSH_REMOTE:-}"
if [[ -z "${push_remote}" && -n "${git_dir:-}" ]]; then
  push_remote="$(safe_git -C "${repo_root}" config --get akra.githubPushRemote 2>/dev/null || true)"
fi
push_remote="${push_remote:-origin}"
if [[ ! "${push_remote}" =~ ^[A-Za-z0-9._-]+$ ]] ||
  [[ "${push_remote}" == -* || "${push_remote}" == *. || "${push_remote}" == *.lock ||
    "${push_remote}" == *..* || "${push_remote}" == *@\{* ]]; then
  usage_error "invalid GitHub push remote name"
fi
if [[ -n "${git_dir:-}" ]] &&
  ! safe_git -C "${repo_root}" remote get-url --push "${push_remote}" >/dev/null 2>&1; then
  usage_error "configured GitHub push remote ${push_remote} is not available"
fi


parse_repo_full_name() {
  local owner
  local repository
  local remote_url
  remote_url="$(safe_git -C "${repo_root}" remote get-url --push "${push_remote}")"

  case "${remote_url}" in
    git@github.com:*)
      remote_url="${remote_url#git@github.com:}"
      ;;
    ssh://git@github.com/*)
      remote_url="${remote_url#ssh://git@github.com/}"
      ;;
    https://github.com/*)
      remote_url="${remote_url#https://github.com/}"
      ;;
    https://*@github.com/*)
      usage_error "configured GitHub HTTPS push remote must not embed a username or credential"
      ;;
    *)
      usage_error "configured GitHub push remote ${push_remote} does not use a supported GitHub URL"
      ;;
  esac

  remote_url="${remote_url%.git}"
  if [[ ! "${remote_url}" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
    usage_error "failed to parse a safe GitHub repository identity from push remote ${push_remote}"
  fi
  owner="${remote_url%%/*}"
  repository="${remote_url#*/}"
  if [[ "${owner}" == "." || "${owner}" == ".." ||
    "${repository}" == "." || "${repository}" == ".." ]]; then
    usage_error "configured GitHub repository identity contains a path traversal segment"
  fi
  printf '%s\n' "${remote_url}"
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
    printf 'silent\nshow-error\nproto = "=https"\noutput = "%s"\nwrite-out = "%%{http_code}"\nconnect-timeout = 10\nmax-time = 30\nmax-filesize = "%s"\nrequest = "%s"\nheader = "Accept: application/vnd.github+json"\nheader = "Authorization: Bearer %s"\nheader = "User-Agent: gh-akra.sh"\nheader = "X-GitHub-Api-Version: 2022-11-28"\nurl = "https://api.github.com%s"\n' \
      "$(curl_config_escape "${response_file}")" \
      "${github_response_max_bytes}" \
      "$(curl_config_escape "${method}")" \
      "$(curl_config_escape "${token}")" \
      "$(curl_config_escape "${endpoint}")"
    if [[ -n "${payload}" ]]; then
      printf 'data = "%s"\n' "$(curl_config_escape "${payload}")"
    fi
  )
  printf '%s' "${config}" | curl -q --config -
}

read_bounded_response_file() {
  local response_file
  response_file="$1"

  python3 - "${response_file}" "${github_response_max_bytes}" <<'PY'
import os
import stat
import sys

path = sys.argv[1]
limit = int(sys.argv[2])
flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
try:
    before = os.lstat(path)
    if not stat.S_ISREG(before.st_mode):
        raise SystemExit("gh-akra: GitHub response file is not a regular file")
    if before.st_nlink != 1 or before.st_size > limit:
        raise SystemExit("gh-akra: GitHub response file failed its metadata bounds")
    flags |= getattr(os, "O_NONBLOCK", 0)
    descriptor = os.open(path, flags)
except OSError:
    raise SystemExit("gh-akra: GitHub response file could not be opened safely")

try:
    opened = os.fstat(descriptor)
    if not stat.S_ISREG(opened.st_mode):
        raise SystemExit("gh-akra: GitHub response file is not a regular file")
    if (before.st_dev, before.st_ino) != (opened.st_dev, opened.st_ino):
        raise SystemExit("gh-akra: GitHub response file changed before validation")
    if before.st_nlink != 1 or opened.st_nlink != 1:
        raise SystemExit("gh-akra: GitHub response file has an unsafe link count")
    if before.st_size > limit or opened.st_size > limit:
        raise SystemExit("gh-akra: GitHub response exceeded the configured byte limit")
    with os.fdopen(descriptor, "rb", closefd=False) as stream:
        body = stream.read(limit + 1)
        extra = stream.read(1)
    if len(body) > limit or extra:
        raise SystemExit("gh-akra: GitHub response exceeded the configured byte limit")
    after = os.fstat(descriptor)
    if (opened.st_dev, opened.st_ino, opened.st_size) != (
        after.st_dev,
        after.st_ino,
        after.st_size,
    ):
        raise SystemExit("gh-akra: GitHub response file changed during validation")
    sys.stdout.buffer.write(body)
finally:
    os.close(descriptor)
PY
}


json_string_field() {
  local body
  local field_name
  body="$1"
  field_name="$2"

  printf '%s' "${body}" | python3 -c '
import json
import sys

field_name = sys.argv[1]
data = json.load(sys.stdin)
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

  printf '%s' "${body}" | python3 -c '
import json
import sys

value = json.load(sys.stdin)
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
  true
}

resolve_gh_exec_token() {
  resolve_token
}

credential_source_help() {
  printf '%s' "AKRA_GITHUB_TOKEN, GH_TOKEN, GITHUB_TOKEN, or gh auth token; repository credential helpers and direct credential-file scanning are not used"
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
  local response_body
  local status_code

  method="$1"
  endpoint="$2"
  payload="${3-}"
  response_file="$(mktemp)"
  register_temporary_file "${response_file}"

  if ! status_code="$(run_github_curl "${method}" "${endpoint}" "${payload}" "${response_file}")"; then
    rm -f "${response_file}"
    return 1
  fi
  if ! response_body="$(read_bounded_response_file "${response_file}")"; then
    rm -f "${response_file}"
    return 1
  fi
  rm -f "${response_file}"

  if [[ ! "${status_code}" =~ ^2[0-9][0-9]$ ]]; then
    printf '%s' "${response_body}" >&2
    return 1
  fi

  printf '%s' "${response_body}"
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

github_command_is_read_only() {
  case "${1-}:${2-}" in
    pr:list|pr:view|repo:visibility)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

verify_write_identity() {
  local actual_login
  local remote_url

  if [[ -z "${desired_login}" ]]; then
    usage_error "GitHub writes require AKRA_GITHUB_LOGIN or repo-local git config akra.githubLogin"
  fi
  actual_login="$(github_login_for_token "${token}" 2>/dev/null || true)"
  if [[ "${actual_login}" != "${desired_login}" ]]; then
    usage_error "expected GitHub login ${desired_login}, but API token returned ${actual_login:-unknown}"
  fi

  remote_url="$(safe_git -C "${repo_root}" remote get-url --push "${push_remote}")"
  case "${remote_url}" in
    https://github.com/*)
      ;;
    https://*@github.com/*)
      usage_error "GitHub writes reject HTTPS push remotes with embedded identity"
      ;;
    *)
      usage_error "GitHub writes require an HTTPS push remote so credential identity can be verified"
      ;;
  esac
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

pr_json_requires_gate_enrichment() {
  local fields
  fields=",${1},"
  case "${fields}" in
    *,headRefOid,*|*,reviewDecision,*|*,mergeStateStatus,*|*,statusCheckRollup,*|*,approvedReviewCommitOids,*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

pr_gate_query_payload() {
  local shape
  shape="$1"

  PR_JSON_SHAPE="${shape}" python3 -c '
import json
import os
import sys

shape = os.environ["PR_JSON_SHAPE"]
payload = json.load(sys.stdin)
if shape == "list":
    if not isinstance(payload, list):
        raise SystemExit("gh-akra: GitHub pull request list response must be an array")
    items = payload
elif shape == "view":
    if not isinstance(payload, dict):
        raise SystemExit("gh-akra: GitHub pull request response must be an object")
    items = [payload]
else:
    raise SystemExit("gh-akra: unsupported pull request JSON shape")

node_ids = []
for item in items:
    if not isinstance(item, dict):
        raise SystemExit("gh-akra: GitHub pull request response contains a non-object item")
    node_id = item.get("node_id")
    if not isinstance(node_id, str) or not node_id:
        raise SystemExit("gh-akra: GitHub pull request response omitted its node id")
    node_ids.append(node_id)

if not node_ids:
    raise SystemExit(0)

query = """query($ids: [ID!]!) {
  nodes(ids: $ids) {
    ... on PullRequest {
      number
      headRefOid
      reviewDecision
      mergeStateStatus
      commits(last: 1) {
        nodes {
          commit {
            statusCheckRollup {
              contexts(first: 100) {
                nodes {
                  __typename
                  ... on CheckRun { conclusion }
                  ... on StatusContext { state }
                }
                pageInfo { hasNextPage }
              }
            }
          }
        }
      }
      reviews(first: 100, states: [APPROVED]) {
        nodes {
          state
          commit { oid }
        }
        pageInfo { hasNextPage }
      }
    }
  }
}"""
print(json.dumps({"query": query, "variables": {"ids": node_ids}}, separators=(",", ":")))
'
}

render_pr_json_with_api() {
  local shape
  local json_fields
  local response_body
  local gate_file
  local gate_requested
  local query_payload
  local rendered

  shape="$1"
  json_fields="$2"
  response_body="$3"
  gate_file=""
  gate_requested="false"

  if pr_json_requires_gate_enrichment "${json_fields}"; then
    gate_requested="true"
    query_payload="$(printf '%s' "${response_body}" | pr_gate_query_payload "${shape}")"
    if [[ -z "${query_payload}" ]]; then
      gate_requested="false"
    else
      gate_file="$(mktemp)"
      register_temporary_file "${gate_file}"
      if ! api_request POST "/graphql" "${query_payload}" > "${gate_file}"; then
        rm -f "${gate_file}"
        return 1
      fi
    fi
  fi

  if ! rendered="$({ printf '%s' "${response_body}"; } | \
    PR_JSON_SHAPE="${shape}" \
      JSON_FIELDS="${json_fields}" \
      PR_GATE_REQUESTED="${gate_requested}" \
      PR_GATE_FILE="${gate_file}" \
      python3 -c '
import json
import os
import sys

shape = os.environ["PR_JSON_SHAPE"]
fields = [field for field in os.environ.get("JSON_FIELDS", "").split(",") if field]
gate_requested = os.environ["PR_GATE_REQUESTED"] == "true"
payload = json.load(sys.stdin)
if shape == "list":
    if not isinstance(payload, list):
        raise SystemExit("gh-akra: GitHub pull request list response must be an array")
    items = payload
elif shape == "view":
    if not isinstance(payload, dict):
        raise SystemExit("gh-akra: GitHub pull request response must be an object")
    items = [payload]
else:
    raise SystemExit("gh-akra: unsupported pull request JSON shape")

gate_by_number = {}
if gate_requested:
    with open(os.environ["PR_GATE_FILE"], encoding="utf-8") as gate_stream:
        gate_payload = json.load(gate_stream)
    if gate_payload.get("errors"):
        raise SystemExit("gh-akra: GitHub PR inspection query failed")
    data = gate_payload.get("data")
    nodes = data.get("nodes") if isinstance(data, dict) else None
    if not isinstance(nodes, list):
        raise SystemExit("gh-akra: GitHub PR inspection query omitted nodes")
    for node in nodes:
        if not isinstance(node, dict) or not isinstance(node.get("number"), int):
            raise SystemExit("gh-akra: GitHub PR inspection query returned an invalid node")
        number = node["number"]
        if number in gate_by_number:
            raise SystemExit("gh-akra: GitHub PR inspection query returned duplicate nodes")
        gate_by_number[number] = node

def state_label(item):
    state = item.get("state", "").upper()
    if state == "CLOSED" and item.get("merged_at"):
        return "MERGED"
    return state

def gate_node(item):
    if not gate_requested:
        return None
    number = item.get("number")
    node = gate_by_number.get(number)
    if node is None:
        raise SystemExit("gh-akra: GitHub PR inspection query did not return every requested pull request")
    head_oid = node.get("headRefOid")
    merge_state = node.get("mergeStateStatus")
    if not isinstance(head_oid, str) or not head_oid or not isinstance(merge_state, str):
        raise SystemExit("gh-akra: GitHub PR inspection query omitted required gate fields")
    return node

def status_check_rollup(node):
    commits = node.get("commits")
    commit_nodes = commits.get("nodes") if isinstance(commits, dict) else None
    if not isinstance(commit_nodes, list) or len(commit_nodes) != 1:
        raise SystemExit("gh-akra: GitHub PR inspection query omitted the head commit")
    commit = commit_nodes[0].get("commit") if isinstance(commit_nodes[0], dict) else None
    rollup = commit.get("statusCheckRollup") if isinstance(commit, dict) else None
    if rollup is None:
        return []
    contexts = rollup.get("contexts") if isinstance(rollup, dict) else None
    context_nodes = contexts.get("nodes") if isinstance(contexts, dict) else None
    page_info = contexts.get("pageInfo") if isinstance(contexts, dict) else None
    if not isinstance(context_nodes, list) or not isinstance(page_info, dict):
        raise SystemExit("gh-akra: GitHub PR inspection query returned an invalid check rollup")
    if page_info.get("hasNextPage") is not False:
        raise SystemExit("gh-akra: GitHub PR has more than 100 checks; fallback inspection cannot prove the gate")

    checks = []
    for context in context_nodes:
        if not isinstance(context, dict):
            raise SystemExit("gh-akra: GitHub PR inspection query returned an invalid check")
        context_type = context.get("__typename")
        if context_type == "CheckRun":
            checks.append({"conclusion": context.get("conclusion")})
        elif context_type == "StatusContext":
            checks.append({"state": context.get("state")})
        else:
            checks.append({})
    return checks

def approved_review_commit_oids(node):
    reviews = node.get("reviews")
    review_nodes = reviews.get("nodes") if isinstance(reviews, dict) else None
    page_info = reviews.get("pageInfo") if isinstance(reviews, dict) else None
    if not isinstance(review_nodes, list) or not isinstance(page_info, dict):
        raise SystemExit("gh-akra: GitHub PR inspection query omitted approved reviews")
    if page_info.get("hasNextPage") is not False:
        raise SystemExit("gh-akra: GitHub PR has more than 100 approved reviews; fallback inspection cannot prove the gate")

    commit_oids = set()
    for review in review_nodes:
        if not isinstance(review, dict) or review.get("state") != "APPROVED":
            raise SystemExit("gh-akra: GitHub PR inspection query returned an invalid approved review")
        commit = review.get("commit")
        oid = commit.get("oid") if isinstance(commit, dict) else None
        if not isinstance(oid, str) or not oid:
            raise SystemExit("gh-akra: GitHub approved review omitted its commit OID")
        commit_oids.add(oid)
    return sorted(commit_oids)

default_fields = [
    "number",
    "url",
    "title",
    "state",
    "baseRefName",
    "headRefName",
    "isDraft",
]
selected_fields = fields or default_fields
result = []
for item in items:
    if not isinstance(item, dict):
        raise SystemExit("gh-akra: GitHub pull request response contains a non-object item")
    node = gate_node(item)
    base = item.get("base") or {}
    head = item.get("head") or {}
    field_map = {
        "number": item.get("number"),
        "url": item.get("html_url"),
        "title": item.get("title"),
        "state": state_label(item),
        "baseRefName": base.get("ref") if isinstance(base, dict) else None,
        "headRefName": head.get("ref") if isinstance(head, dict) else None,
        "isDraft": bool(item.get("draft")),
    }
    if node is not None:
        field_map.update({
            "headRefOid": node.get("headRefOid"),
            "reviewDecision": node.get("reviewDecision"),
            "mergeStateStatus": node.get("mergeStateStatus"),
            "statusCheckRollup": status_check_rollup(node),
            "approvedReviewCommitOids": approved_review_commit_oids(node),
        })
    result.append({field: field_map[field] for field in selected_fields if field in field_map})

output = result if shape == "list" else result[0]
print(json.dumps(output))
' )"; then
    [[ -z "${gate_file}" ]] || rm -f "${gate_file}"
    return 1
  fi

  [[ -z "${gate_file}" ]] || rm -f "${gate_file}"
  printf '%s\n' "${rendered}"
}

pr_command_requests_gate_enrichment() {
  local fields
  [[ "${1-}" == "pr" && ("${2-}" == "list" || "${2-}" == "view") ]] || return 1
  shift 2
  while (($# > 0)); do
    case "$1" in
      --json)
        fields="${2-}"
        [[ -n "${fields}" ]] && pr_json_requires_gate_enrichment "${fields}"
        return
        ;;
      --json=*)
        fields="${1#--json=}"
        [[ -n "${fields}" ]] && pr_json_requires_gate_enrichment "${fields}"
        return
        ;;
    esac
    shift
  done
  return 1
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
  render_pr_json_with_api "list" "${json_fields}" "${response_body}"
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
  register_temporary_file "${error_log}"
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

  render_pr_json_with_api "view" "${json_fields}" "${response_body}"
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
      --body|--body=*)
        usage_error "review-reply accepts only --body-file for privacy-safe invocation"
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
    usage_error "review-reply requires --pr, --comment-id, and --body-file"
  fi
  if [[ ! "${pr_number}" =~ ^[1-9][0-9]*$ ]]; then
    usage_error "review-reply --pr must be a positive decimal integer"
  fi
  if [[ ! "${comment_id}" =~ ^[1-9][0-9]*$ ]]; then
    usage_error "review-reply --comment-id must be a positive decimal integer"
  fi
  if [[ "${body_from_file}" != "true" ]]; then
    usage_error "review-reply requires --body-file for privacy-safe invocation"
  fi
}

send_review_comment_reply_with_api() {
  local payload
  payload=$(printf '{"body":"%s"}' "$(json_escape "${body}")")
  api_request POST "/repos/${repo_full_name}/pulls/${pr_number}/comments/${comment_id}/replies" "${payload}" >/dev/null
}

repo_visibility_with_api() {
  local response_body
  local visibility
  response_body="$(api_request GET "/repos/${repo_full_name}")"
  visibility="$(json_string_field "${response_body}" "visibility")"
  case "${visibility}" in
    private|internal|public)
      printf '%s\n' "${visibility}"
      ;;
    *)
      usage_error "GitHub repository visibility response was unavailable"
      ;;
  esac
}

if [[ "${1-}:${2-}" == "auth:status" ]]; then
  if [[ -n "${git_dir:-}" ]]; then
    repo_full_name="$(parse_repo_full_name)"
  fi
  token="$(resolve_gh_exec_token)"
  if [[ -z "${token}" ]]; then
    token="$(resolve_token)"
  fi
  if [[ -z "${token}" ]]; then
    usage_error "gh auth status requires a GitHub token from $(credential_source_help)"
  fi
  shift 2
  auth_status_with_api "$@"
  exit 0
fi

repo_full_name="$(parse_repo_full_name)"

if [[ "${1-}:${2-}" == "auth:write-status" ]]; then
  token="$(resolve_gh_exec_token)"
  if [[ -z "${token}" ]]; then
    token="$(resolve_token)"
  fi
  if [[ -z "${token}" ]]; then
    usage_error "GitHub write identity requires a token from $(credential_source_help)"
  fi
  verify_write_identity
  printf 'GitHub write identity verified as %s\n' "${desired_login}"
  exit 0
fi

if [[ "${1-}" == "review-reply" ]]; then
  parse_review_reply_args "${@:2}"
fi

if ! github_command_is_read_only "$@"; then
  token="$(resolve_gh_exec_token)"
  if [[ -z "${token}" ]]; then
    token="$(resolve_token)"
  fi
  if [[ -z "${token}" ]]; then
    usage_error "GitHub write identity requires a token from $(credential_source_help)"
  fi
  verify_write_identity
fi

# `approvedReviewCommitOids` is an Akra-owned security field rather than a gh CLI
# JSON field. Resolve every gate snapshot through one GraphQL request so head,
# aggregate decision, checks, and review commit OIDs describe the same PR state.
if pr_command_requests_gate_enrichment "$@"; then
  token="$(resolve_gh_exec_token)"
  if [[ -z "${token}" ]]; then
    token="$(resolve_token)"
  fi
  if [[ -z "${token}" ]]; then
    usage_error "PR gate inspection requires a GitHub token from $(credential_source_help)"
  fi
  verify_api_login_if_requested
  case "${1-}:${2-}" in
    pr:list)
      shift 2
      list_prs_with_api "$@"
      ;;
    pr:view)
      shift 2
      view_pr_with_api "$@"
      ;;
    *)
      usage_error "unsupported PR gate inspection command"
      ;;
  esac
  exit 0
fi

if command -v gh >/dev/null 2>&1; then
  gh_exec_token="$(resolve_gh_exec_token)"
  verify_gh_login_if_requested
  if [[ "${1-}:${2-}" == "repo:visibility" ]]; then
    token="${gh_exec_token}"
    if [[ -z "${token}" ]]; then
      token="$(resolve_token)"
    fi
    if [[ -z "${token}" ]]; then
      usage_error "repo visibility requires a GitHub token from $(credential_source_help)"
    fi
    shift 2
    repo_visibility_with_api "$@"
    exit 0
  fi
  if [[ "${1-}" == "review-reply" ]]; then
    token="${gh_exec_token}"
    if [[ -z "${token}" ]]; then
      token="$(resolve_token)"
    fi
    if [[ -z "${token}" ]]; then
      usage_error "review-reply requires a GitHub token from $(credential_source_help)"
    fi
    send_review_comment_reply_with_api
    exit 0
  fi
  if [[ "${1-}" == "pr" && "${2-}" == "create" ]]; then
    token="${gh_exec_token}"
    if [[ -z "${token}" ]]; then
      token="$(resolve_token)"
    fi
    if [[ -z "${token}" ]]; then
      usage_error "pr create requires a GitHub token from $(credential_source_help)"
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
  usage_error "gh is not installed and no GitHub token was found from $(credential_source_help)"
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
  repo:visibility)
    verify_api_login_if_requested
    shift 2
    repo_visibility_with_api "$@"
    ;;
  review-reply:*)
    verify_api_login_if_requested
    send_review_comment_reply_with_api
    ;;
  *)
    usage_error "gh is not installed and direct fallback supports 'auth status', 'repo visibility', 'pr create', 'pr list', 'pr view', 'pr close', 'pr merge', and 'review-reply'"
    ;;
esac
