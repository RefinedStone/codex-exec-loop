#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: .github/scripts/publish-release-assets.sh \
  --tag <tag> --repo <owner/repo> --expected-commit <full-commit-sha> \
  --title <title> --notes <notes> --asset-dir <directory>

Create a GitHub Release or add missing assets to an existing release. Existing assets are
downloaded and compared byte-for-byte; a different asset with the same name fails closed.
EOF
}

require_value() {
  local option="$1"
  local value="${2-}"
  if [[ -z "${value}" ]]; then
    echo "publish-release-assets: missing value for ${option}" >&2
    usage >&2
    exit 1
  fi
}

tag=""
repo=""
expected_commit=""
title=""
notes=""
asset_dir=""

while (($# > 0)); do
  case "$1" in
    --tag)
      require_value "$1" "${2-}"
      tag="$2"
      shift 2
      ;;
    --repo)
      require_value "$1" "${2-}"
      repo="$2"
      shift 2
      ;;
    --expected-commit)
      require_value "$1" "${2-}"
      expected_commit="$2"
      shift 2
      ;;
    --title)
      require_value "$1" "${2-}"
      title="$2"
      shift 2
      ;;
    --notes)
      require_value "$1" "${2-}"
      notes="$2"
      shift 2
      ;;
    --asset-dir)
      require_value "$1" "${2-}"
      asset_dir="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "publish-release-assets: unsupported option '$1'" >&2
      usage >&2
      exit 1
      ;;
  esac
done

for required in tag repo expected_commit title notes asset_dir; do
  if [[ -z "${!required}" ]]; then
    echo "publish-release-assets: --${required//_/-} is required" >&2
    exit 1
  fi
done
if [[ ! -d "${asset_dir}" ]]; then
  echo "publish-release-assets: asset directory not found: ${asset_dir}" >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
verify_tag_target() {
  bash "${script_dir}/verify-tag-target.sh" \
    --repo "${repo}" \
    --tag "${tag}" \
    --expected-commit "${expected_commit}" >/dev/null
}

verify_tag_target
if ! command -v gh >/dev/null 2>&1; then
  echo "publish-release-assets: gh is required" >&2
  exit 1
fi
if ! command -v cmp >/dev/null 2>&1; then
  echo "publish-release-assets: cmp is required" >&2
  exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "publish-release-assets: python3 is required" >&2
  exit 1
fi

