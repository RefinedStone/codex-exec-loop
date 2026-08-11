#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/validate_native_release_version.sh [--tag <tag>] [--manifest <path>] \
  [--release-commit <sha> --allowed-ref <ref>] [--repository <path>]

Validate that a stable release tag using the vMAJOR.MINOR.PATCH convention matches the Rust
package version in Cargo.toml. Prerelease and build-metadata tags are rejected until the release
workflow defines an explicit npm dist-tag policy for them.

When --release-commit and --allowed-ref are supplied together, the tag must resolve to that exact
commit and the commit must be an ancestor of the fetched allowed ref. The release workflow uses
this gate with origin/prerelease so an arbitrary off-branch tag cannot publish packages.

Examples:
  scripts/validate_native_release_version.sh --tag v1.3.4
  GITHUB_REF_NAME=v1.3.4 scripts/validate_native_release_version.sh
EOF
}

require_value() {
  local option="$1"
  local value="${2-}"
  if [[ -z "${value}" ]]; then
    echo "validate_native_release_version: missing value for ${option}" >&2
    usage >&2
    exit 1
  fi
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
manifest_path="${repo_root}/Cargo.toml"
tag_name="${GITHUB_REF_NAME:-}"
release_commit=""
allowed_ref=""
repository_path="${repo_root}"

while (($# > 0)); do
  case "$1" in
    --tag)
      require_value "$1" "${2-}"
      tag_name="$2"
      shift 2
      ;;
    --manifest)
      require_value "$1" "${2-}"
      manifest_path="$2"
      shift 2
      ;;
    --release-commit)
      require_value "$1" "${2-}"
      release_commit="$2"
      shift 2
      ;;
    --allowed-ref)
      require_value "$1" "${2-}"
      allowed_ref="$2"
      shift 2
      ;;
    --repository)
      require_value "$1" "${2-}"
      repository_path="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "validate_native_release_version: unsupported option '$1'" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "${tag_name}" ]]; then
  echo "validate_native_release_version: --tag or GITHUB_REF_NAME is required" >&2
  exit 1
fi

if [[ ! -f "${manifest_path}" ]]; then
  echo "validate_native_release_version: manifest not found: ${manifest_path}" >&2
  exit 1
fi
if ! command -v python3 >/dev/null 2>&1 || ! python3 -c '
try:
    import tomllib
except ModuleNotFoundError:
    import tomli
' >/dev/null 2>&1; then
  echo "validate_native_release_version: python3 with tomllib support is required" >&2
  exit 1
fi

if [[ -n "${release_commit}" || -n "${allowed_ref}" ]]; then
  if [[ -z "${release_commit}" || -z "${allowed_ref}" ]]; then
    echo "validate_native_release_version: --release-commit and --allowed-ref must be supplied together" >&2
    exit 1
  fi
  if [[ ! -d "${repository_path}" ]]; then
    echo "validate_native_release_version: repository not found: ${repository_path}" >&2
    exit 1
  fi
  if [[ ! "${release_commit}" =~ ^[0-9A-Fa-f]{40,64}$ ]]; then
    echo "validate_native_release_version: --release-commit must be a full hexadecimal object id" >&2
    exit 1
  fi
  if [[ "${allowed_ref}" != refs/* ]] ||
    ! git -C "${repository_path}" check-ref-format "${allowed_ref}" >/dev/null 2>&1; then
    echo "validate_native_release_version: --allowed-ref must be a valid fully qualified git ref" >&2
    exit 1
  fi
fi

if [[ "${tag_name}" != v* || "${tag_name}" == "v" ]]; then
  echo "validate_native_release_version: release tag must use stable vMAJOR.MINOR.PATCH: ${tag_name}" >&2
  exit 1
fi
release_version="${tag_name#v}"

if [[ ! "${release_version}" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  cat >&2 <<EOF
validate_native_release_version: official releases require a stable vMAJOR.MINOR.PATCH tag
  tag: ${tag_name}

Prerelease and build-metadata tags are not published by this workflow. Define and test an explicit
npm dist-tag policy before enabling them.
EOF
  exit 1
fi

if ! crate_version="$(python3 - "${manifest_path}" <<'PY'
import re
import sys
try:
    import tomllib as toml
except ModuleNotFoundError:
    import tomli as toml

manifest_path = sys.argv[1]
try:
    with open(manifest_path, "rb") as stream:
        manifest = toml.load(stream)
except (OSError, toml.TOMLDecodeError) as error:
    raise SystemExit(f"validate_native_release_version: failed to parse Cargo manifest: {error}")
package = manifest.get("package")
version = package.get("version") if isinstance(package, dict) else None
if not isinstance(version, str):
    raise SystemExit("validate_native_release_version: [package].version must be a string")
if re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version) is None:
    raise SystemExit("validate_native_release_version: [package].version must be stable MAJOR.MINOR.PATCH")
print(version)
PY
)"; then
  exit 1
fi

if [[ "${release_version}" != "${crate_version}" ]]; then
  cat >&2 <<EOF
validate_native_release_version: release tag and Cargo.toml version do not match
  tag: ${tag_name}
  tag version: ${release_version}
  Cargo.toml version: ${crate_version}

Update Cargo.toml before tagging, or create a new tag that matches the package version.
EOF
  exit 1
fi

if [[ -n "${release_commit}" ]]; then
  if ! release_oid="$(git -C "${repository_path}" rev-parse --verify "${release_commit}^{commit}" 2>/dev/null)"; then
    echo "validate_native_release_version: release commit is unavailable: ${release_commit}" >&2
    exit 1
  fi
  if ! tag_oid="$(git -C "${repository_path}" rev-parse --verify "refs/tags/${tag_name}^{commit}" 2>/dev/null)"; then
    echo "validate_native_release_version: release tag is unavailable in the fetched repository: ${tag_name}" >&2
    exit 1
  fi
  if [[ "${tag_oid}" != "${release_oid}" ]]; then
    echo "validate_native_release_version: release tag does not resolve to GITHUB_SHA" >&2
    exit 1
  fi
  if ! git -C "${repository_path}" rev-parse --verify "${allowed_ref}^{commit}" >/dev/null 2>&1; then
    echo "validate_native_release_version: allowed release ref is unavailable: ${allowed_ref}" >&2
    exit 1
  fi
  if ! git -C "${repository_path}" merge-base --is-ancestor "${release_oid}" "${allowed_ref}"; then
    echo "validate_native_release_version: release commit is not contained in ${allowed_ref}" >&2
    exit 1
  fi
fi

printf 'release_version=%s\n' "${release_version}"
printf 'crate_version=%s\n' "${crate_version}"
printf 'npm_dist_tag=latest\n'
