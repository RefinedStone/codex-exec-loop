#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/package_native_release.sh [--target <triple>] [--out-dir <path>] [--profile <release|debug>]

Build the Rust client and stage a distributable bundle under dist/native/.
Set SOURCE_DATE_EPOCH to the release commit timestamp for reproducible archives.

Examples:
  ./scripts/package_native_release.sh
  ./scripts/package_native_release.sh --target aarch64-apple-darwin
  ./scripts/package_native_release.sh --out-dir /tmp/native-dist --profile debug
EOF
}

require_value() {
  local option="$1"
  local value="${2-}"
  if [[ -z "${value}" ]]; then
    echo "package_native_release: missing value for ${option}" >&2
    usage >&2
    exit 1
  fi
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "${script_dir}/.." && pwd -P)"
manifest_path="${repo_root}/Cargo.toml"
runbook_path="${repo_root}/docs/plan/13-native-packaging-and-operator-runbook.md"
runtime_scripts_dir="${repo_root}/scripts"
admin_font_license_path="${repo_root}/assets/admin/fonts/LICENSE-Galmuri.txt"

unset GZIP POSIXLY_CORRECT TAR_OPTIONS TAR_READER_OPTIONS TAR_WRITER_OPTIONS
export COPYFILE_DISABLE=1
export LC_ALL=C
export TZ=UTC

checksum_tool=""
profile="release"
target=""
out_dir="${repo_root}/dist/native"

while (($# > 0)); do
  case "$1" in
    --target)
      require_value "$1" "${2-}"
      target="$2"
      shift 2
      ;;
    --out-dir)
      require_value "$1" "${2-}"
      out_dir="$2"
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

for cmd in cargo date git gzip rustc tar touch; do
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

source_date_epoch="${SOURCE_DATE_EPOCH:-}"
if [[ -z "${source_date_epoch}" ]]; then
  source_date_epoch="$(git -C "${repo_root}" log -1 --format=%ct HEAD)"
fi
if [[ ! "${source_date_epoch}" =~ ^[0-9]+$ ]]; then
  echo "package_native_release: SOURCE_DATE_EPOCH must be a non-negative integer" >&2
  exit 1
fi

format_source_date() {
  local format="$1"
  local formatted

  if formatted="$(date -u -d "@${source_date_epoch}" "+${format}" 2>/dev/null)"; then
    printf '%s\n' "${formatted}"
    return 0
  fi
  if formatted="$(date -u -r "${source_date_epoch}" "+${format}" 2>/dev/null)"; then
    printf '%s\n' "${formatted}"
    return 0
  fi

  echo "package_native_release: SOURCE_DATE_EPOCH is outside the supported date range" >&2
  exit 1
}

source_date_tar="$(format_source_date '%Y-%m-%d %H:%M:%S UTC')"
source_date_touch="$(format_source_date '%Y%m%d%H%M.%S')"
export SOURCE_DATE_EPOCH="${source_date_epoch}"

host_triple="$(rustc -vV | sed -n 's/^host: //p')"
artifact_target="${target:-${host_triple}}"
binary_name="codex-exec-loop-native"
binary_file_name="${binary_name}"
launcher_file_name="akra"
if [[ "${artifact_target}" == *windows* ]]; then
  binary_file_name="${binary_name}.exe"
  launcher_file_name="akra.cmd"
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
archive_member_list="${out_dir}/.${package_name}.members"
archive_member_list_sorted="${out_dir}/.${package_name}.members.sorted"
archive_mtree_manifest="${out_dir}/.${package_name}.mtree"
uncompressed_archive_path="${out_dir}/.${package_name}.tar"

rm -rf "${bundle_dir}"
rm -f "${archive_path}"
rm -f "${archive_checksum_path}"
rm -f "${archive_member_list}"
rm -f "${archive_member_list_sorted}"
rm -f "${archive_mtree_manifest}"
rm -f "${uncompressed_archive_path}"
mkdir -p "${bundle_dir}"

