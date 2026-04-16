#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/verify_native_release.sh [--archive <path>] [--bundle-dir <path>]

Verify native release checksum artifacts produced by scripts/package_native_release.sh.

Examples:
  ./scripts/verify_native_release.sh --archive dist/native/codex-exec-loop-native-0.1.0-x86_64-unknown-linux-gnu.tar.gz
  ./scripts/verify_native_release.sh --bundle-dir dist/native/codex-exec-loop-native-0.1.0-x86_64-unknown-linux-gnu
  ./scripts/verify_native_release.sh \
    --archive dist/native/codex-exec-loop-native-0.1.0-x86_64-unknown-linux-gnu.tar.gz \
    --bundle-dir dist/native/codex-exec-loop-native-0.1.0-x86_64-unknown-linux-gnu
EOF
}

checksum_tool=""
archive_path=""
bundle_dir=""

while (($# > 0)); do
  case "$1" in
    --archive)
      archive_path="${2-}"
      shift 2
      ;;
    --bundle-dir)
      bundle_dir="${2-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "verify_native_release: unsupported option '$1'" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "${archive_path}" && -z "${bundle_dir}" ]]; then
  echo "verify_native_release: at least one of --archive or --bundle-dir is required" >&2
  usage >&2
  exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
  checksum_tool="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
  checksum_tool="shasum"
elif command -v openssl >/dev/null 2>&1; then
  checksum_tool="openssl"
else
  echo "verify_native_release: one of sha256sum, shasum, or openssl is required" >&2
  exit 1
fi

compute_sha256() {
  local path="$1"
  local output

  case "${checksum_tool}" in
    sha256sum)
      output="$(sha256sum "${path}")"
      printf '%s\n' "${output%% *}"
      ;;
    shasum)
      output="$(shasum -a 256 "${path}")"
      printf '%s\n' "${output%% *}"
      ;;
    openssl)
      output="$(openssl dgst -sha256 "${path}")"
      printf '%s\n' "${output##*= }"
      ;;
  esac
}

verify_checksum_file() {
  local checksum_file="$1"
  local root_dir="$2"
  local verified_count=0

  if [[ ! -f "${checksum_file}" ]]; then
    echo "verify_native_release: checksum file not found: ${checksum_file}" >&2
    exit 1
  fi

  while IFS= read -r line || [[ -n "${line}" ]]; do
    [[ -z "${line}" ]] && continue

    local expected_digest="${line%% *}"
    local relative_path="${line#*  }"
    local artifact_path="${root_dir}/${relative_path}"

    if [[ ! -f "${artifact_path}" ]]; then
      echo "verify_native_release: referenced artifact not found: ${artifact_path}" >&2
      exit 1
    fi

    local actual_digest
    actual_digest="$(compute_sha256 "${artifact_path}")"
    if [[ "${actual_digest}" != "${expected_digest}" ]]; then
      echo "verify_native_release: checksum mismatch for ${artifact_path}" >&2
      echo "expected: ${expected_digest}" >&2
      echo "actual:   ${actual_digest}" >&2
      exit 1
    fi

    printf 'verified %s\n' "${artifact_path}"
    verified_count=$((verified_count + 1))
  done < "${checksum_file}"

  printf 'verified_count=%s\n' "${verified_count}"
}

if [[ -n "${archive_path}" ]]; then
  archive_path="$(cd "$(dirname "${archive_path}")" && pwd)/$(basename "${archive_path}")"
  archive_checksum_path="${archive_path}.sha256"
  verify_checksum_file "${archive_checksum_path}" "$(dirname "${archive_path}")"
fi

if [[ -n "${bundle_dir}" ]]; then
  bundle_dir="$(cd "${bundle_dir}" && pwd)"
  verify_checksum_file "${bundle_dir}/SHA256SUMS.txt" "${bundle_dir}"
fi
