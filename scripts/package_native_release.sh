#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/package_native_release.sh [--target <triple>] [--out-dir <path>] [--profile <release|debug>]

Build the native Rust client and stage a distributable bundle under dist/native/.

Examples:
  ./scripts/package_native_release.sh
  ./scripts/package_native_release.sh --target aarch64-apple-darwin
  ./scripts/package_native_release.sh --out-dir /tmp/native-dist --profile debug
EOF
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
native_dir="${repo_root}/native"
runbook_path="${native_dir}/docs/plan/13-native-packaging-and-operator-runbook.md"

profile="release"
target=""
out_dir="${repo_root}/dist/native"

while (($# > 0)); do
  case "$1" in
    --target)
      target="${2-}"
      shift 2
      ;;
    --out-dir)
      out_dir="${2-}"
      shift 2
      ;;
    --profile)
      profile="${2-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "package_native_release: unsupported option '$1'" >&2
      usage >&2
      exit 1
      ;;
  esac
done

case "${profile}" in
  release)
    cargo_profile_args=(--release)
    profile_dir="release"
    ;;
  debug)
    cargo_profile_args=()
    profile_dir="debug"
    ;;
  *)
    echo "package_native_release: profile must be 'release' or 'debug'" >&2
    exit 1
    ;;
esac

if [[ -f "${HOME}/.cargo/env" ]]; then
  # shellcheck disable=SC1090
  . "${HOME}/.cargo/env"
fi

for cmd in cargo rustc tar; do
  if ! command -v "${cmd}" >/dev/null 2>&1; then
    echo "package_native_release: ${cmd} is required" >&2
    exit 1
  fi
done

host_triple="$(rustc -vV | sed -n 's/^host: //p')"
artifact_target="${target:-${host_triple}}"
binary_name="codex-exec-loop-native"
binary_file_name="${binary_name}"
if [[ "${artifact_target}" == *windows* ]]; then
  binary_file_name="${binary_name}.exe"
fi

build_args=(cargo build --locked --manifest-path "${native_dir}/Cargo.toml")
build_args+=("${cargo_profile_args[@]}")
if [[ -n "${target}" ]]; then
  build_args+=(--target "${target}")
fi
"${build_args[@]}"

binary_path="${native_dir}/target"
if [[ -n "${target}" ]]; then
  binary_path="${binary_path}/${target}"
fi
binary_path="${binary_path}/${profile_dir}/${binary_file_name}"

if [[ ! -f "${binary_path}" ]]; then
  echo "package_native_release: built binary not found at ${binary_path}" >&2
  exit 1
fi

version="$(
  sed -n 's/^[[:space:]]*version[[:space:]]*=[[:space:]]*"\(.*\)"/\1/p' "${native_dir}/Cargo.toml" | head -n 1
)"
if [[ -z "${version}" ]]; then
  echo "package_native_release: failed to read native crate version" >&2
  exit 1
fi

package_name="${binary_name}-${version}-${artifact_target}"
mkdir -p "${out_dir}"
bundle_dir="${out_dir}/${package_name}"
archive_path="${out_dir}/${package_name}.tar.gz"

rm -rf "${bundle_dir}"
rm -f "${archive_path}"
mkdir -p "${bundle_dir}"

cp "${binary_path}" "${bundle_dir}/${binary_file_name}"
cp "${native_dir}/README.md" "${bundle_dir}/README.md"
cp "${runbook_path}" "${bundle_dir}/OPERATOR.md"

cat > "${bundle_dir}/VERSION.txt" <<EOF
name=${binary_name}
version=${version}
target=${artifact_target}
profile=${profile}
binary=${binary_file_name}
EOF

tar -C "${out_dir}" -czf "${archive_path}" "${package_name}"

printf 'bundle_dir=%s\n' "${bundle_dir}"
printf 'archive=%s\n' "${archive_path}"