safe_bundle_path() {
  local relative_path="$1"
  [[ "${relative_path}" =~ ^[A-Za-z0-9._/@+-]+$ ]] || return 1
  case "${relative_path}" in
    /*|*//*|../*|*/../*|*/..|./*|*/./*|*/.) return 1 ;;
  esac
  return 0
}

require_regular_repo_file() {
  local source_path="$1"
  local display_name="$2"
  local physical_parent

  if [[ ! -f "${source_path}" || -L "${source_path}" ]]; then
    echo "package_native_release: input must be a non-symlink regular file: ${display_name}" >&2
    exit 1
  fi
  if ! physical_parent="$(cd "$(dirname "${source_path}")" && pwd -P)"; then
    echo "package_native_release: failed to resolve input path: ${display_name}" >&2
    exit 1
  fi
  case "${physical_parent}/$(basename "${source_path}")" in
    "${repo_root}/"*) ;;
    *)
      echo "package_native_release: input resolves outside the repository: ${display_name}" >&2
      exit 1
      ;;
  esac
}

copy_tracked_paths() {
  local relative_path
  while IFS= read -r -d '' relative_path; do
    [[ -z "${relative_path}" ]] && continue

    local source_path="${repo_root}/${relative_path}"
    local destination_path="${bundle_dir}/${relative_path}"
    if ! safe_bundle_path "${relative_path}"; then
      printf 'package_native_release: unsupported bundle path: %q\n' "${relative_path}" >&2
      exit 1
    fi
    require_regular_repo_file "${source_path}" "${relative_path}"
    mkdir -p "$(dirname "${destination_path}")"
    cp "${source_path}" "${destination_path}"
  done < <(git -C "${repo_root}" ls-files -z -- "$@")
}

require_regular_repo_file "${binary_path}" "${binary_path#${repo_root}/}"
require_regular_repo_file "${repo_root}/README.md" "README.md"
require_regular_repo_file "${runbook_path}" "${runbook_path#${repo_root}/}"
require_regular_repo_file "${runtime_scripts_dir}/gh-akra.sh" "scripts/gh-akra.sh"
require_regular_repo_file "${admin_font_license_path}" "assets/admin/fonts/LICENSE-Galmuri.txt"
cp "${binary_path}" "${bundle_dir}/${binary_file_name}"
cp "${repo_root}/README.md" "${bundle_dir}/README.md"
cp "${runbook_path}" "${bundle_dir}/OPERATOR.md"
mkdir -p "${bundle_dir}/THIRD_PARTY_NOTICES"
cp "${admin_font_license_path}" "${bundle_dir}/THIRD_PARTY_NOTICES/Galmuri-OFL.txt"
copy_tracked_paths assets/app-server/skills
mkdir -p "${bundle_dir}/scripts"
cp "${runtime_scripts_dir}/gh-akra.sh" "${bundle_dir}/scripts/gh-akra.sh"
chmod +x "${bundle_dir}/scripts/gh-akra.sh"

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
release_tag=v${version}
target=${artifact_target}
profile=${profile}
binary=${binary_file_name}
launcher=${launcher_file_name}
EOF

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

normalize_bundle_metadata() {
  local artifact_path

  while IFS= read -r -d '' artifact_path; do
    chmod 0755 "${artifact_path}"
  done < <(find "${bundle_dir}" -type d -print0)

  while IFS= read -r -d '' artifact_path; do
    chmod 0644 "${artifact_path}"
  done < <(find "${bundle_dir}" -type f -print0)

  chmod 0755 "${bundle_dir}/${binary_file_name}"
  chmod 0755 "${bundle_dir}/scripts/gh-akra.sh"
  if [[ "${artifact_target}" != *windows* ]]; then
    chmod 0755 "${bundle_dir}/akra"
  fi

  while IFS= read -r -d '' artifact_path; do
    touch -t "${source_date_touch}" "${artifact_path}"
  done < <(find "${bundle_dir}" -print0)
}

