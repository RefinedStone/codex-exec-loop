#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/validate_native_release_version.sh [--tag <tag>] [--manifest <path>]

Validate that a release tag matches the Rust package version in Cargo.toml.

Examples:
  scripts/validate_native_release_version.sh --tag v1.3.4
  scripts/validate_native_release_version.sh --tag 1.3.4 --manifest Cargo.toml
EOF
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
manifest_path="${repo_root}/Cargo.toml"
tag_name="${GITHUB_REF_NAME:-}"

while (($# > 0)); do
  case "$1" in
    --tag)
      tag_name="${2-}"
      shift 2
      ;;
    --manifest)
      manifest_path="${2-}"
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

release_version="${tag_name#v}"
if [[ -z "${release_version}" || "${tag_name}" == "v" ]]; then
  echo "validate_native_release_version: invalid release tag: ${tag_name}" >&2
  exit 1
fi

crate_version="$(
  sed -n 's/^[[:space:]]*version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "${manifest_path}" | head -n 1
)"
if [[ -z "${crate_version}" ]]; then
  echo "validate_native_release_version: failed to read package version from ${manifest_path}" >&2
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

printf 'release_version=%s\n' "${release_version}"
printf 'crate_version=%s\n' "${crate_version}"
