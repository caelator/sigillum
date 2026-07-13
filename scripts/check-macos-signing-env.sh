#!/usr/bin/env bash
set -euo pipefail

fail() {
  echo "macOS signing environment check failed: $*" >&2
  exit 1
}

trim_env() {
  local name="$1"
  local value="${!name-}"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "${value}"
}

count_nonblank() {
  local count=0
  local name
  for name in "$@"; do
    if [[ -n "$(trim_env "${name}")" ]]; then
      count=$((count + 1))
    fi
  done
  printf '%s\n' "${count}"
}

certificate_count="$(count_nonblank \
  APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY)"
identity="$(trim_env APPLE_SIGNING_IDENTITY)"

if [[ "${certificate_count}" == "0" ]]; then
  signing_mode="adhoc"
elif [[ "${certificate_count}" == "1" && "${identity}" == "-" ]]; then
  signing_mode="adhoc"
elif [[ "${certificate_count}" == "3" && "${identity}" != "-" ]]; then
  signing_mode="developer-id"
else
  fail "set all of APPLE_CERTIFICATE, APPLE_CERTIFICATE_PASSWORD, and a non-'-' APPLE_SIGNING_IDENTITY, or set none of them (APPLE_SIGNING_IDENTITY=- is the only explicit ad-hoc form)"
fi

apple_id_count="$(count_nonblank APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID)"
api_key_count="$(count_nonblank APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH)"

if [[ "${apple_id_count}" != "0" && "${apple_id_count}" != "3" ]]; then
  fail "APPLE_ID, APPLE_PASSWORD, and APPLE_TEAM_ID must be set together"
fi

if [[ "${api_key_count}" != "0" && "${api_key_count}" != "3" ]]; then
  fail "APPLE_API_KEY, APPLE_API_ISSUER, and APPLE_API_KEY_PATH must be set together"
fi

if [[ "${apple_id_count}" == "3" && "${api_key_count}" == "3" ]]; then
  fail "configure exactly one notarization credential family, not both Apple ID and API key credentials"
fi

if [[ "${signing_mode}" == "adhoc" && ( "${apple_id_count}" == "3" || "${api_key_count}" == "3" ) ]]; then
  fail "notarization credentials require a complete Developer ID signing configuration"
fi

if [[ "${signing_mode}" == "developer-id" && "${apple_id_count}" == "0" && "${api_key_count}" == "0" ]]; then
  fail "Developer ID signing requires exactly one complete notarization credential family"
fi

if [[ "${api_key_count}" == "3" ]]; then
  api_key_path="$(trim_env APPLE_API_KEY_PATH)"
  if [[ ! -f "${api_key_path}" || -L "${api_key_path}" || ! -r "${api_key_path}" || ! -s "${api_key_path}" ]]; then
    fail "APPLE_API_KEY_PATH must name a readable, nonempty, non-symlink regular file"
  fi
fi

printf '%s\n' "${signing_mode}"