mapfile -d '' -t files < <(find "${asset_dir}" -maxdepth 1 -type f -print0 | sort -z)
if ((${#files[@]} == 0)); then
  echo "publish-release-assets: no release assets found in ${asset_dir}" >&2
  exit 1
fi
for file in "${files[@]}"; do
  asset_name="$(basename "${file}")"
  if [[ ! "${asset_name}" =~ ^[A-Za-z0-9._-]+$ ]]; then
    echo "publish-release-assets: unsafe asset filename: ${asset_name}" >&2
    exit 1
  fi
done

temp_dir="$(mktemp -d)"
trap 'rm -rf "${temp_dir}"' EXIT
view_error="${temp_dir}/release-view.err"
expected_asset_names="${temp_dir}/expected-asset-names"
for file in "${files[@]}"; do
  basename "${file}"
done | LC_ALL=C sort > "${expected_asset_names}"

load_and_validate_release_snapshot() {
  local mode="$1"
  local snapshot="${temp_dir}/release-${mode}.json"
  if ! gh release view "${tag}" \
    --repo "${repo}" \
    --json tagName,name,isDraft,isPrerelease,assets > "${snapshot}"; then
    echo "publish-release-assets: failed to inspect existing release metadata" >&2
    return 1
  fi

  python3 - "${snapshot}" "${expected_asset_names}" "${tag}" "${title}" "${mode}" <<'PY'
import json
import pathlib
import sys

snapshot_path, expected_path, expected_tag, expected_title, mode = sys.argv[1:]
with open(snapshot_path, encoding="utf-8") as stream:
    snapshot = json.load(stream)
if not isinstance(snapshot, dict):
    raise SystemExit("publish-release-assets: release metadata must be an object")
if snapshot.get("tagName") != expected_tag:
    raise SystemExit("publish-release-assets: existing release tag metadata does not match")
if snapshot.get("name") != expected_title:
    raise SystemExit("publish-release-assets: existing release title metadata does not match")
if snapshot.get("isDraft") is not False or snapshot.get("isPrerelease") is not False:
    raise SystemExit("publish-release-assets: existing release must be published and stable")

assets = snapshot.get("assets")
if not isinstance(assets, list):
    raise SystemExit("publish-release-assets: existing release asset metadata is invalid")
names = []
for asset in assets:
    name = asset.get("name") if isinstance(asset, dict) else None
    if not isinstance(name, str) or not name:
        raise SystemExit("publish-release-assets: existing release contains an invalid asset name")
    names.append(name)
if len(names) != len(set(names)):
    raise SystemExit("publish-release-assets: existing release contains duplicate asset names")

expected = set(pathlib.Path(expected_path).read_text(encoding="utf-8").splitlines())
actual = set(names)
unexpected = sorted(actual - expected)
if unexpected:
    raise SystemExit(
        "publish-release-assets: existing release contains unexpected assets: "
        + ", ".join(unexpected)
    )
if mode == "final" and actual != expected:
    missing = sorted(expected - actual)
    raise SystemExit(
        "publish-release-assets: final release asset set is incomplete: "
        + ", ".join(missing)
    )
for name in sorted(actual):
    print(name)
PY
}

verify_final_release_asset_bytes() {
  local verification_dir
  local downloaded_names
  local file
  local asset_name
  local downloaded
  verification_dir="$(mktemp -d "${temp_dir}/final-assets.XXXXXX")"

  verify_tag_target
  if ! gh release download "${tag}" \
    --repo "${repo}" \
    --dir "${verification_dir}"; then
    echo "publish-release-assets: failed to download the final release asset set" >&2
    return 1
  fi
  downloaded_names="${verification_dir}/../final-downloaded-asset-names"
  find "${verification_dir}" -mindepth 1 -maxdepth 1 -printf '%f\n' \
    | LC_ALL=C sort > "${downloaded_names}"
  if ! cmp -s -- "${expected_asset_names}" "${downloaded_names}"; then
    echo "publish-release-assets: final downloaded asset set does not match the expected release" >&2
    return 1
  fi

  for file in "${files[@]}"; do
    asset_name="$(basename "${file}")"
    downloaded="${verification_dir}/${asset_name}"
    if [[ ! -f "${downloaded}" || -L "${downloaded}" ]]; then
      echo "publish-release-assets: final downloaded asset is missing or unsafe: ${asset_name}" >&2
      return 1
    fi
    if ! cmp -s -- "${file}" "${downloaded}"; then
      echo "publish-release-assets: final release asset differs from local build: ${asset_name}" >&2
      echo "Release assets changed during publication; publish a new version tag." >&2
      return 1
    fi
  done
  verify_tag_target
}

if ! gh release view "${tag}" --repo "${repo}" >/dev/null 2>"${view_error}"; then
  if grep -Eiq 'release not found|HTTP 404|Not Found' "${view_error}"; then
    verify_tag_target
    gh release create "${tag}" "${files[@]}" \
      --repo "${repo}" \
      --verify-tag \
      --title "${title}" \
      --notes "${notes}"
    verify_tag_target
    load_and_validate_release_snapshot final >/dev/null
    verify_final_release_asset_bytes
    gh release view "${tag}" --repo "${repo}" --json url --jq '.url'
    exit 0
  fi

  echo "publish-release-assets: failed to inspect existing release" >&2
  sed -n '1,20p' "${view_error}" >&2
  exit 1
fi

asset_list="${temp_dir}/asset-names"
load_and_validate_release_snapshot existing >"${asset_list}"

missing_files=()
index=0
for file in "${files[@]}"; do
  asset_name="$(basename "${file}")"
  if ! grep -Fxq -- "${asset_name}" "${asset_list}"; then
    missing_files+=("${file}")
    continue
  fi

  download_dir="${temp_dir}/download-${index}"
  index=$((index + 1))
  mkdir -p "${download_dir}"
  if ! gh release download "${tag}" \
    --repo "${repo}" \
    --pattern "${asset_name}" \
    --dir "${download_dir}"; then
    echo "publish-release-assets: failed to download existing asset ${asset_name}" >&2
    exit 1
  fi
  downloaded="${download_dir}/${asset_name}"
  if [[ ! -f "${downloaded}" ]]; then
    echo "publish-release-assets: downloaded asset is missing: ${asset_name}" >&2
    exit 1
  fi
  if ! cmp -s -- "${file}" "${downloaded}"; then
    echo "publish-release-assets: existing asset differs from local build: ${asset_name}" >&2
    echo "Refusing to overwrite immutable release output; publish a new version tag." >&2
    exit 1
  fi
  echo "Verified existing release asset: ${asset_name}"
done

if ((${#missing_files[@]} > 0)); then
  verify_tag_target
  gh release upload "${tag}" "${missing_files[@]}" --repo "${repo}"
  verify_tag_target
fi

verify_tag_target
load_and_validate_release_snapshot final >/dev/null
verify_final_release_asset_bytes

verify_tag_target
gh release view "${tag}" --repo "${repo}" --json url --jq '.url'
