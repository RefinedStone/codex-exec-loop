#!/usr/bin/env bash
set -euo pipefail

unset GZIP POSIXLY_CORRECT TAR_OPTIONS TAR_READER_OPTIONS TAR_WRITER_OPTIONS
export LC_ALL=C
export TZ=UTC

usage() {
  cat <<'EOF'
Usage: scripts/verify_native_release.sh --version <version> --target <triple> [--profile <release|debug>] [--archive <path>] [--bundle-dir <path>]

Verify native release checksum artifacts produced by scripts/package_native_release.sh.

Examples:
  ./scripts/verify_native_release.sh --version 0.1.0 --target x86_64-unknown-linux-gnu --archive dist/native/codex-exec-loop-native-0.1.0-x86_64-unknown-linux-gnu.tar.gz
  ./scripts/verify_native_release.sh --version 0.1.0 --target x86_64-unknown-linux-gnu --bundle-dir dist/native/codex-exec-loop-native-0.1.0-x86_64-unknown-linux-gnu
  ./scripts/verify_native_release.sh \
    --version 0.1.0 \
    --target x86_64-unknown-linux-gnu \
    --archive dist/native/codex-exec-loop-native-0.1.0-x86_64-unknown-linux-gnu.tar.gz \
    --bundle-dir dist/native/codex-exec-loop-native-0.1.0-x86_64-unknown-linux-gnu
EOF
}

require_value() {
  local option="$1"
  local value="${2-}"
  if [[ -z "${value}" ]]; then
    echo "verify_native_release: missing value for ${option}" >&2
    usage >&2
    exit 1
  fi
}

archive_path=""
bundle_dir=""
version=""
target=""
profile="release"

while (($# > 0)); do
  case "$1" in
    --archive)
      require_value "$1" "${2-}"
      archive_path="$2"
      shift 2
      ;;
    --bundle-dir)
      require_value "$1" "${2-}"
      bundle_dir="$2"
      shift 2
      ;;
    --version)
      require_value "$1" "${2-}"
      version="$2"
      shift 2
      ;;
    --target)
      require_value "$1" "${2-}"
      target="$2"
      shift 2
      ;;
    --profile)
      require_value "$1" "${2-}"
      profile="$2"
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

if [[ -z "${version}" || -z "${target}" ]]; then
  echo "verify_native_release: --version and --target are required" >&2
  usage >&2
  exit 1
fi

normalized_version="${version#v}"
if [[ ! "${normalized_version}" =~ ^[0-9A-Za-z][0-9A-Za-z.+-]*$ ]]; then
  echo "verify_native_release: invalid version '${version}'" >&2
  exit 1
fi

case "${target}" in
  x86_64-unknown-linux-gnu|aarch64-apple-darwin|x86_64-pc-windows-msvc) ;;
  *)
    echo "verify_native_release: unsupported target '${target}'" >&2
    exit 1
    ;;
esac

case "${profile}" in
  release|debug) ;;
  *)
    echo "verify_native_release: unsupported profile '${profile}'" >&2
    exit 1
    ;;
esac

if ! command -v node >/dev/null 2>&1; then
  echo "verify_native_release: node is required for bounded release verification" >&2
  exit 1
fi
if ! node -e 'process.exit(Number(process.versions.node.split(".")[0]) >= 18 ? 0 : 1)'; then
  echo "verify_native_release: Node.js 18 or newer is required" >&2
  exit 1
fi
if [[ -n "${archive_path}" ]] && ! command -v tar >/dev/null 2>&1; then
  echo "verify_native_release: tar is required for archive policy verification" >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "${script_dir}/.." && pwd -P)"
node_verifier="${repo_root}/npm/scripts/verify-native-release-assets.mjs"
expected_root="codex-exec-loop-native-${normalized_version}-${target}"

forbidden_release_path() {
  local relative_path="$1"

  case "/${relative_path}" in
    */node_modules|*/node_modules/*|*/dist|*/dist/*|*/.vite|*/.vite/*|*/assets/admin|*/assets/admin/*)
      return 0
      ;;
  esac

  return 1
}

verify_no_forbidden_bundle_paths() {
  local root_dir="$1"
  local artifact_path
  local relative_path

  while IFS= read -r -d '' artifact_path; do
    relative_path="${artifact_path#${root_dir}/}"
    if forbidden_release_path "${relative_path}"; then
      echo "verify_native_release: forbidden build artifact found in bundle: ${relative_path}" >&2
      exit 1
    fi
  done < <(find "${root_dir}" -mindepth 1 -print0)
}

verify_no_forbidden_archive_paths() {
  local archive="$1"
  local relative_path

  while IFS= read -r relative_path; do
    if forbidden_release_path "${relative_path}"; then
      echo "verify_native_release: forbidden build artifact found in archive: ${relative_path}" >&2
      exit 1
    fi
  done < <(tar -tzf "${archive}")
}

if [[ -n "${archive_path}" ]]; then
  if [[ ! -f "${archive_path}" ]]; then
    echo "verify_native_release: archive not found: ${archive_path}" >&2
    exit 1
  fi
  archive_path="$(cd "$(dirname "${archive_path}")" && pwd -P)/$(basename "${archive_path}")"
  if [[ "$(basename "${archive_path}")" != "${expected_root}.tar.gz" ]]; then
    echo "verify_native_release: archive name does not match the expected version and target" >&2
    exit 1
  fi
  node "${node_verifier}" \
    --release-assets-dir "$(dirname "${archive_path}")" \
    --version "${normalized_version}" \
    --target "${target}" \
    --profile "${profile}"
  verify_no_forbidden_archive_paths "${archive_path}"
fi

if [[ -n "${bundle_dir}" ]]; then
  if [[ ! -d "${bundle_dir}" ]]; then
    echo "verify_native_release: bundle directory not found: ${bundle_dir}" >&2
    exit 1
  fi
  bundle_dir="$(cd "${bundle_dir}" && pwd -P)"
  if [[ "$(basename "${bundle_dir}")" != "${expected_root}" ]]; then
    echo "verify_native_release: bundle name does not match the expected version and target" >&2
    exit 1
  fi
  node "${node_verifier}" \
    --bundle-dir "${bundle_dir}" \
    --version "${normalized_version}" \
    --target "${target}" \
    --profile "${profile}"
  verify_no_forbidden_bundle_paths "${bundle_dir}"
fi
