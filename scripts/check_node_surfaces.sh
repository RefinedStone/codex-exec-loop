#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

for command_name in node npm; do
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "check_node_surfaces: ${command_name} is required" >&2
    exit 1
  fi
done

printf '\n==> repository Node script syntax\n'
for script_path in \
  scripts/capture_admin_graphic.mjs \
  scripts/agent-plan.mjs \
  scripts/ci-scope.mjs \
  scripts/normalize_codex_app_server_schema.mjs \
  npm/scripts/publish-package.mjs \
  npm/scripts/stage-npm-packages.mjs \
  npm/scripts/verify-native-release-assets.mjs \
  npm/bin/akra.js; do
  node --check "${script_path}"
done

printf '\n==> CI and agent planning policy tests\n'
node --test scripts/ci-scope.test.mjs scripts/agent-plan.test.mjs

printf '\n==> npm launcher tests\n'
npm --prefix npm test

printf '\n==> admin game dependencies\n'
npm --prefix assets/admin/game ci

printf '\n==> admin game dependency audit\n'
npm --prefix assets/admin/game audit --audit-level=high

printf '\n==> admin game typecheck\n'
npm --prefix assets/admin/game run check

printf '\n==> admin game reproducible build\n'
bundle_hash_before="$(git hash-object assets/admin/game/akra-diorama.js)"
npm --prefix assets/admin/game run build
bundle_hash_after="$(git hash-object assets/admin/game/akra-diorama.js)"
if [[ "${bundle_hash_before}" != "${bundle_hash_after}" ]]; then
  echo "check_node_surfaces: admin game bundle was stale before the check" >&2
  exit 1
fi

printf '\n==> admin offline asset policy\n'
if rg -n 'https?://|//(cdnjs|unpkg|esm\.sh|fastly\.jsdelivr)' \
  templates/admin \
  assets/admin/game/src \
  assets/admin/scripts; then
  echo "check_node_surfaces: admin source contains a remote runtime asset reference" >&2
  exit 1
fi
