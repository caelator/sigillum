#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

fail() {
  echo "macOS dmg notarization failed: $*" >&2
  exit 1
}

usage() {
  echo "usage: $0 <Sigillum.dmg>" >&2
  exit 2
}

require_command() {
  local command_name="$1"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    fail "required command is missing: ${command_name}"
  fi
}

trim_env() {
  local name="$1"
  local value="${!name-}"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "${value}"
}

case "$-" in
  *x*) fail "refusing to handle notarization credentials while shell xtrace is enabled" ;;
esac

if [[ "$#" != "1" ]]; then
  usage
fi

dmg_path="$1"

if [[ "$(uname -s)" != "Darwin" ]]; then
  fail "dmg notarization is supported only on macOS"
fi

require_command codesign
require_command jq
require_command xcrun

if [[ ! -f "${dmg_path}" || -L "${dmg_path}" || ! -r "${dmg_path}" ]]; then
  fail "dmg must be a readable, non-symlink regular file: ${dmg_path}"
fi

signing_mode="$("${ROOT}/scripts/check-macos-signing-env.sh")"
if [[ "${signing_mode}" != "developer-id" ]]; then
  fail "dmg notarization requires a complete Developer ID signing configuration"
fi

verify_output=""
if ! verify_output="$(codesign --verify --strict --verbose=4 "${dmg_path}" 2>&1)"; then
  echo "${verify_output}" >&2
  fail "dmg failed strict code-signature verification before notarization"
fi

metadata=""
if ! metadata="$(codesign -dv --verbose=4 "${dmg_path}" 2>&1)"; then
  echo "${metadata}" >&2
  fail "dmg signature metadata could not be read before notarization"
fi
if grep -Fqx 'Signature=adhoc' <<<"${metadata}" || \
   ! grep -Eq '^Authority=Developer ID Application: .+' <<<"${metadata}" || \
   ! grep -Eq '^TeamIdentifier=[A-Z0-9]+$' <<<"${metadata}"; then
  echo "${metadata}" >&2
  fail "dmg is not Developer ID signed"
fi

notary_args=(
  notarytool submit "${dmg_path}"
  --wait
  --output-format json
)

apple_id="$(trim_env APPLE_ID)"
if [[ -n "${apple_id}" ]]; then
  notary_args+=(
    --apple-id "${apple_id}"
    --password "${APPLE_PASSWORD}"
    --team-id "$(trim_env APPLE_TEAM_ID)"
  )
else
  api_key_path="$(trim_env APPLE_API_KEY_PATH)"
  api_key_dir="$(cd "$(dirname "${api_key_path}")" && pwd -P)"
  api_key_path="${api_key_dir}/$(basename "${api_key_path}")"
  if [[ ! -f "${api_key_path}" || -L "${api_key_path}" || \
        ! -r "${api_key_path}" || ! -s "${api_key_path}" ]]; then
    fail "APPLE_API_KEY_PATH must remain a readable, nonempty, non-symlink regular file"
  fi
  notary_args+=(
    --key-id "$(trim_env APPLE_API_KEY)"
    --key "${api_key_path}"
    --issuer "$(trim_env APPLE_API_ISSUER)"
  )
fi

notary_output=""
if ! notary_output="$(xcrun "${notary_args[@]}")"; then
  fail "notarytool submission failed"
fi
if ! jq -e '
  .status == "Accepted" and
  (.id | type == "string" and length > 0)
' <<<"${notary_output}" >/dev/null 2>&1; then
  echo "${notary_output}" >&2
  fail "notarytool did not return an accepted submission with a nonempty id"
fi

if ! xcrun stapler staple -v "${dmg_path}"; then
  fail "could not staple the accepted notarization ticket to the dmg"
fi
if ! xcrun stapler validate "${dmg_path}"; then
  fail "the stapled dmg notarization ticket did not validate"
fi

echo "macOS dmg notarization passed: ${dmg_path}"
