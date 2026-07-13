#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

fail() {
  echo "macOS bundle build failed: $*" >&2
  exit 1
}

trim_env() {
  local name="$1"
  local value="${!name-}"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "${value}"
}

normalize_text_env() {
  local name="$1"
  local value
  value="$(trim_env "${name}")"
  if [[ -n "${value}" ]]; then
    printf -v "${name}" '%s' "${value}"
    export "${name}"
  else
    unset "${name}"
  fi
}

if [[ "$(uname -s)" != "Darwin" ]]; then
  fail "Tauri app/dmg bundling is supported only on macOS"
fi

if ! cargo tauri --version >/dev/null 2>&1; then
  fail "required cargo subcommand is missing: cargo tauri. Install with: cargo install tauri-cli --version 2.11.4 --locked"
fi

signing_mode="$("${ROOT}/scripts/check-macos-signing-env.sh")"

for name in \
  APPLE_SIGNING_IDENTITY APPLE_ID APPLE_TEAM_ID \
  APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH
do
  normalize_text_env "${name}"
done

for name in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_PASSWORD; do
  if [[ -z "$(trim_env "${name}")" ]]; then
    unset "${name}"
  else
    export "${name}"
  fi
done

if [[ "${signing_mode}" == "adhoc" ]]; then
  export APPLE_SIGNING_IDENTITY="-"
else
  api_key_path="$(trim_env APPLE_API_KEY_PATH)"
  if [[ -n "${api_key_path}" ]]; then
    api_key_dir="$(cd "$(dirname "${api_key_path}")" && pwd -P)"
    export APPLE_API_KEY_PATH="${api_key_dir}/$(basename "${api_key_path}")"
  fi
fi

cd "${ROOT}/crates/sigillum-desktop"
exec cargo tauri build "$@"
