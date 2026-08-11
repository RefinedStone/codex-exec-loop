#!/usr/bin/env bash
set -euo pipefail

if (($# > 0)); then
  echo "validate_npm_release_auth: this command does not accept arguments" >&2
  exit 1
fi

if [[ -z "${NODE_AUTH_TOKEN:-}" ]]; then
  cat >&2 <<'EOF'
validate_npm_release_auth: NPM_TOKEN is required for an official tag release
Configure the repository NPM_TOKEN secret with a read/write granular access token limited to the
@refinedstone/akra package. Enable 2FA bypass only when the package publish policy requires it.
No native assets or GitHub Release should be published without the matching npm release.
EOF
  exit 1
fi

if ! command -v npm >/dev/null 2>&1; then
  echo "validate_npm_release_auth: npm is required" >&2
  exit 1
fi

for command_name in chmod mkdir mktemp rm tr; do
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "validate_npm_release_auth: ${command_name} is required" >&2
    exit 1
  fi
done

umask 077
runtime_root="$(mktemp -d "${TMPDIR:-/tmp}/akra-npm-auth.XXXXXX")"
cleanup() {
  rm -rf -- "${runtime_root}"
}
trap cleanup EXIT

chmod 0700 "${runtime_root}"
mkdir -p "${runtime_root}/cache"
chmod 0700 "${runtime_root}/cache"

write_controlled_npm_config() {
  local destination="$1"
  cat > "${destination}" <<'EOF'
registry=https://registry.npmjs.org/
@refinedstone:registry=https://registry.npmjs.org/
//registry.npmjs.org/:_authToken=${NODE_AUTH_TOKEN}
EOF
  chmod 0600 "${destination}"
}

write_controlled_npm_config "${runtime_root}/.npmrc"
write_controlled_npm_config "${runtime_root}/user.npmrc"
: > "${runtime_root}/global.npmrc"
chmod 0600 "${runtime_root}/global.npmrc"
cat > "${runtime_root}/package.json" <<'EOF'
{"name":"akra-release-auth-runtime","private":true}
EOF
chmod 0600 "${runtime_root}/package.json"

while IFS= read -r variable_name; do
  normalized_variable_name="$(printf '%s' "${variable_name}" | LC_ALL=C tr '[:lower:]' '[:upper:]')"
  if [[ "${normalized_variable_name}" == NPM_CONFIG_* ]]; then
    unset "${variable_name}"
  fi
done < <(compgen -e)

export NPM_CONFIG_CACHE="${runtime_root}/cache"
export NPM_CONFIG_GLOBALCONFIG="${runtime_root}/global.npmrc"
export NPM_CONFIG_REGISTRY="https://registry.npmjs.org"
export NPM_CONFIG_UPDATE_NOTIFIER="false"
export NPM_CONFIG_USERCONFIG="${runtime_root}/user.npmrc"

if ! (
  cd "${runtime_root}"
  npm whoami --registry https://registry.npmjs.org >/dev/null 2>&1
); then
  cat >&2 <<'EOF'
validate_npm_release_auth: npm authentication failed
Replace NPM_TOKEN with a read/write granular access token limited to @refinedstone/akra. Enable
2FA bypass only when the package publish policy requires it.
EOF
  exit 1
fi

echo "npm release authentication verified"
