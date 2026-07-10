#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
destination="${repo_root}/schema/codex_app_server_protocol.v2.schemas.json"
server_request_destination="${repo_root}/schema/codex_app_server_protocol.server_request.schema.json"

for command_name in codex node; do
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "refresh_codex_app_server_schema: ${command_name} is required" >&2
    exit 1
  fi
done

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/akra-app-server-schema.XXXXXX")"
cleanup() {
  rm -rf "${work_dir}"
}
trap cleanup EXIT

codex app-server generate-json-schema --experimental --out "${work_dir}"
generated="${work_dir}/codex_app_server_protocol.v2.schemas.json"
generated_server_request="${work_dir}/ServerRequest.json"
if [[ ! -s "${generated}" ]]; then
  echo "refresh_codex_app_server_schema: generator did not produce the v2 schema bundle" >&2
  exit 1
fi
if [[ ! -s "${generated_server_request}" ]]; then
  echo "refresh_codex_app_server_schema: generator did not produce ServerRequest.json" >&2
  exit 1
fi

source_cli_version="$(codex --version)"
generated_at="$(date -u +%Y-%m-%d)"
output="${work_dir}/codex_app_server_protocol.v2.pinned.json"
server_request_output="${work_dir}/codex_app_server_protocol.server_request.pinned.json"

node "${repo_root}/scripts/normalize_codex_app_server_schema.mjs" \
  --input="${generated}" \
  --output="${output}" \
  --source-cli="${source_cli_version}" \
  --generated-at="${generated_at}" \
  --schema-id="urn:codex-exec-loop-native:app-server-protocol:v2:snapshot" \
  --schema-version="v2" \
  --description="Checked-in Codex app-server protocol snapshot pinned by codex-exec-loop-native for startup diagnostics and adapter contract verification." \
  --source-artifact="codex_app_server_protocol.v2.schemas.json"

node "${repo_root}/scripts/normalize_codex_app_server_schema.mjs" \
  --input="${generated_server_request}" \
  --output="${server_request_output}" \
  --source-cli="${source_cli_version}" \
  --generated-at="${generated_at}" \
  --schema-id="urn:codex-exec-loop-native:app-server-protocol:server-request:snapshot" \
  --schema-version="server-request" \
  --description="Checked-in Codex app-server server-request snapshot used to keep client request handling and explicit decline behavior aligned with the installed protocol." \
  --source-artifact="ServerRequest.json"

mv "${output}" "${destination}"
mv "${server_request_output}" "${server_request_destination}"
printf 'refreshed %s and %s from %s\n' \
  "${destination}" \
  "${server_request_destination}" \
  "${source_cli_version}"
