#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/package_native_release.sh [--target <triple>] [--out-dir <path>] [--profile <release|debug>]

Build the Rust client and stage a distributable bundle under dist/native/.

Examples:
  ./scripts/package_native_release.sh
  ./scripts/package_native_release.sh --target aarch64-apple-darwin
  ./scripts/package_native_release.sh --out-dir /tmp/native-dist --profile debug
EOF
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
manifest_path="${repo_root}/Cargo.toml"
runbook_path="${repo_root}/docs/plan/13-native-packaging-and-operator-runbook.md"
assets_dir="${repo_root}/assets"

checksum_tool=""
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

for cmd in cargo git rustc tar; do
  if ! command -v "${cmd}" >/dev/null 2>&1; then
    echo "package_native_release: ${cmd} is required" >&2
    exit 1
  fi
done

if command -v sha256sum >/dev/null 2>&1; then
  checksum_tool="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
  checksum_tool="shasum"
elif command -v openssl >/dev/null 2>&1; then
  checksum_tool="openssl"
else
  echo "package_native_release: one of sha256sum, shasum, or openssl is required" >&2
  exit 1
fi

host_triple="$(rustc -vV | sed -n 's/^host: //p')"
artifact_target="${target:-${host_triple}}"
binary_name="codex-exec-loop-native"
binary_file_name="${binary_name}"
if [[ "${artifact_target}" == *windows* ]]; then
  binary_file_name="${binary_name}.exe"
fi

build_args=(cargo build --locked --manifest-path "${manifest_path}")
build_args+=("${cargo_profile_args[@]}")
if [[ -n "${target}" ]]; then
  build_args+=(--target "${target}")
fi
"${build_args[@]}"

binary_path="${repo_root}/target"
if [[ -n "${target}" ]]; then
  binary_path="${binary_path}/${target}"
fi
binary_path="${binary_path}/${profile_dir}/${binary_file_name}"

if [[ ! -f "${binary_path}" ]]; then
  echo "package_native_release: built binary not found at ${binary_path}" >&2
  exit 1
fi

version="$(
  sed -n 's/^[[:space:]]*version[[:space:]]*=[[:space:]]*"\(.*\)"/\1/p' "${manifest_path}" | head -n 1
)"
if [[ -z "${version}" ]]; then
  echo "package_native_release: failed to read crate version" >&2
  exit 1
fi

package_name="${binary_name}-${version}-${artifact_target}"
mkdir -p "${out_dir}"
bundle_dir="${out_dir}/${package_name}"
archive_path="${out_dir}/${package_name}.tar.gz"
archive_checksum_path="${archive_path}.sha256"
bundle_checksum_path="${bundle_dir}/SHA256SUMS.txt"

rm -rf "${bundle_dir}"
rm -f "${archive_path}"
rm -f "${archive_checksum_path}"
mkdir -p "${bundle_dir}"

cp "${binary_path}" "${bundle_dir}/${binary_file_name}"
cp "${repo_root}/README.md" "${bundle_dir}/README.md"
cp "${runbook_path}" "${bundle_dir}/OPERATOR.md"
cp -R "${assets_dir}" "${bundle_dir}/assets"

write_bundle_launcher() {
  if [[ "${artifact_target}" == *windows* ]]; then
    cat > "${bundle_dir}/akra.cmd" <<EOF
@echo off
setlocal
set "SCRIPT_DIR=%~dp0"
"%SCRIPT_DIR%${binary_file_name}" %*
EOF
    return 0
  fi

  cat > "${bundle_dir}/akra" <<EOF
#!/usr/bin/env bash
set -euo pipefail

script_dir="\$(cd "\$(dirname "\${BASH_SOURCE[0]}")" && pwd)"
exec "\${script_dir}/${binary_file_name}" "\$@"
EOF
  chmod +x "${bundle_dir}/akra"
}

cat > "${bundle_dir}/VERSION.txt" <<EOF
name=${binary_name}
version=${version}
target=${artifact_target}
profile=${profile}
binary=${binary_file_name}
launcher=akra
EOF

copy_tracked_bundle_samples() {
  while IFS= read -r relative_path; do
    [[ -z "${relative_path}" ]] && continue

    local source_path="${repo_root}/${relative_path}"
    local destination_path="${bundle_dir}/${relative_path}"
    mkdir -p "$(dirname "${destination_path}")"
    cp "${source_path}" "${destination_path}"
  done < <(git -C "${repo_root}" ls-files examples .codex-exec-loop/followups)
}

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

write_checksum_entry() {
  local path="$1"
  local display_name="$2"
  printf '%s  %s\n' "$(compute_sha256 "${path}")" "${display_name}"
}

write_bundle_checksums() {
  while IFS= read -r artifact_path; do
    local relative_path="${artifact_path#${bundle_dir}/}"
    write_checksum_entry "${artifact_path}" "${relative_path}"
  done < <(find "${bundle_dir}" -type f ! -name 'SHA256SUMS.txt' | sort)
}

write_bundle_launcher
copy_tracked_bundle_samples
write_bundle_checksums > "${bundle_checksum_path}"

tar -C "${out_dir}" -czf "${archive_path}" "${package_name}"
write_checksum_entry "${archive_path}" "$(basename "${archive_path}")" > "${archive_checksum_path}"

printf 'bundle_dir=%s\n' "${bundle_dir}"
printf 'archive=%s\n' "${archive_path}"
printf 'archive_checksum=%s\n' "${archive_checksum_path}"