write_archive_member_list() {
  local member

  : > "${archive_member_list}"
  while IFS= read -r -d '' member; do
    if ! safe_bundle_path "${member}"; then
      printf 'package_native_release: unsupported archive member name: %q\n' "${member}" >&2
      exit 1
    fi
    printf '%s\n' "${member}" >> "${archive_member_list}"
  done < <(
    cd "${out_dir}"
    find "${package_name}" -print0
  )
  LC_ALL=C sort "${archive_member_list}" > "${archive_member_list_sorted}"
  mv "${archive_member_list_sorted}" "${archive_member_list}"
}

write_bsdtar_manifest() {
  local member
  local member_path
  local mode

  printf '#mtree v2.0\n' > "${archive_mtree_manifest}"
  while IFS= read -r member; do
    if ! safe_bundle_path "${member}"; then
      printf 'package_native_release: unsupported archive member name: %q\n' "${member}" >&2
      exit 1
    fi

    member_path="${out_dir}/${member}"
    if [[ -d "${member_path}" ]]; then
      printf '%s type=dir uid=0 gid=0 mode=0755 time=%s\n' \
        "${member}" "${source_date_epoch}" >> "${archive_mtree_manifest}"
      continue
    fi
    if [[ ! -f "${member_path}" ]]; then
      echo "package_native_release: unsupported archive member type: ${member}" >&2
      exit 1
    fi

    mode="0644"
    case "${member}" in
      "${package_name}/${binary_file_name}"|"${package_name}/scripts/gh-akra.sh"|"${package_name}/akra")
        if [[ "${member}" != "${package_name}/akra" || "${artifact_target}" != *windows* ]]; then
          mode="0755"
        fi
        ;;
    esac
    printf '%s type=file uid=0 gid=0 mode=%s time=%s content=%s\n' \
      "${member}" "${mode}" "${source_date_epoch}" "${member}" \
      >> "${archive_mtree_manifest}"
  done < "${archive_member_list}"
}

create_reproducible_archive() {
  local tar_version
  tar_version="$(tar --version 2>&1 || true)"

  if [[ "${tar_version}" == *"GNU tar"* ]]; then
    tar \
      --create \
      --file=- \
      --format=ustar \
      --sort=name \
      --mtime="${source_date_tar}" \
      --owner=0 \
      --group=0 \
      --numeric-owner \
      --no-recursion \
      --verbatim-files-from \
      --directory="${out_dir}" \
      --files-from="${archive_member_list}" > "${uncompressed_archive_path}"
  elif [[ "${tar_version}" == *bsdtar* || "${tar_version}" == *libarchive* ]]; then
    write_bsdtar_manifest
    tar \
      --create \
      --file=- \
      --format=ustar \
      --uid 0 \
      --gid 0 \
      --numeric-owner \
      --no-acls \
      --no-fflags \
      --no-mac-metadata \
      --no-xattrs \
      --no-recursion \
      --directory "${out_dir}" \
      "@${archive_mtree_manifest}" > "${uncompressed_archive_path}"
  else
    echo "package_native_release: GNU tar or bsdtar/libarchive is required" >&2
    exit 1
  fi

  gzip -n < "${uncompressed_archive_path}" > "${archive_path}"
  rm -f "${uncompressed_archive_path}"
  rm -f "${archive_member_list}"
  rm -f "${archive_member_list_sorted}"
  rm -f "${archive_mtree_manifest}"
}

write_bundle_launcher
copy_tracked_paths examples .codex-exec-loop/followups
write_bundle_checksums > "${bundle_checksum_path}"
normalize_bundle_metadata
write_archive_member_list
create_reproducible_archive

write_checksum_entry "${archive_path}" "$(basename "${archive_path}")" > "${archive_checksum_path}"

printf 'bundle_dir=%s\n' "${bundle_dir}"
printf 'archive=%s\n' "${archive_path}"
printf 'archive_checksum=%s\n' "${archive_checksum_path}"
